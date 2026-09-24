//! Source-Engine / GoldSrc server-query protocol.
//!
//! ## Request bytes (GoldSrc, verified against ReHLDS `engine/net.h`)
//!
//! GoldSrc does **not** use the bare `0x69` byte that later Source games use
//! for `A2S_INFO`. The engine declares:
//!
//! ```c
//! const char A2A_PING         = 'i';  // 0x69 — legacy echo, NOT info
//! const char A2S_INFO         = 'T';  // 0x54 — request server info
//! const char A2S_PLAYER       = 'U';  // 0x55
//! const char A2S_RULES        = 'V';  // 0x56
//! const char A2A_GETCHALLENGE = 'W';  // 0x57
//! const char S2A_INFO         = 'C';  // 0x43 — deprecated reply
//! const char S2A_INFO_DETAILED= 'm';  // 0x6D — "new query protocol" reply
//! const char S2A_PLAYER       = 'D';  // 0x44
//! const char S2A_RULES        = 'E';  // 0x45
//! const char S2A_CHALLENGE    = 'A';  // 0x41
//! ```
//!
//! Every request is prefixed with the `0xFFFFFFFF` "connectionless packet"
//! marker, which the engine consumes via `MSG_ReadLong()` before it
//! tokenises the command (`SV_ConnectionlessPacket`). Sending a bare
//! `0x69` therefore lands on `A2A_PING` and never produces server info.
//!
//! Empirically, the form that GoldSrc servers actually answer is the
//! Source-style info request:
//!
//! ```text
//! client -> server : FF FF FF FF "TSource Engine Query" 00
//! server -> client : FF FF FF FF 41 <challenge:i32le>        (if challenged)
//! client -> server : FF FF FF FF 54 <challenge:i32le>
//! server -> client : FF FF FF FF 6D <info_data>              (0x6D, not 0x6E!)
//! ```
//!
//! The reply header for GoldSrc is `0x6D` (`S2A_INFO_DETAILED`). Older
//! servers may answer the deprecated `0x43` (`S2A_INFO`) form, and a few
//! proxy implementations still hand back the Source-style `0x6E`. The parser
//! accepts all three.
//!
//! ## A2S_PLAYER
//!
//! ```text
//! client -> server : FF FF FF FF 55 <challenge:i32le>
//! server -> client : FF FF FF FF 44 <count:u8> <Player[]>
//! Player := <index:u8> <name:null-terminated CString> <score:i32le> <duration:f32le>
//! ```
//!
//! ## A2S_RULES
//!
//! ```text
//! client -> server : FF FF FF FF 56 <challenge:i32le>
//! server -> client : FF FF FF FF 45 <count:u16le> <(name,value) CString pairs>
//! ```
//!
//! ## info_data format (GoldSrc 0x6D / Source 0x6E)
//!
//! ```text
//! address : null-terminated string "<ip>:<port>"
//! hostname: null-terminated string
//! map     : null-terminated string
//! gamedir : null-terminated string
//! gamedesc: null-terminated string
//! players : 1 byte
//! max     : 1 byte
//! protocol: 1 byte      (GoldSrc: PROTOCOL_VERSION)
//! server_type : 1 byte  ('d' dedicated, 'l' listen, 'p' SourceTV)
//! os      : 1 byte      ('w', 'l', 'm')
//! password: 1 byte      (0 or 1)
//! vac     : 1 byte      (0 or 1)
//! bots    : 1 byte      (GoldSrc 0x6D only — Source puts this in ExtraDataFlag)
//! version : null-terminated string
//! ```
//!
//! ## Split packet handling
//!
//! If the response is too big for a single UDP datagram the server
//! sends `0xFE 0xFF 0xFF 0xFF <sequence:i32le> <chunk>` packets and the
//! client re-assembles them by sequence. Real-world CS 1.6 servers always
//! fit in one packet, but the parser tolerates the split form.

use crate::model::{Endpoint, Os, ServerType, Vac};
use anyhow::{bail, Result};
use std::net::SocketAddr;
use std::time::Instant;
use tokio::net::UdpSocket;
use tokio::time::{timeout, Duration};

/// Single-packet response cap.
pub const UDP_RECV_BUF: usize = 16 * 1024;

/// Connectionless packet marker that prefixes every GoldSrc request/reply.
pub const CONNECTIONLESS: [u8; 4] = [0xFF, 0xFF, 0xFF, 0xFF];

/// The Source-style info request string. GoldSrc accepts this spelling even
/// though it is not the native `0x54`-only form.
pub const INFO_REQUEST_STRING: &[u8] = b"TSource Engine Query\0";

/// Header byte for a GoldSrc detailed info reply (`S2A_INFO_DETAILED`).
pub const INFO_HEADER_GOLDSRC: u8 = 0x6D;
/// Header byte for the deprecated GoldSrc info reply (`S2A_INFO`).
pub const INFO_HEADER_DEPRECATED: u8 = 0x43;
/// Header byte for a Source-style info reply (`S2A_INFO_SOURCE`). Modern
/// GoldSrc builds that speak the Source query protocol answer with this.
pub const INFO_HEADER_SOURCE: u8 = 0x49;
/// Header byte for a Source-style info reply (`S2A_INFO` 0x6E).
pub const INFO_HEADER: u8 = 0x6E;
/// Header byte for a challenge reply.
pub const CHALLENGE_HEADER: u8 = 0x41;
/// Header byte for a player list reply.
pub const PLAYER_HEADER: u8 = 0x44;
/// Header byte for a rules reply.
pub const RULES_HEADER: u8 = 0x45;

/// Header byte for a split-packet sequence.
pub const PACKET_SPLIT: u8 = 0xFE;

/// A2S request opcodes, as declared by the GoldSrc engine
/// (`rehlds/engine/net.h`).
pub const REQ_INFO: u8 = b'T'; // 0x54 — A2S_INFO
pub const REQ_PLAYER: u8 = b'U'; // 0x55 — A2S_PLAYER
pub const REQ_RULES: u8 = b'V'; // 0x56 — A2S_RULES
pub const REQ_CHALLENGE: u8 = b'W'; // 0x57 — A2A_GETCHALLENGE
/// Legacy `A2A_PING`. Responds with `0x6A`, never with server info.
pub const REQ_PING: u8 = b'i'; // 0x69
pub const REQ_ACK: u8 = b'j'; // 0x6A — A2A_ACK

