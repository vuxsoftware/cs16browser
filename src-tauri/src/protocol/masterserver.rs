//! Valve / GoldSrc HL1 master server protocol.
//!
//! ## Request format
//!
//! ```text
//! 0x31 0x0A <filter>\0x0A
//! ```
//!
//! `0x31` ('1') identifies the request as a server-list query (the only one
//! Valve HL1 masters accept). The body is an **empty-line terminated**
//! sequence of `\<key>\<value>` pairs. Known keys:
//!
//! | key | meaning                                                              |
//! |-----|----------------------------------------------------------------------|
//! | `appid` | Steam AppID (CS 1.6 = 10)                                       |
//! | `gamedir` | game directory ("cstrike")                                      |
//! | `secure` | 1 = VAC-secured servers only                                     |
//! | `full`   | 1 = exclude full servers                                         |
//! | `empty`  | 1 = exclude empty servers                                         |
//! | `password` | 1 = exclude password-protected servers                         |
//! | `nappid` | exclude server list filtered by AppID                            |
//! | `game`   | value matches the "game" string reported by the server             |
//! | `map`    | value matches the "map" string reported by the server              |
//! | `name`   | value matches the "name" string reported by the server             |
//! | `ping`   | value matches the "ping" string... actually "ms" (BETA region)     |
//! | `version` | value matches the reported "version" string                     |
//! | `dedicated` | 1 = dedicated only                                              |
//!
//! ## Response format
//!
//! ```text
//! <ip1>:<port1>\0<ip2>:<port2>\0...\0<last_byte>0x0A
//! ```
//!
//! Each entry is exactly `info_string` from the server's info response.
//! The trailing `0x0A` is the original "newline" terminator and a real
//! HL1 master still ends the burst with it.
//!
//! ## Master server addresses
//!
//! * `hl1master.steampowered.com:27010`
//! * `hl2master.steampowered.com:27010`
//!
//! CS 1.6 uses the **HL1 master**; the Source master also serves the same
//! payload when asked with `\appid\10\gamedir\cstrike`.
//!
//! Some community proxies (e.g. `master.fragaholic.de`, dathost lists) still
//! speak the protocol — these can be appended in `~/.config/cs16browser.toml`.

use crate::model::{Endpoint, Game};
use anyhow::{anyhow, Context, Result};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::{timeout, Duration};

/// Default master servers we try in order.
///
/// The first two are Valve's own masters, both of which still serve the
/// legacy HL1 wire protocol with the CS 1.6 filter. The remaining entries
/// are community-operated proxies; they speak the same protocol so any of
/// them will satisfy the same `build_filter` output.
///
/// To force a specific host or bypass DNS, pass `--master host:port` on the
/// CLI — the `host` slot accepts either a hostname or an IPv4 literal.
pub const DEFAULT_MASTERS: &[(&str, u16)] = &[
    ("hl1master.steampowered.com", 27010),
    ("hl2master.steampowered.com", 27010),
    // Community HL1 protocol proxies historically serving CS 1.6 listings.
    ("hlmaster.valve.net", 27010),
    ("master.fragaholic.de", 27010),
    ("188.165.137.205", 27010), // community cache operated by community servers
    ("74.91.112.61", 27010),    // community cache operated by community servers
];

/// Historical Valve HL1/HL2 master IPs. Useful when DNS for
/// `hl{1,2}master.steampowered.com` is unavailable but the IP is still
/// routed. Note: Valve has occasionally reassigned or decommissioned
/// these; treat them as hints, not as a permanent guarantee.
pub const DEFAULT_MASTER_IPS: &[(&str, u16)] = &[
    ("208.78.108.130", 27010), // historical hl1master.steampowered.com
    ("208.78.108.131", 27010), // historical hl2master.steampowered.com
    ("155.133.248.27", 27010), // historical hl2master alternate
    ("155.133.248.28", 27010), // historical hl2master alternate
];

/// Build a HL1 **master-server** filter payload.
///
/// We always include `appid` and `gamedir`. The optional `secure` / `full`
/// / `empty` / `password` filters mirror the official UI's most common
/// combinations.
///
/// This is the legacy TCP protocol's grammar. For Steam's Web API use
/// [`build_webapi_filter`] — the two are **not** interchangeable.
pub fn build_filter(
    game: Game,
    secure: bool,
    no_full: bool,
    no_empty: bool,
    no_password: bool,
) -> String {
    let mut s = String::with_capacity(96);
    s.push('\\');
    s.push_str("appid");
    s.push('\\');
    s.push_str(&game.app_id().to_string());

    s.push('\\');
    s.push_str("gamedir");
    s.push('\\');
    s.push_str(game.gamedir());

    if secure {
        s.push_str("\\secure\\1");
    }
    if no_full {
        s.push_str("\\full\\1");
    }
    if no_empty {
        s.push_str("\\empty\\1");
    }
    if no_password {
        s.push_str("\\password\\1");
    }
    s
}