/// Decoded A2S_INFO data.
#[derive(Debug, Clone)]
pub struct ServerInfoReply {
    pub endpoint: Endpoint,
    pub hostname: String,
    pub map: String,
    pub gamedir: String,
    pub gamedesc: String,
    pub players: u8,
    pub max_players: u8,
    pub bots: u8,
    pub server_type: ServerType,
    pub os: Os,
    pub password: bool,
    pub vac: Vac,
    pub version: String,
}

/// Decoded A2S_PLAYER data.
#[derive(Debug, Clone)]
pub struct ServerPlayersReply {
    pub players: Vec<crate::model::Player>,
}

/// Cursor over a sequence of null-terminated C-strings.
pub struct CStrIter<'a> {
    pub buf: &'a [u8],
    pub pos: usize,
}

impl<'a> CStrIter<'a> {
    pub fn new(buf: &'a [u8]) -> Self {
        Self { buf, pos: 0 }
    }

    /// Read the next null-terminated string.
    ///
    /// Named `read_cstr` rather than `next` so it is not confused with
    /// `Iterator::next`.
    ///
    /// Decoded **lossily**: GoldSrc strings are raw bytes in whatever codepage
    /// the admin's console used (cp1251 hostnames are common), not UTF-8. A
    /// strict decode rejected the whole reply, so a live server vanished from
    /// the list over a single byte in its name.
    pub fn read_cstr(&mut self) -> Result<String> {
        if self.pos >= self.buf.len() {
            bail!("cstr: out of range");
        }
        let start = self.pos;
        while self.pos < self.buf.len() && self.buf[self.pos] != 0 {
            self.pos += 1;
        }
        if self.pos >= self.buf.len() {
            bail!("cstr: missing null terminator");
        }
        let s = String::from_utf8_lossy(&self.buf[start..self.pos]).into_owned();
        self.pos += 1;
        Ok(s)
    }

    pub fn read_u8(&mut self) -> Result<u8> {
        if self.pos >= self.buf.len() {
            bail!("read_u8: out of range");
        }
        let v = self.buf[self.pos];
        self.pos += 1;
        Ok(v)
    }

    pub fn read_i32(&mut self) -> Result<i32> {
        if self.pos + 4 > self.buf.len() {
            bail!("read_i32: out of range");
        }
        let v = i32::from_le_bytes(self.buf[self.pos..self.pos + 4].try_into().unwrap());
        self.pos += 4;
        Ok(v)
    }

    pub fn read_f32(&mut self) -> Result<f32> {
        if self.pos + 4 > self.buf.len() {
            bail!("read_f32: out of range");
        }
        let v = f32::from_le_bytes(self.buf[self.pos..self.pos + 4].try_into().unwrap());
        self.pos += 4;
        Ok(v)
    }

    pub fn remaining(&self) -> &[u8] {
        &self.buf[self.pos..]
    }
}

/// Parse an A2S_INFO payload.
///
/// Two reply shapes appear in the wild and they are **not** interchangeable —
/// the field order differs, so a parser that assumes one reads garbage from
/// the other.
///
/// ### `0x49` — `S2A_INFO_SOURCE` (the modern query protocol)
///
/// This is what current Steam/ReHLDS CS 1.6 servers answer with. Verified
/// byte-for-byte against a live server:
///
/// ```text
/// protocol:u8  name:str  map:str  folder:str  game:str
/// appid:u16le  players:u8  maxplayers:u8  bots:u8
/// servertype:u8  environment:u8  visibility:u8  vac:u8
/// version:str  [ExtraDataFlag:u8 ...]
/// ```
///
/// ### `0x6D` — `S2A_INFO_DETAILED` (native GoldSrc)
///
/// The older shape, which begins with the address and has no appid or game
/// version (Valve's "Obsolete GoldSource Response"):
///
/// ```text
/// address:str  name:str  map:str  folder:str  game:str
/// players:u8  maxplayers:u8  protocol:u8
/// servertype:u8  environment:u8  visibility:u8  mod:u8
/// [mod == 1: link:str  downloadlink:str  null:u8  version:i32le
///            size:i32le  type:u8  dll:u8]
/// vac:u8  bots:u8
/// ```
///
/// The `mod` byte and its optional block sit **before** VAC. Reading VAC
/// straight after `visibility` picked up the mod flag instead, so every
/// server answering in this shape was reported as unsecured.
///
/// `0x43` (`S2A_INFO`, deprecated GoldSrc) is treated as the same layout
/// without the bots byte. A leading `0xFFFFFFFF` connectionless marker is
/// tolerated.
pub fn parse_info(endpoint: Endpoint, mut buf: &[u8]) -> Result<ServerInfoReply> {
    if buf.is_empty() {
        bail!("empty info packet");
    }
    if buf.len() >= 4 && buf[..4] == CONNECTIONLESS {
        buf = &buf[4..];
    }
    if buf.is_empty() {
        bail!("empty info packet after marker");
    }
    if buf[0] == PACKET_SPLIT {
        bail!("split-packet info not reassembled");
    }

    let header = buf[0];
    buf = &buf[1..];

    match header {
        // Modern Source query protocol. `0x6E` is accepted as an alias: some
        // proxies emit it for the same layout.
        INFO_HEADER_SOURCE | INFO_HEADER => parse_info_source(endpoint, buf),
        // Native GoldSrc.
        INFO_HEADER_GOLDSRC => parse_info_goldsrc(endpoint, buf, true),
        INFO_HEADER_DEPRECATED => parse_info_goldsrc(endpoint, buf, false),
        other => bail!("bad info header: {other:#x}"),
    }
}

/// `0x49` / `0x6E` — the Source query protocol layout.
fn parse_info_source(endpoint: Endpoint, buf: &[u8]) -> Result<ServerInfoReply> {
    let mut it = CStrIter::new(buf);
    let _protocol = it.read_u8()?;
    let hostname = it.read_cstr()?;
    let map = it.read_cstr()?;
    let gamedir = it.read_cstr()?;
    let gamedesc = it.read_cstr()?;
    let _app_id = {
        let lo = it.read_u8()? as u16;
        let hi = it.read_u8()? as u16;
        lo | (hi << 8)
    };
    let players = it.read_u8()?;
    let max_players = it.read_u8()?;
    let bots = it.read_u8()?;
    let server_type = ServerType::from_byte(it.read_u8()?);
    let os = Os::from_byte(it.read_u8()?);
    // "Visibility" is the password flag: 1 = private.
    let password = it.read_u8()? != 0;
    let vac = if it.read_u8()? != 0 {
        Vac::Secured
    } else {
        Vac::Unsecured
    };
    let version = it.read_cstr().unwrap_or_default();

    Ok(ServerInfoReply {
        endpoint,
        hostname,
        map,
        gamedir,
        gamedesc,
        players,
        max_players,
        bots,
        server_type,
        os,
        password,
        vac,
        version,
    })
}

/// `0x6D` / `0x43` — the native GoldSrc layout.
fn parse_info_goldsrc(endpoint: Endpoint, buf: &[u8], has_bots: bool) -> Result<ServerInfoReply> {
    let mut it = CStrIter::new(buf);
    let _addr = it.read_cstr().ok();
    let hostname = it.read_cstr()?;
    let map = it.read_cstr()?;
    let gamedir = it.read_cstr()?;
    let gamedesc = it.read_cstr()?;
    let players = it.read_u8()?;
    let max_players = it.read_u8()?;
    let _protocol = it.read_u8()?;
    let server_type = ServerType::from_byte(it.read_u8()?);
    let os = Os::from_byte(it.read_u8()?);
    let password = it.read_u8()? != 0;
    if it.read_u8()? == 1 {
        // Half-Life mod block; nothing in it is shown.
        let _link = it.read_cstr()?;
        let _download = it.read_cstr()?;
        let _null = it.read_u8()?;
        let _mod_version = it.read_i32()?;
        let _size = it.read_i32()?;
        let _server_only = it.read_u8()?;
        let _custom_dll = it.read_u8()?;
    }
    let vac = if it.read_u8()? != 0 {
        Vac::Secured
    } else {
        Vac::Unsecured
    };
    let bots = if has_bots {
        it.read_u8().unwrap_or(0)
    } else {
        0
    };
    // This shape carries no game version; the listing's value stands in.
    let version = String::new();

    Ok(ServerInfoReply {
        endpoint,
        hostname,
        map,
        gamedir,
        gamedesc,
        players,
        max_players,
        bots,
        server_type,
        os,
        password,
        vac,
        version,
    })
}

/// Parse an A2S_PLAYER payload.
///
/// Layout: `FF FF FF FF 44 <count:u8> <Player[]>` where each player is
/// `<index:u8> <name:cstr> <score:i32le> <duration:f32le>`.
pub fn parse_players(buf: &[u8]) -> Result<ServerPlayersReply> {
    let buf = strip_marker(buf);
    if buf.is_empty() || buf[0] != PLAYER_HEADER {
        bail!(
            "bad player header: {:#x}",
            buf.first().copied().unwrap_or(0)
        );
    }
    let mut it = CStrIter::new(&buf[1..]);
    let count = it.read_u8()? as usize;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let idx = it.read_u8()?;
        let name = it.read_cstr()?;
        let score = it.read_i32()?;
        let dur = it.read_f32()?;
        out.push(crate::model::Player {
            index: idx,
            name,
            score,
            duration_seconds: dur,
        });
    }
    Ok(ServerPlayersReply { players: out })
}

/// Build a connectionless request: `FF FF FF FF <opcode> [challenge]`.
///
/// With `challenge == None` the four-byte `0xFFFFFFFF` placeholder is
/// appended, which is what GoldSrc expects for the first attempt at
/// `A2S_PLAYER` / `A2S_RULES`. The challenge-less form (`FF FF FF FF 57`) is
/// used for `A2A_GETCHALLENGE`.
pub fn build_request(opcode: u8, challenge: Option<i32>) -> Vec<u8> {
    let mut pkt = Vec::with_capacity(9);
    pkt.extend_from_slice(&CONNECTIONLESS);
    pkt.push(opcode);
    match challenge {
        Some(ch) => pkt.extend_from_slice(&ch.to_le_bytes()),
        None => pkt.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF]),
    }
    pkt
}

/// `A2A_GETCHALLENGE`: `FF FF FF FF 57` (no trailing placeholder).
pub fn build_challenge_request() -> Vec<u8> {
    let mut pkt = Vec::with_capacity(5);
    pkt.extend_from_slice(&CONNECTIONLESS);
    pkt.push(REQ_CHALLENGE);
    pkt
}

/// The GoldSrc/Source info request: `FF FF FF FF "TSource Engine Query"\0`.
///
/// GoldSrc consumes the leading long as its connectionless marker and then
/// tokenises the remaining bytes as the command, so this spelling reaches the
/// `A2S_INFO` handler. A bare `0x54` does not (verified against live servers),
/// and a bare `0x69` is `A2A_PING`.
pub fn build_info_request() -> Vec<u8> {
    let mut pkt = Vec::with_capacity(4 + INFO_REQUEST_STRING.len());
    pkt.extend_from_slice(&CONNECTIONLESS);
    pkt.extend_from_slice(INFO_REQUEST_STRING);
    pkt
}

/// The same info request with a challenge appended.
///
/// The challenge is *appended to the query string*, it does not replace it:
/// `FF FF FF FF "TSource Engine Query"\0 <challenge:i32le>`. Sending
/// `FF FF FF FF 54 <challenge>` gets no reply at all.
pub fn build_info_request_challenged(challenge: i32) -> Vec<u8> {
    let mut pkt = build_info_request();
    pkt.extend_from_slice(&challenge.to_le_bytes());
    pkt
}

/// Receive one datagram **from `target`**, ignoring anything else.
///
/// A2S is connectionless UDP: without checking the source, any datagram that
/// happens to arrive (a late reply to a previous query on a reused socket, or a
/// spoofed packet) is accepted as the answer. Reading with `recv_from` and
/// discarding datagrams from other peers makes the parse correct by
/// construction, and lets sockets be safely pooled.
async fn recv_from_peer(
    sock: &tokio::net::UdpSocket,
    target: std::net::SocketAddr,
    timeout_ms: u64,
    buf: &mut [u8],
) -> Result<usize> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            bail!("timeout");
        }
        let (n, from) = match timeout(remaining, sock.recv_from(buf)).await {
            Ok(Ok(v)) => v,
            // Windows reports an ICMP "port unreachable" for an *earlier*
            // datagram as `WSAECONNRESET` on the next receive. On a pooled
            // socket that earlier datagram was usually sent to a different,
            // dead server, so failing here would kill a healthy query. The
            // error carries no peer address; keep waiting for ours.
            Ok(Err(e)) if e.kind() == std::io::ErrorKind::ConnectionReset => continue,
            Ok(Err(e)) => bail!("recv: {e}"),
            Err(_) => bail!("timeout"),
        };
        if from == target {
            return Ok(n);
        }
        // Not our server: keep waiting, but never past the deadline.
    }
}