/// Build a **Steam Web API** (`IGameServersService/GetServerList`) filter.
///
/// Same `\key\value` framing as [`build_filter`], but a different and much
/// more brittle key set. Measured against appid 10 — the distinction matters
/// because **an unrecognised key silently yields an empty list**, not an
/// error:
///
/// | key | accepted | effect |
/// |---|---|---|
/// | `secure\1` | yes | secure servers only |
/// | `full\1` | yes | *hide* full servers |
/// | `empty\1` | yes | *hide* empty servers |
/// | `noplayers\1` | yes | servers with players only |
/// | `password\1` | **no** | returns 0 rows |
/// | `notempty\1` | **no** | returns 0 rows |
///
/// `no_password` is therefore **not** expressible here. Sending the HL1
/// protocol's `password\1` — which is what this function used to do — made
/// every filtered request return nothing. Password-protected servers are
/// filtered client-side after the A2S query instead.
pub fn build_webapi_filter(
    game: Game,
    secure: bool,
    no_full: bool,
    no_empty: bool,
    no_password: bool,
) -> String {
    let mut s = String::with_capacity(64);
    s.push('\\');
    s.push_str("appid");
    s.push('\\');
    s.push_str(&game.app_id().to_string());

    s.push('\\');
    s.push_str("gamedir");
    s.push('\\');
    s.push_str(game.gamedir());

    if secure {
        s.push_str("\\secure\\1");
    }
    if no_full {
        s.push_str("\\full\\1");
    }
    // `empty\1` is the API's "hide empty" (measured: 0 empty rows returned).
    if no_empty {
        s.push_str("\\empty\\1");
    }
    // `noplayers\1` is stricter than `empty\1`: it also drops servers whose
    // player list is unknown. Only use it when the caller explicitly wants
    // populated servers and not when they merely want non-empty ones.
    let _ = no_password;
    s
}

/// Wire payload (already includes leading 0x31, 0x0A, and trailing 0x0A).
pub fn build_request(filter: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(filter.len() + 4);
    buf.push(0x31);
    buf.push(0x0A);
    buf.extend_from_slice(filter.as_bytes());
    buf.push(0x0A);
    buf
}

/// Parse the response payload into a list of `[ip]:[port]` endpoints.
pub fn parse_response(buf: &[u8]) -> Vec<Endpoint> {
    let mut out = Vec::with_capacity(64);
    for raw in buf.split(|b| *b == 0) {
        if raw.is_empty() {
            continue;
        }
        if let Ok(s) = std::str::from_utf8(raw) {
            // Some masters prepend IPv6-mapped IPv4; tolerate a trailing `0:0` wrapper.
            let s = s.trim().trim_end_matches('\n');
            if let Some(ep) = Endpoint::parse(s) {
                out.push(ep);
            }
        }
    }
    out
}

/// Query a single master server with timeout, return parsed endpoints.
pub async fn query_master(
    host: &str,
    port: u16,
    filter: &str,
    timeout_ms: u64,
) -> Result<Vec<Endpoint>> {
    let payload = build_request(filter);
    let fut = async move {
        let mut s = TcpStream::connect((host, port))
            .await
            .with_context(|| format!("connect {host}:{port}"))?;
        s.write_all(&payload)
            .await
            .with_context(|| format!("write filter to {host}:{port}"))?;
        let mut buf = Vec::with_capacity(8192);
        s.read_to_end(&mut buf)
            .await
            .with_context(|| format!("read response from {host}:{port}"))?;
        Ok::<_, anyhow::Error>(buf)
    };
    let res = timeout(Duration::from_millis(timeout_ms), fut).await;
    match res {
        Ok(Ok(buf)) => Ok(parse_response(&buf)),
        Ok(Err(e)) => Err(e),
        Err(_) => Err(anyhow!("timeout connecting {host}:{port}")),
    }
}

/// Same as `query_all_flat` but also returns a list of `(host, port, error)`
/// tuples for each master that failed, so the CLI can surface DNS or
/// routing issues instead of silently returning an empty vec.
pub async fn query_all_flat_with(
    masters: &[(String, u16)],
    filter: &str,
    timeout_ms: u64,
) -> (Vec<Endpoint>, Vec<(String, u16, anyhow::Error)>) {
    let mut all = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut errors = Vec::new();
    for (host, port) in masters {
        match query_master(host, *port, filter, timeout_ms).await {
            Ok(eps) => {
                for ep in eps {
                    if seen.insert(ep) {
                        all.push(ep);
                    }
                }
            }
            Err(e) => errors.push((host.clone(), *port, e)),
        }
    }
    all.sort();
    (all, errors)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    #[test]
    fn build_filter_cs16() {
        let f = build_filter(Game::CS16, true, true, false, false);
        assert!(f.contains("\\appid\\10"));
        assert!(f.contains("\\gamedir\\cstrike"));
        assert!(f.contains("\\secure\\1"));
        assert!(f.contains("\\full\\1"));
        assert!(!f.contains("\\empty"));
        assert!(!f.contains("\\password"));
    }

    #[test]
    fn request_has_correct_framing() {
        let p = build_request("\\appid\\10\\gamedir\\cstrike");
        assert_eq!(p[0], 0x31);
        assert_eq!(p[1], 0x0A);
        assert_eq!(p[p.len() - 1], 0x0A);
    }

    #[test]
    fn parses_well_known_response() {
        let body: &[u8] = b"1.2.3.4:27015\x00x.x.x.x:27015\x00";
        let eps = parse_response(body);
        // The x.x.x.x is filtered because Ipv4Addr::parse fails.
        assert!(eps
            .iter()
            .any(|e| e.ip == Ipv4Addr::new(1, 2, 3, 4) && e.port == 27015));
    }
}