/// As [`recv_from_peer`], but discards info replies.
///
/// Many GoldSrc servers answer one info request **twice**, a `0x6D` and then a
/// `0x49` (measured on about half the populated servers in a live list). The
/// info query returns on the first, so the second is still queued on the
/// socket when the player or rules request goes out, and was read as its
/// answer: the player list came back "unavailable" on servers the in-game
/// browser shows players for.
async fn recv_reply(
    sock: &tokio::net::UdpSocket,
    target: SocketAddr,
    timeout_ms: u64,
    buf: &mut [u8],
) -> Result<usize> {
    let deadline = Instant::now() + Duration::from_millis(timeout_ms);
    loop {
        let left = deadline.saturating_duration_since(Instant::now()).as_millis() as u64;
        if left == 0 {
            bail!("timeout");
        }
        let n = recv_from_peer(sock, target, left, buf).await?;
        if !looks_like_info(strip_marker(&buf[..n])) {
            return Ok(n);
        }
        tracing::debug!(%target, "[a2s] discarded a duplicate info reply");
    }
}

/// Strip a leading connectionless marker, if present.
fn strip_marker(buf: &[u8]) -> &[u8] {
    if buf.len() >= 4 && buf[..4] == CONNECTIONLESS {
        &buf[4..]
    } else {
        buf
    }
}

/// Does this reply look like a server-info payload?
fn looks_like_info(body: &[u8]) -> bool {
    matches!(
        body.first().copied(),
        Some(INFO_HEADER)
            | Some(INFO_HEADER_GOLDSRC)
            | Some(INFO_HEADER_SOURCE)
            | Some(INFO_HEADER_DEPRECATED)
    )
}

/// Send the A2S_INFO query and parse the response.
///
/// Handles the optional challenge round-trip: some servers answer the info
/// request directly, others reply with `0x41 <challenge>` first and expect it
/// echoed back.
///
/// Binds a fresh socket. Prefer [`query_info_on`] inside a scan so the socket
/// can be shared across servers.
pub async fn query_info_udp(endpoint: Endpoint, timeout_ms: u64) -> Result<(ServerInfoReply, u32)> {
    let local = UdpSocket::bind("0.0.0.0:0").await?;
    query_info_on(&local, endpoint, timeout_ms).await
}

/// As [`query_info_udp`], but reuses a caller-owned socket.
///
/// Reusing one socket across a whole scan removes a bind+close syscall pair and
/// an ephemeral-port allocation per server — the dominant per-server cost when
/// sweeping tens of thousands of endpoints.
///
/// Returns the reply and the server's **round-trip time** in ms: the fastest
/// single request→reply exchange, which is what the in-game browser shows as
/// latency. It is deliberately not the duration of the whole query: a
/// challenged server (every current ReHLDS build) needs two exchanges, and
/// summing them reported double the real ping.
pub async fn query_info_on(
    local: &tokio::net::UdpSocket,
    endpoint: Endpoint,
    timeout_ms: u64,
) -> Result<(ServerInfoReply, u32)> {
    let target = endpoint.socket_addr();

    let sent = Instant::now();
    local.send_to(&build_info_request(), &target).await?;

    let mut buf = vec![0u8; UDP_RECV_BUF];
    let n = recv_from_peer(local, target, timeout_ms, &mut buf).await?;
    let rtt = elapsed_ms(sent);
    let data = buf[..n].to_vec();

    // Split packet? Reassemble before deciding what we have.
    let data = if data.first() == Some(&PACKET_SPLIT) {
        reassemble_split(local, target, data, n).await?
    } else {
        data
    };

    let body = strip_marker(&data);

    // Server asked us to prove we can receive by echoing a challenge.
    if body.first() == Some(&CHALLENGE_HEADER) {
        if body.len() < 5 {
            bail!("short challenge reply");
        }
        let ch = i32::from_le_bytes(body[1..5].try_into().unwrap());
        tracing::debug!(endpoint = %endpoint, challenge = ch, "[a2s] info challenge; retrying");
        let sent2 = Instant::now();
        local
            .send_to(&build_info_request_challenged(ch), &target)
            .await?;
        let mut buf2 = vec![0u8; UDP_RECV_BUF];
        let n2 = recv_from_peer(local, target, timeout_ms, &mut buf2).await?;
        let rtt = rtt.min(elapsed_ms(sent2));
        let d2 = buf2[..n2].to_vec();
        let d2 = if d2.first() == Some(&PACKET_SPLIT) {
            reassemble_split(local, target, d2, n2).await?
        } else {
            d2
        };
        return Ok((parse_info(endpoint, &d2)?, rtt));
    }

    if !looks_like_info(body) {
        bail!(
            "info: unexpected header {:#x} (len={})",
            body.first().copied().unwrap_or(0),
            data.len()
        );
    }
    Ok((parse_info(endpoint, &data)?, rtt))
}

fn elapsed_ms(since: Instant) -> u32 {
    u32::try_from(since.elapsed().as_millis()).unwrap_or(u32::MAX)
}

/// Collect the remaining chunks of a split packet.
///
/// Chunks are only accepted from `target`, for the same reason as
/// [`recv_from_peer`]: a pooled socket may see unrelated datagrams.
async fn reassemble_split(
    sock: &tokio::net::UdpSocket,
    target: SocketAddr,
    first: Vec<u8>,
    first_len: usize,
) -> Result<Vec<u8>> {
    if first_len < 6 {
        bail!("short split packet");
    }
    let _seq = i32::from_le_bytes(first[2..6].try_into().unwrap());
    let mut collected = vec![first[6..first_len].to_vec()];
    let mut chunks = 1u32;
    for _ in 0..16 {
        let mut more = vec![0u8; UDP_RECV_BUF];
        // Short per-chunk deadline: the rest of a split reply normally follows
        // immediately, and the tail must not delay the whole scan.
        match recv_from_peer(sock, target, 60, &mut more).await {
            Ok(m) if m > 6 && more[0] == PACKET_SPLIT => {
                collected.push(more[6..m].to_vec());
                chunks += 1;
            }
            _ => break,
        }
    }
    tracing::debug!(target = %target, chunks, "[a2s] reassembled split packet");
    Ok(collected.into_iter().flatten().collect())
}

/// Query the player list.
///
/// `FF FF FF FF 55 FF FF FF FF` → `FF FF FF FF 44 <count> <Player[]>`.
///
/// GoldSrc answers the `0xFFFFFFFF` placeholder in one shot; a server that
/// wants a real challenge replies `0x41 <challenge>` first, and we retry with
/// that value.
pub async fn query_players_udp(endpoint: Endpoint, timeout_ms: u64) -> Result<ServerPlayersReply> {
    let local = UdpSocket::bind("0.0.0.0:0").await?;
    query_players_on(&local, endpoint, timeout_ms).await
}

/// As [`query_players_udp`], but reuses a caller-owned socket.
pub async fn query_players_on(
    local: &tokio::net::UdpSocket,
    endpoint: Endpoint,
    timeout_ms: u64,
) -> Result<ServerPlayersReply> {
    parse_players(&challenged_request(local, endpoint, REQ_PLAYER, timeout_ms).await?)
}

/// Challenges a server may hand out before it answers. Some issue a fresh one
/// in reply to the challenge they just gave (measured: `41 <a>`, then
/// `41 <b>`, then the player list), so the first echo is not always enough.
const MAX_CHALLENGES: usize = 3;

/// Send a player or rules request, following the server's challenges, and
/// return the reply (reassembled when split).
///
/// `FF FF FF FF <opcode> FF FF FF FF` asks without a challenge; a server that
/// wants one answers `41 <challenge>` and the request is repeated with it.
async fn challenged_request(
    local: &tokio::net::UdpSocket,
    endpoint: Endpoint,
    opcode: u8,
    timeout_ms: u64,
) -> Result<Vec<u8>> {
    let target = endpoint.socket_addr();
    let mut buf = vec![0u8; UDP_RECV_BUF];
    let mut challenge = None;
    for _ in 0..=MAX_CHALLENGES {
        local
            .send_to(&build_request(opcode, challenge), &target)
            .await?;
        let n = recv_reply(local, target, timeout_ms, &mut buf).await?;
        let body = strip_marker(&buf[..n]);
        if body.first() != Some(&CHALLENGE_HEADER) {
            return if buf[0] == PACKET_SPLIT {
                reassemble_goldsrc(local, target, &buf[..n]).await
            } else {
                Ok(buf[..n].to_vec())
            };
        }
        if body.len() < 5 {
            bail!("short challenge reply");
        }
        challenge = Some(i32::from_le_bytes(body[1..5].try_into().unwrap()));
    }
    bail!("server kept re-issuing challenges")
}

/// Parse an A2S_RULES payload into `(name, value)` pairs.
///
/// Layout: `FF FF FF FF 45 <count:u16le> <(name, value) CString pairs>`. A
/// truncated tail keeps the pairs read so far: a bot plugin's cvar is only
/// looked up, never required.
pub fn parse_rules(buf: &[u8]) -> Result<Vec<(String, String)>> {
    let buf = strip_marker(buf);
    if buf.is_empty() || buf[0] != RULES_HEADER {
        bail!(
            "bad rules header: {:#x}",
            buf.first().copied().unwrap_or(0)
        );
    }
    let mut it = CStrIter::new(&buf[1..]);
    let count = it.read_u8()? as usize | (it.read_u8()? as usize) << 8;
    let mut out = Vec::with_capacity(count);
    for _ in 0..count {
        let (Ok(name), Ok(value)) = (it.read_cstr(), it.read_cstr()) else {
            break;
        };
        out.push((name, value));
    }
    Ok(out)
}

/// Reassemble a GoldSrc split reply.
///
/// Each chunk is `FE FF FF FF <id:i32le> <number:u8> <payload>`, where the
/// number's low nibble is the chunk count and its high nibble this chunk's
/// index (measured: a ReHLDS rules reply arrives as two such chunks, even from
/// a server whose info reply is the Source-style `0x49`). Chunks may arrive
/// out of order, so they are placed by index.
async fn reassemble_goldsrc(
    sock: &tokio::net::UdpSocket,
    target: SocketAddr,
    first: &[u8],
) -> Result<Vec<u8>> {
    fn split_header(d: &[u8]) -> Option<(i32, usize, usize)> {
        if d.len() < 9 || d[0] != PACKET_SPLIT {
            return None;
        }
        let id = i32::from_le_bytes(d[4..8].try_into().unwrap());
        Some((id, (d[8] >> 4) as usize, (d[8] & 0x0F) as usize))
    }
    let Some((id, index, total)) = split_header(first) else {
        bail!("short split packet");
    };
    if total == 0 || index >= total {
        bail!("bad split numbering");
    }
    let mut chunks: Vec<Option<Vec<u8>>> = vec![None; total];
    chunks[index] = Some(first[9..].to_vec());
    while chunks.iter().any(Option::is_none) {
        let mut more = vec![0u8; UDP_RECV_BUF];
        // The rest of a split reply follows immediately; a short deadline
        // keeps a lost chunk from stalling the sweep.
        let m = recv_from_peer(sock, target, 250, &mut more).await?;
        match split_header(&more[..m]) {
            Some((i, idx, t)) if i == id && t == total && idx < total => {
                chunks[idx] = Some(more[9..m].to_vec());
            }
            _ => {}
        }
    }
    Ok(chunks.into_iter().flatten().flatten().collect())
}

/// Query the server's rules (its public cvars).
///
/// `FF FF FF FF 56 FF FF FF FF` → `41 <challenge>`, then the request again
/// with that challenge → `45 …`, usually split across datagrams.
pub async fn query_rules_on(
    local: &tokio::net::UdpSocket,
    endpoint: Endpoint,
    timeout_ms: u64,
) -> Result<Vec<(String, String)>> {
    parse_rules(&challenged_request(local, endpoint, REQ_RULES, timeout_ms).await?)
}

/// Bot plugins, recognised by the cvar prefix each one registers. A plugin
/// can make the server report its bots as humans (YaPB's query hook does:
/// measured on a server answering `bots = 0` with 18 of its 20 players bots),
/// so the plugin being installed is the only signal the A2S reply still gives.
const BOT_PLUGINS: &[(&str, &str)] = &[
    ("yb_", "YaPB"),
    ("pb_", "PODBot"),
    ("ebot_", "E-Bot"),
    ("jk_botti", "JK_Botti"),
    ("rcbot", "RCBot"),
];

/// Name the bot plugin a server's rules reveal, with its version when one of
/// its cvars carries one. `bot_quota` above zero counts too: that is the
/// built-in CZ bot (ReGameDLL) with bots switched on.
pub fn bot_plugin(rules: &[(String, String)]) -> Option<String> {
    for (prefix, plugin) in BOT_PLUGINS {
        let mut cvars = rules
            .iter()
            .filter(|(k, _)| k.to_ascii_lowercase().starts_with(prefix));
        if let Some(first) = cvars.next() {
            let version = std::iter::once(first)
                .chain(cvars)
                .find(|(k, _)| k.to_ascii_lowercase().contains("ver"))
                .map(|(_, v)| v.trim())
                .filter(|v| !v.is_empty());
            return Some(match version {
                Some(v) => format!("{plugin} {v}"),
                None => plugin.to_string(),
            });
        }
    }
    rules
        .iter()
        .find(|(k, v)| k.eq_ignore_ascii_case("bot_quota") && v.trim().parse::<f32>().is_ok_and(|q| q > 0.0))
        .map(|_| "CZ bots".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    fn rules(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn parses_rules_with_marker() {
        let mut pkt = CONNECTIONLESS.to_vec();
        pkt.push(RULES_HEADER);
        pkt.extend_from_slice(&2u16.to_le_bytes());
        pkt.extend_from_slice(b"mp_timelimit\x0020\x00yb_version\x004.4.957\x00");
        let r = parse_rules(&pkt).unwrap();
        assert_eq!(r, rules(&[("mp_timelimit", "20"), ("yb_version", "4.4.957")]));
    }

    #[test]
    fn truncated_rules_keep_the_pairs_read() {
        let mut pkt = vec![RULES_HEADER];
        pkt.extend_from_slice(&3u16.to_le_bytes());
        pkt.extend_from_slice(b"a\x001\x00b\x00");
        assert_eq!(parse_rules(&pkt).unwrap(), rules(&[("a", "1")]));
    }

    #[test]
    fn names_the_bot_plugin() {
        // The cvars 46.205.242.7:27015 answers with (trimmed).
        let live = rules(&[
            ("amxmodx_version", "1.9.0.5303"),
            ("mp_timelimit", "20"),
            ("whb_version", "1.5.697"),
            ("yb_version", "4.4.957"),
        ]);
        assert_eq!(bot_plugin(&live).as_deref(), Some("YaPB 4.4.957"));
        assert_eq!(bot_plugin(&rules(&[("pb_bot_quota", "4")])).as_deref(), Some("PODBot"));
        assert_eq!(bot_plugin(&rules(&[("bot_quota", "6")])).as_deref(), Some("CZ bots"));
        assert_eq!(bot_plugin(&rules(&[("bot_quota", "0")])), None);
        assert_eq!(bot_plugin(&rules(&[("amxmodx_version", "1.9")])), None);
    }

    fn info_reply(header: u8) -> Vec<u8> {
        let mut r = CONNECTIONLESS.to_vec();
        r.push(header);
        if header == INFO_HEADER_SOURCE {
            r.push(48);
        } else {
            r.extend_from_slice(b"127.0.0.1:0\0");
        }
        for f in ["Srv", "de_dust2", "cstrike", "CS"] {
            r.extend_from_slice(f.as_bytes());
            r.push(0);
        }
        if header == INFO_HEADER_SOURCE {
            r.extend_from_slice(&10u16.to_le_bytes());
            r.extend_from_slice(&[1, 32, 0, b'd', b'l', 0, 1]);
            r.extend_from_slice(b"1.1.2.7\0");
        } else {
            r.extend_from_slice(&[1, 32, 48, b'd', b'l', 0, 0, 1, 0]);
        }
        r
    }

    fn player_reply(name: &str) -> Vec<u8> {
        let mut r = CONNECTIONLESS.to_vec();
        r.extend_from_slice(&[PLAYER_HEADER, 1, 0]);
        r.extend_from_slice(name.as_bytes());
        r.push(0);
        r.extend_from_slice(&3i32.to_le_bytes());
        r.extend_from_slice(&60.0f32.to_le_bytes());
        r
    }

    /// Regression: a server that answers one info request with both a `0x6D`
    /// and a `0x49` left the second queued, and it was read as the player
    /// reply — "Player list unavailable" on a server with players.
    #[tokio::test]
    async fn duplicate_info_reply_is_not_read_as_the_player_list() {
        let server = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap();
        std::thread::spawn(move || {
            let mut buf = [0u8; 256];
            let (_, peer) = server.recv_from(&mut buf).unwrap();
            server.send_to(&info_reply(INFO_HEADER_GOLDSRC), peer).unwrap();
            server.send_to(&info_reply(INFO_HEADER_SOURCE), peer).unwrap();
            let (_, peer) = server.recv_from(&mut buf).unwrap();
            server.send_to(&player_reply("alice"), peer).unwrap();
        });
        let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let ep = Endpoint::new(Ipv4Addr::LOCALHOST, addr.port());
        query_info_on(&sock, ep, 2_000).await.unwrap();
        // Let the duplicate land before the player request goes out.
        tokio::time::sleep(Duration::from_millis(50)).await;
        let players = query_players_on(&sock, ep, 2_000).await.unwrap();
        assert_eq!(players.players[0].name, "alice");
    }

    /// A server may answer a challenge with a fresh one before it answers.
    #[tokio::test]
    async fn follows_a_reissued_challenge() {
        let server = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap();
        std::thread::spawn(move || {
            let mut buf = [0u8; 256];
            for ch in [[1u8; 4], [2u8; 4]] {
                let (_, peer) = server.recv_from(&mut buf).unwrap();
                let mut r = CONNECTIONLESS.to_vec();
                r.push(CHALLENGE_HEADER);
                r.extend_from_slice(&ch);
                server.send_to(&r, peer).unwrap();
            }
            let (n, peer) = server.recv_from(&mut buf).unwrap();
            assert_eq!(&buf[5..n], &[2u8; 4], "the latest challenge is echoed");
            server.send_to(&player_reply("bob"), peer).unwrap();
        });
        let ep = Endpoint::new(Ipv4Addr::LOCALHOST, addr.port());
        let players = query_players_udp(ep, 2_000).await.unwrap();
        assert_eq!(players.players[0].name, "bob");
    }

    /// A rules reply split in two GoldSrc chunks, delivered out of order after
    /// a challenge — the shape a live ReHLDS server answers with.
    #[tokio::test]
    async fn queries_split_rules_after_a_challenge() {
        let server = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap();
        std::thread::spawn(move || {
            let mut buf = [0u8; 256];
            let (_, peer) = server.recv_from(&mut buf).unwrap();
            server
                .send_to(&[0xFF, 0xFF, 0xFF, 0xFF, CHALLENGE_HEADER, 9, 9, 9, 9], peer)
                .unwrap();
            let (n, peer) = server.recv_from(&mut buf).unwrap();
            assert_eq!(&buf[..n], &build_request(REQ_RULES, Some(i32::from_le_bytes([9; 4]))));
            let mut body = CONNECTIONLESS.to_vec();
            body.push(RULES_HEADER);
            body.extend_from_slice(&2u16.to_le_bytes());
            body.extend_from_slice(b"sv_gravity\x00800\x00yb_version\x004.4.957\x00");
            let (a, b) = body.split_at(12);
            let chunk = |num: u8, part: &[u8]| {
                let mut c = vec![PACKET_SPLIT, 0xFF, 0xFF, 0xFF, 7, 0, 0, 0, num];
                c.extend_from_slice(part);
                c
            };
            server.send_to(&chunk(0x12, b), peer).unwrap();
            server.send_to(&chunk(0x02, a), peer).unwrap();
        });

        let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let ep = Endpoint::new(Ipv4Addr::LOCALHOST, addr.port());
        let r = query_rules_on(&sock, ep, 2_000).await.unwrap();
        assert_eq!(bot_plugin(&r).as_deref(), Some("YaPB 4.4.957"));
    }

    #[test]
    fn parses_goldsrc_detailed_info() {
        // GoldSrc 0x6D reply with a mod block: VAC and bots come after it.
        let mut pkt = vec![INFO_HEADER_GOLDSRC];
        for f in [
            "127.0.0.1:27015",
            "Test Server",
            "de_dust2",
            "cstrike",
            "Counter-Strike",
        ] {
            pkt.extend_from_slice(f.as_bytes());
            pkt.push(0);
        }
        pkt.push(20); // players
        pkt.push(32); // max
        pkt.push(48); // protocol
        pkt.push(b'd'); // dedicated
        pkt.push(b'w'); // windows
        pkt.push(1); // visibility: password protected
        pkt.push(1); // mod block follows
        pkt.extend_from_slice(b"http://mod\0http://dl\0");
        pkt.push(0); // null
        pkt.extend_from_slice(&1i32.to_le_bytes()); // mod version
        pkt.extend_from_slice(&1024i32.to_le_bytes()); // size
        pkt.push(0); // type
        pkt.push(1); // dll
        pkt.push(1); // vac
        pkt.push(2); // bots

        let ep = Endpoint::new(Ipv4Addr::LOCALHOST, 27015);
        let r = parse_info(ep, &pkt).unwrap();
        assert_eq!(r.hostname, "Test Server");
        assert_eq!(r.map, "de_dust2");
        assert_eq!(r.gamedir, "cstrike");
        assert_eq!(r.gamedesc, "Counter-Strike");
        assert_eq!(r.players, 20);
        assert_eq!(r.max_players, 32);
        assert_eq!(r.bots, 2);
        assert!(r.password);
        assert!(matches!(r.server_type, ServerType::Dedicated));
        assert!(matches!(r.os, Os::Windows));
        assert_eq!(r.vac, Vac::Secured);
    }

    /// Regression: without a mod block the byte after `visibility` is the mod
    /// flag, not VAC. Reading it as VAC reported every secured 0x6D server
    /// as unsecured (measured: Steam lists them as secure).
    #[test]
    fn goldsrc_vac_is_read_after_the_mod_flag() {
        let mut pkt = vec![INFO_HEADER_GOLDSRC];
        for f in ["1.2.3.4:27015", "Secure Srv", "de_dust2", "cstrike", "CS"] {
            pkt.extend_from_slice(f.as_bytes());
            pkt.push(0);
        }
        // players max proto type env vis mod vac bots
        pkt.extend_from_slice(&[5, 32, 48, b'd', b'l', 0, 0, 1, 1]);
        let ep = Endpoint::new(Ipv4Addr::new(1, 2, 3, 4), 27015);
        let r = parse_info(ep, &pkt).unwrap();
        assert_eq!(r.vac, Vac::Secured);
        assert_eq!(r.bots, 1);
        assert!(!r.password);
    }

    #[test]
    fn parses_source_style_0x6e_info() {
        // 0x6E uses the same layout as 0x49.
        let mut pkt = vec![INFO_HEADER];
        pkt.push(48);
        for f in ["Srv", "de_aztec", "cstrike", "CS"] {
            pkt.extend_from_slice(f.as_bytes());
            pkt.push(0);
        }
        pkt.extend_from_slice(&10u16.to_le_bytes());
        pkt.push(5); // players
        pkt.push(16); // max
        pkt.push(0); // bots
        pkt.push(b'd');
        pkt.push(b'l');
        pkt.push(0); // visibility
        pkt.push(0); // vac
        pkt.extend_from_slice(b"1.1.2.7\x00");

        let ep = Endpoint::new(Ipv4Addr::LOCALHOST, 27015);
        let r = parse_info(ep, &pkt).unwrap();
        assert_eq!(r.hostname, "Srv");
        assert_eq!(r.map, "de_aztec");
        assert_eq!(r.players, 5);
        assert_eq!(r.max_players, 16);
        assert_eq!(r.version, "1.1.2.7");
    }

    #[test]
    fn accepts_connectionless_marker() {
        let mut pkt = Vec::new();
        pkt.extend_from_slice(&CONNECTIONLESS);
        pkt.push(INFO_HEADER_GOLDSRC);
        for f in ["1.2.3.4:27015", "N", "map", "cstrike", "CS"] {
            pkt.extend_from_slice(f.as_bytes());
            pkt.push(0);
        }
        pkt.extend_from_slice(&[1, 2, 48, b'd', b'l', 0, 0, 0, 0]);
        let ep = Endpoint::new(Ipv4Addr::new(1, 2, 3, 4), 27015);
        let r = parse_info(ep, &pkt).unwrap();
        assert_eq!(r.hostname, "N");
    }

    #[test]
    fn non_utf8_hostname_is_decoded_lossily_not_rejected() {
        // cp1251 "Сервер" — raw bytes a Russian admin's console produces.
        let mut pkt = vec![INFO_HEADER_GOLDSRC];
        pkt.extend_from_slice(b"1.2.3.4:27015\0");
        pkt.extend_from_slice(&[0xD1, 0xE5, 0xF0, 0xE2, 0xE5, 0xF0, b' ', b'1', 0]);
        for f in ["de_dust2", "cstrike", "CS"] {
            pkt.extend_from_slice(f.as_bytes());
            pkt.push(0);
        }
        pkt.extend_from_slice(&[3, 32, 48, b'd', b'l', 0, 0, 1, 0]);
        let ep = Endpoint::new(Ipv4Addr::new(1, 2, 3, 4), 27015);
        let r = parse_info(ep, &pkt).expect("a codepage hostname must not drop the server");
        assert!(r.hostname.ends_with(" 1"), "{}", r.hostname);
        assert_eq!(r.map, "de_dust2");
        assert_eq!(r.players, 3);
    }

    /// The reported latency is one exchange, not the whole challenged query:
    /// summing both exchanges doubled every ping on current ReHLDS servers.
    #[tokio::test]
    async fn rtt_is_the_fastest_exchange_not_the_whole_query() {
        let server = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let addr = server.local_addr().unwrap();
        std::thread::spawn(move || {
            let mut buf = [0u8; 256];
            // First exchange is slow (the challenge), second is instant.
            let (_, peer) = server.recv_from(&mut buf).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(200));
            server
                .send_to(
                    &[0xFF, 0xFF, 0xFF, 0xFF, CHALLENGE_HEADER, 1, 2, 3, 4],
                    peer,
                )
                .unwrap();
            let (_, peer) = server.recv_from(&mut buf).unwrap();
            let mut reply = CONNECTIONLESS.to_vec();
            reply.push(INFO_HEADER_GOLDSRC);
            for f in ["127.0.0.1:0", "Srv", "de_dust2", "cstrike", "CS"] {
                reply.extend_from_slice(f.as_bytes());
                reply.push(0);
            }
            reply.extend_from_slice(&[1, 32, 48, b'd', b'l', 0, 0, 1, 0]);
            server.send_to(&reply, peer).unwrap();
        });

        let ep = Endpoint::new(Ipv4Addr::LOCALHOST, addr.port());
        let started = std::time::Instant::now();
        let (info, rtt) = query_info_udp(ep, 2_000).await.unwrap();
        assert_eq!(info.hostname, "Srv");
        assert!(started.elapsed().as_millis() >= 200);
        assert!(
            rtt < 150,
            "rtt {rtt}ms must not include the slow challenge exchange"
        );
    }

    #[test]
    fn rejects_bare_ping_header() {
        // 0x69 is A2A_PING, not an info reply.
        let ep = Endpoint::new(Ipv4Addr::LOCALHOST, 27015);
        assert!(parse_info(ep, &[REQ_PING, 0, 0]).is_err());
    }

    #[test]
    fn parses_source_0x49_info() {
        // Byte-for-byte from a live server (captured hex, with the address
        // string replaced since 0x49 does not carry one).
        let mut pkt = Vec::new();
        pkt.push(INFO_HEADER_SOURCE); // 0x49
        pkt.push(48); // protocol
        for f in [
            "FRAGHUB 1.6 | Poland [FFA]",
            "cs_backalley",
            "cstrike",
            "Counter-Strike",
        ] {
            pkt.extend_from_slice(f.as_bytes());
            pkt.push(0);
        }
        pkt.extend_from_slice(&10u16.to_le_bytes()); // appid
        pkt.push(0); // players
        pkt.push(21); // maxplayers
        pkt.push(0); // bots
        pkt.push(b'd'); // servertype
        pkt.push(b'l'); // environment
        pkt.push(0); // visibility
        pkt.push(1); // vac
        pkt.extend_from_slice(b"1.1.2.7/Stdio\x00");
        pkt.push(0xB1); // ExtraDataFlag
        pkt.push(8); // bots (EDF 0x80)

        let ep = Endpoint::new(Ipv4Addr::LOCALHOST, 27015);
        let r = parse_info(ep, &pkt).unwrap();
        assert_eq!(r.hostname, "FRAGHUB 1.6 | Poland [FFA]");
        assert_eq!(r.map, "cs_backalley");
        assert_eq!(r.gamedir, "cstrike");
        assert_eq!(r.gamedesc, "Counter-Strike");
        assert_eq!(r.players, 0);
        assert_eq!(r.max_players, 21);
        assert_eq!(r.bots, 0); // read from the dedicated byte, not EDF
        assert_eq!(r.version, "1.1.2.7/Stdio");
        assert!(matches!(r.os, Os::Linux));
        assert_eq!(r.vac, Vac::Secured);
        assert!(!r.password);

        // The same reply from a passworded server.
        let vis = pkt.len() - b"1.1.2.7/Stdio ".len() - 4;
        pkt[vis] = 1;
        assert!(parse_info(ep, &pkt).unwrap().password);
    }

    #[test]
    fn info_request_matches_goldsrc_expectations() {
        let req = build_info_request();
        assert_eq!(&req[..4], &CONNECTIONLESS);
        assert_eq!(&req[4..], b"TSource Engine Query\0");
        assert_eq!(REQ_INFO, b'T');
        assert_ne!(REQ_INFO, REQ_PING);
    }

    #[test]
    fn builds_challenge_request() {
        assert_eq!(
            build_challenge_request(),
            vec![0xFF, 0xFF, 0xFF, 0xFF, b'W']
        );
    }

    #[test]
    fn builds_player_request_with_placeholder() {
        // First attempt uses the 0xFFFFFFFF placeholder, not a bare opcode.
        let req = build_request(REQ_PLAYER, None);
        assert_eq!(
            req,
            vec![0xFF, 0xFF, 0xFF, 0xFF, b'U', 0xFF, 0xFF, 0xFF, 0xFF]
        );
        // Retry echoes the server's challenge.
        let req2 = build_request(REQ_PLAYER, Some(0x11223344));
        assert_eq!(req2[..5], [0xFF, 0xFF, 0xFF, 0xFF, b'U']);
        assert_eq!(&req2[5..], &0x11223344i32.to_le_bytes());
    }

    #[test]
    fn parses_players_with_marker() {
        // The real reply carries the connectionless marker before 0x44.
        let mut pkt = Vec::new();
        pkt.extend_from_slice(&CONNECTIONLESS);
        pkt.push(PLAYER_HEADER);
        pkt.push(1);
        pkt.push(0);
        pkt.extend_from_slice(b"player\x00");
        pkt.extend_from_slice(&9i32.to_le_bytes());
        pkt.extend_from_slice(&42.0f32.to_le_bytes());
        let r = parse_players(&pkt).unwrap();
        assert_eq!(r.players.len(), 1);
        assert_eq!(r.players[0].name, "player");
        assert_eq!(r.players[0].score, 9);
    }

    #[test]
    fn parses_players() {
        let mut pkt = vec![PLAYER_HEADER];
        pkt.push(2);
        pkt.push(0);
        pkt.extend_from_slice(b"alice\x00");
        pkt.extend_from_slice(&15i32.to_le_bytes());
        pkt.extend_from_slice(&120.5f32.to_le_bytes());
        pkt.push(1);
        pkt.extend_from_slice(b"bob\x00");
        pkt.extend_from_slice(&7i32.to_le_bytes());
        pkt.extend_from_slice(&300.0f32.to_le_bytes());

        let r = parse_players(&pkt).unwrap();
        assert_eq!(r.players.len(), 2);
        assert_eq!(r.players[0].name, "alice");
        assert_eq!(r.players[1].score, 7);
    }
}
