//! Concurrent server queryer that combines master fetch + per-server info.

use crate::detect::{analyze, Analysis, Context as DetCtx};
use crate::history::NameHistory;
use crate::model::{Endpoint, Game, ServerInfo};
use crate::ratelimit::now_ms;
use chrono::Utc;
use futures::stream::{FuturesUnordered, StreamExt};
use std::collections::{HashMap, HashSet};
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::Instant;

/// Result returned by the querying pipeline.
#[derive(Debug, Clone)]
pub struct ScannedServer {
    pub info: Option<ServerInfo>,
    pub analysis: Analysis,
    pub endpoint: Endpoint,
    pub query_ms: u32,
    pub outcome: Outcome,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Ok,
    Timeout,
    BadHeader,
    ConnectRefused,
    UnknownError(String),
    /// Not queried: the endpoint was inside its anti-spam interval.
    ///
    /// Distinct from a timeout — nothing was sent, so the server must not be
    /// treated as dead or fake.
    RateLimited,
    /// Not queried: the endpoint carries a hard ban from a previous run.
    ///
    /// Skipped entirely (no packet sent) until the ban list is cleared with
    /// `--hard-refresh`.
    Banned,
}

impl Outcome {
    pub fn from_err(e: &anyhow::Error) -> Self {
        let s = e.to_string().to_ascii_lowercase();
        if s.contains("timeout") {
            Outcome::Timeout
        } else if s.contains("refused") {
            Outcome::ConnectRefused
        } else {
            Outcome::UnknownError(e.to_string())
        }
    }
}

/// As [`scan_one_with`], but reuses a caller-supplied socket.
pub async fn scan_one_on(
    sock: &tokio::net::UdpSocket,
    endpoint: Endpoint,
    timeout_ms: u64,
    include_players: bool,
) -> ScannedServer {
    let start = Instant::now();
    let info_res = crate::protocol::a2s::query_info_on(sock, endpoint, timeout_ms).await;
    let players_res = if include_players && info_res.is_ok() {
        crate::protocol::a2s::query_players_on(sock, endpoint, timeout_ms).await
    } else {
        Err(anyhow::anyhow!("players not requested"))
    };
    // Read rules once for both AMXX language and the existing bot-plugin hint.
    // A failed rules request does not turn a successful INFO into a failed row.
    let rules = if info_res.is_ok() {
        match crate::protocol::a2s::query_rules_on(sock, endpoint, timeout_ms).await {
            Ok(rules) => Some(rules),
            Err(e) => {
                tracing::debug!(endpoint = %endpoint, error = %e, "rules query failed");
                None
            }
        }
    } else {
        None
    };
    let elapsed = start.elapsed().as_millis() as u32;
    let mut scan = finish_scan(endpoint, info_res, players_res, elapsed);
    if let Some(info) = scan.info.as_mut() {
        info.bot_plugin = if may_hide_bots_from_server(info) {
            rules
                .as_ref()
                .and_then(|rules| crate::protocol::a2s::bot_plugin(rules))
        } else {
            None
        };
        info.country = crate::country::resolve(endpoint.ip, rules.as_deref()).map(str::to_string);
    }
    scan
}

/// Only a server with players that claims none are bots can be hiding them:
/// one that already reports bots is caught by `bots`, and an empty one has
/// nobody playing. All answering servers supply rules for the flag, but only
/// these need the bot-plugin hint.
fn may_hide_bots_from_server(info: &ServerInfo) -> bool {
    info.players > 0 && info.bots == 0
}

/// Turn the raw query results into a `ScannedServer`.
fn finish_scan(
    endpoint: Endpoint,
    info_res: anyhow::Result<(crate::protocol::a2s::ServerInfoReply, u32)>,
    players_res: anyhow::Result<crate::protocol::a2s::ServerPlayersReply>,
    elapsed: u32,
) -> ScannedServer {
    let (info, rtt) = match info_res {
        Ok(v) => v,
        Err(e) => {
            let outcome = if e
                .to_string()
                .to_ascii_lowercase()
                .contains("bad info header")
                || e.to_string().contains("0x6e")
            {
                Outcome::BadHeader
            } else {
                Outcome::from_err(&e)
            };
            tracing::debug!(endpoint = %endpoint, ?outcome, elapsed_ms = elapsed, error = %e, "query failed");
            return ScannedServer {
                info: None,
                analysis: Analysis { reasons: vec![] },
                endpoint,
                query_ms: elapsed,
                outcome,
            };
        }
    };
    let players = players_res.ok().map(|r| r.players).unwrap_or_default();

    let last_seen = Utc::now();
    let server_info = ServerInfo {
        endpoint,
        protocol: 48,
        hostname: info.hostname,
        map: info.map,
        gamedir: info.gamedir.clone(),
        game: Game::CS16,
        app_id: Game::CS16.app_id() as u16,
        game_desc: info.gamedesc,
        players: info.players,
        max_players: info.max_players,
        bots: info.bots,
        server_type: info.server_type,
        os: info.os,
        password: info.password,
        vac: info.vac,
        version: info.version,
        // The round-trip time, not `elapsed`: the whole query spans a
        // challenge exchange and (on demand) the player list, so it would
        // overstate the latency the user actually gets.
        ping_ms: Some(rtt),
        country: None,
        city: None,
        players_list: players,
        ping_history: Vec::new(),
        bot_plugin: None,
        response_time_ms: elapsed,
        last_seen,
    };

    let analysis = analyze(&server_info, &DetCtx::default());
    if analysis.is_fake() {
        tracing::debug!(
            endpoint = %endpoint,
            reasons = ?analysis.reasons,
            "query answered; flagged as fake"
        );
    }

    ScannedServer {
        info: Some(server_info),
        analysis,
        endpoint,
        query_ms: elapsed,
        outcome: Outcome::Ok,
    }
}

/// Query a single server's A2S_INFO.
///
/// `include_players` additionally issues A2S_PLAYER. It is **off by default**:
/// it doubles the number of packets and sockets per server, and the player list
/// is not shown in the list view — so paying for it on a 20k sweep is pure
/// waste. The TUI enables it on demand for the selected server instead.
pub async fn scan_one(endpoint: Endpoint, timeout_ms: u64) -> ScannedServer {
    scan_one_with(endpoint, timeout_ms, false).await
}

/// As [`scan_one`], but lets the caller request the player list.
pub async fn scan_one_with(
    endpoint: Endpoint,
    timeout_ms: u64,
    include_players: bool,
) -> ScannedServer {
    match tokio::net::UdpSocket::bind("0.0.0.0:0").await {
        Ok(sock) => scan_one_on(&sock, endpoint, timeout_ms, include_players).await,
        Err(e) => {
            let start = Instant::now();
            finish_scan(
                endpoint,
                Err(e.into()),
                Err(anyhow::anyhow!("no socket")),
                start.elapsed().as_millis() as u32,
            )
        }
    }
}

/// A fixed set of UDP sockets, handed out to concurrent queries.
///
/// Binding one socket per query costs a syscall pair and an ephemeral port per
/// server, which dominates the per-server cost when sweeping tens of thousands
/// of endpoints. A small fixed pool removes that entirely: each in-flight query
/// borrows a socket, and answers are matched by source address (see
/// `a2s::recv_from_peer`), so sharing a socket across servers is safe.
struct SocketPool {
    idle: parking_lot::Mutex<Vec<Arc<tokio::net::UdpSocket>>>,
    cv: tokio::sync::Notify,
}

impl SocketPool {
    fn new(size: usize) -> Arc<Self> {
        let mut idle = Vec::with_capacity(size);
        for _ in 0..size.max(1) {
            // A bind failure here is fatal for scanning; surface it on first use
            // rather than panicking inside a worker.
            if let Ok(sock) = std::net::UdpSocket::bind("0.0.0.0:0") {
                let _ = sock.set_nonblocking(true);
                if let Ok(sock) = tokio::net::UdpSocket::from_std(sock) {
                    idle.push(Arc::new(sock));
                }
            }
        }
        Arc::new(Self {
            idle: parking_lot::Mutex::new(idle),
            cv: tokio::sync::Notify::new(),
        })
    }

    async fn acquire(self: &Arc<Self>) -> Option<Arc<tokio::net::UdpSocket>> {
        loop {
            if let Some(sock) = self.idle.lock().pop() {
                return Some(sock);
            }
            // Waiter registration happens before the re-check below via
            // `notify_one`'s permit semantics; a spurious wake just re-loops.
            self.cv.notified().await;
        }
    }

    fn release(&self, sock: Arc<tokio::net::UdpSocket>) {
        self.idle.lock().push(sock);
        self.cv.notify_one();
    }
}

/// Concurrent batch queryer.
pub struct Scanner {
    pub concurrency: usize,
    pub timeout_ms: u64,
    pub progress: Option<Arc<dyn Fn(usize, usize) + Send + Sync>>,
    /// Per-endpoint A2S pacing. When set, an endpoint inside its interval is
    /// reported as [`Outcome::RateLimited`] **without any packet being sent**,
    /// so anti-flood protection on the server is not tripped.
    pub gate: Option<Arc<parking_lot::Mutex<crate::ratelimit::QueryGate>>>,
    /// Hard-banned endpoints. When set, a banned endpoint is reported as
    /// [`Outcome::Banned`] and **never queried**, so a sweep is not spent on
    /// servers already known to be fake.
    pub bans: Option<Arc<parking_lot::Mutex<crate::banlist::BanList>>>,
    /// Cooperative cancel flag. When set and flipped, [`scan_with`] stops
    /// handing out new work and drops the in-flight futures, aborting their
    /// queries. Results already delivered stay valid.
    pub cancel: Option<Arc<std::sync::atomic::AtomicBool>>,
}

impl Scanner {
    pub fn new(concurrency: usize, timeout_ms: u64) -> Self {
        Self {
            concurrency,
            timeout_ms,
            progress: None,
            gate: None,
            bans: None,
            cancel: None,
        }
    }

    /// Attach a cancel flag so a long sweep can be stopped by the UI.
    pub fn with_cancel(mut self, cancel: Arc<std::sync::atomic::AtomicBool>) -> Self {
        self.cancel = Some(cancel);
        self
    }

    /// Attach a hard-ban list; banned endpoints are skipped without a query.
    pub fn with_bans(mut self, bans: Arc<parking_lot::Mutex<crate::banlist::BanList>>) -> Self {
        self.bans = Some(bans);
        self
    }

    /// Attach per-endpoint pacing.
    pub fn with_gate(mut self, gate: Arc<parking_lot::Mutex<crate::ratelimit::QueryGate>>) -> Self {
        self.gate = Some(gate);
        self
    }

    pub fn with_progress(mut self, f: Arc<dyn Fn(usize, usize) + Send + Sync>) -> Self {
        self.progress = Some(f);
        self
    }

    /// Scan all endpoints, invoking `on_result` for each one as it finishes.
    ///
    /// Results are delivered through a **callback**, not a channel. An earlier
    /// version pushed into a bounded `mpsc` channel and returned only once every
    /// endpoint had been scanned; callers that drained the channel *after*
    /// awaiting `scan` deadlocked as soon as `capacity` results accumulated
    /// (observed: `[scan] 256/10000` then a permanent hang). A callback cannot
    /// block the producer, so that failure mode is gone by construction.
    ///
    /// `on_result` runs on the scanning task; keep it cheap (push into a `Vec`,
    /// update a counter). It must not block.
    pub async fn scan_with<F>(&self, endpoints: Vec<Endpoint>, mut on_result: F) -> usize
    where
        F: FnMut(ScannedServer),
    {
        let total = endpoints.len();
        if total == 0 {
            return 0;
        }
        let tm = self.timeout_ms;
        let concurrency = self.concurrency.max(1);
        let gate = self.gate.clone();
        let bans = self.bans.clone();

        // Sockets are pooled rather than bound per query: one bind per worker
        // for the whole sweep instead of one per server.
        let pool = SocketPool::new(concurrency);

        let stream = endpoints.into_iter().map(|ep| {
            let pool = Arc::clone(&pool);
            let gate = gate.clone();
            let bans = bans.clone();
            async move {
                // A hard ban short-circuits everything: no packet is sent, so a
                // sweep costs nothing for servers already known to be fake.
                // Cleared only by `--hard-refresh`.
                if let Some(bans) = &bans {
                    if bans.lock().contains(&ep.to_string()) {
                        tracing::debug!(endpoint = %ep, "banned; not queried");
                        return (
                            ep,
                            ScannedServer {
                                info: None,
                                analysis: Analysis { reasons: vec![] },
                                endpoint: ep,
                                query_ms: 0,
                                outcome: Outcome::Banned,
                            },
                        );
                    }
                }

                // Pacing next: if this endpoint is inside its anti-spam
                // interval, send nothing at all and report it as skipped. The
                // caller keeps whatever it already knew about the server.
                if let Some(gate) = &gate {
                    let verdict = gate.lock().try_acquire(&ep.to_string(), now_ms());
                    if !verdict.is_allowed() {
                        tracing::debug!(
                            endpoint = %ep,
                            reason = verdict.label(),
                            retry_after_ms = verdict.retry_after_ms(),
                            "rate-limited; not queried"
                        );
                        // Unmeasured: no info, and deliberately no `reasons`
                        // entry, so the row is not mistaken for a fake server.
                        // The caller re-analyses whatever info it already had.
                        return (
                            ep,
                            ScannedServer {
                                info: None,
                                analysis: Analysis { reasons: vec![] },
                                endpoint: ep,
                                query_ms: 0,
                                outcome: Outcome::RateLimited,
                            },
                        );
                    }
                }

                // A missing socket (bind failure at startup) degrades to the
                // unpooled path rather than dropping the server.
                match pool.acquire().await {
                    Some(sock) => {
                        let res = scan_one_on(&sock, ep, tm, false).await;
                        pool.release(sock);
                        (ep, res)
                    }
                    None => (ep, scan_one_with(ep, tm, false).await),
                }
            }
        });

        let mut futures = FuturesUnordered::new();
        let mut iter = stream;
        for _ in 0..concurrency {
            if let Some(f) = iter.next() {
                futures.push(f);
            }
        }
        let mut done = 0usize;
        loop {
            if self
                .cancel
                .as_ref()
                .is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed))
            {
                break;
            }
            let next = tokio::select! {
                next = futures.next() => next,
                _ = tokio::time::sleep(std::time::Duration::from_millis(50)) => continue,
            };
            let Some((_ep, res)) = next else { break };
            done += 1;
            if let Some(f) = &self.progress {
                f(done, total);
            }
            on_result(res);
            // Checked after delivery so a result that already arrived is never
            // thrown away; the remaining futures are dropped, which cancels
            // their in-flight queries.
            if let Some(c) = &self.cancel {
                if c.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
            }
            if let Some(f) = iter.next() {
                futures.push(f);
            }
        }
        done
    }

    /// Scan all endpoints, returning them once complete.
    ///
    /// Prefer this over any channel-based API: it cannot deadlock, and it
    /// preserves completion order.
    pub async fn scan_collect(&self, endpoints: Vec<Endpoint>) -> Vec<ScannedServer> {
        let mut out = Vec::with_capacity(endpoints.len());
        self.scan_with(endpoints, |s| out.push(s)).await;
        out
    }
}

/// Cross-server and cross-run context for fake detection.
///
/// A redirect farm answers A2S convincingly, so no single reply gives it
/// away; what does is the listing as a whole (hundreds of ports on one IP,
/// the same name cloned across endpoints) and the endpoint's own history
/// (name churn). Built **once per sweep from the whole listing, before any
/// row is shown**, so every verdict is final when the row reaches the UI —
/// a row is never displayed as clean and flipped to fake afterwards.
pub struct Verifier {
    farms: HashSet<Ipv4Addr>,
    names: HashMap<String, usize>,
    /// Name history, updated by every measured row; persisted by the caller.
    pub history: NameHistory,
}

impl Verifier {
    /// `endpoints` is everything the sweep knows about (the per-IP counts);
    /// `listing` supplies the advertised names.
    pub fn new(endpoints: &[Endpoint], listing: &[ScannedServer], history: NameHistory) -> Self {
        // Farm verdicts from the listing's names; endpoints Steam gave no
        // name for (the HL1 fallback) can still be a farm by volume.
        let named: HashSet<Endpoint> = listing.iter().map(|s| s.endpoint).collect();
        let unnamed: HashSet<Endpoint> = endpoints
            .iter()
            .filter(|e| !named.contains(e))
            .copied()
            .collect();
        let farms = crate::detect::farm_ips(
            listing
                .iter()
                .filter_map(|s| {
                    s.info.as_ref().map(|i| crate::detect::ListedEndpoint {
                        ip: s.endpoint.ip,
                        hostname: &i.hostname,
                    })
                })
                .chain(unnamed.iter().map(|e| crate::detect::ListedEndpoint {
                    ip: e.ip,
                    hostname: "",
                })),
        );
        let mut names: HashMap<String, usize> = HashMap::new();
        for s in listing {
            if let Some(info) = &s.info {
                let k = info.hostname.trim();
                if !k.is_empty() {
                    *names.entry(k.to_string()).or_default() += 1;
                }
            }
        }
        Self {
            farms,
            names,
            history,
        }
    }

    /// Re-analyse `s` with the full context.
    ///
    /// Only a row this sweep actually measured is recorded into history: a
    /// row that came from Steam's list but never answered A2S has an
    /// unverified name, and recording it would manufacture churn out of
    /// sampling noise.
    pub fn verify(&mut self, s: &mut ScannedServer) {
        let Some(info) = &s.info else { return };
        let key = s.endpoint.to_string();
        let measured = s.outcome == Outcome::Ok && info.ping_ms.is_some();
        let churn = if measured {
            self.history
                .observe(&key, &info.hostname, crate::history::now_unix())
        } else {
            self.history.churn_for(&key)
        };
        let ctx = DetCtx {
            hostname_repeat_count: self.names.get(info.hostname.trim()).copied().unwrap_or(0),
            on_farm_ip: self.farms.contains(&s.endpoint.ip),
            name_major_changes: churn.major_changes,
            name_variants: self.history.names_for(&key).len(),
        };
        s.analysis = analyze(info, &ctx);
    }
}

/// Test shim: verify a batch as one sweep (its own listing and endpoints).
#[cfg(test)]
pub(crate) fn annotate_duplicates(scans: &mut [ScannedServer]) {
    let mut history = NameHistory::default();
    annotate_history(scans, &mut history);
}

/// Test shim: verify a batch as one sweep, carrying `history` across calls.
/// Only measured rows are re-analysed, as in a real sweep.
#[cfg(test)]
pub(crate) fn annotate_history(scans: &mut [ScannedServer], history: &mut NameHistory) {
    let eps: Vec<Endpoint> = scans.iter().map(|s| s.endpoint).collect();
    let mut v = Verifier::new(&eps, scans, std::mem::take(history));
    for s in scans.iter_mut() {
        if s.info.as_ref().is_some_and(|i| i.ping_ms.is_some()) {
            v.verify(s);
        }
    }
    *history = v.history;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Os, ServerType, Vac};
    use std::net::Ipv4Addr;

    fn sample_info(name: &str) -> crate::model::ServerInfo {
        crate::model::ServerInfo {
            endpoint: Endpoint::new(Ipv4Addr::new(1, 1, 1, 1), 27015),
            protocol: 48,
            hostname: name.into(),
            map: "de_dust2".into(),
            gamedir: "cstrike".into(),
            game: Game::CS16,
            app_id: 10,
            game_desc: "Counter-Strike".into(),
            players: 10,
            max_players: 32,
            bots: 0,
            server_type: ServerType::Dedicated,
            os: Os::Linux,
            password: false,
            vac: Vac::Secured,
            version: "1.1.2.6".into(),
            ping_ms: Some(50),
            country: None,
            city: None,
            players_list: vec![],
            ping_history: vec![],
            bot_plugin: None,
            response_time_ms: 50,
            last_seen: Utc::now(),
        }
    }

    fn sample_scan(name: &str) -> ScannedServer {
        ScannedServer {
            info: Some(sample_info(name)),
            analysis: Analysis { reasons: vec![] },
            endpoint: Endpoint::new(Ipv4Addr::new(1, 1, 1, 1), 27015),
            query_ms: 50,
            outcome: Outcome::Ok,
        }
    }

    #[test]
    fn counts_duplicate_hostnames() {
        let mut scans = vec![];
        for _ in 0..6 {
            scans.push(sample_scan("BOT"));
        }
        scans.push(sample_scan("unique"));
        annotate_duplicates(&mut scans);
        let bot = scans
            .iter()
            .find(|s| s.info.as_ref().unwrap().hostname == "BOT")
            .unwrap();
        assert!(bot
            .analysis
            .reasons
            .contains(&crate::detect::Reason::RepeatedHostname));
        let uniq = scans
            .iter()
            .find(|s| s.info.as_ref().unwrap().hostname == "unique")
            .unwrap();
        assert!(!uniq
            .analysis
            .reasons
            .contains(&crate::detect::Reason::RepeatedHostname));
    }
}
#[cfg(test)]
mod regression_tests {
    use super::*;
    use crate::model::Endpoint;
    use std::time::Duration;

    /// Regression: the scan must complete for a list far larger than any
    /// internal buffer.
    ///
    /// The previous implementation fed a bounded `mpsc` channel (capacity 256)
    /// and returned only after every endpoint was scanned; callers that drained
    /// the channel afterwards hung forever the moment 256 results accumulated —
    /// observed in production as `[scan] 256/10000` followed by a dead UI.
    ///
    /// All endpoints here are unroutable and time out fast, so the test is
    /// deterministic and offline.
    #[tokio::test]
    async fn scan_completes_for_large_list() {
        let endpoints: Vec<Endpoint> = (0..600)
            .map(|i| {
                // TEST-NET-1 (RFC 5737) — guaranteed unroutable, so every query
                // times out rather than touching the network meaningfully.
                Endpoint::parse(&format!("192.0.2.{}:27015", (i % 254) + 1)).unwrap()
            })
            .collect();

        let scanner = Scanner::new(64, 50); // 50ms timeout per query
        let results = tokio::time::timeout(
            Duration::from_secs(30),
            scanner.scan_collect(endpoints.clone()),
        )
        .await
        .expect("scan must not hang (bounded-channel regression)");

        assert_eq!(
            results.len(),
            endpoints.len(),
            "every endpoint must be reported"
        );
    }

    /// The callback form must observe results incrementally, not only at the end.
    #[tokio::test]
    async fn scan_streams_results_as_they_finish() {
        let endpoints: Vec<Endpoint> = (0..100)
            .map(|i| Endpoint::parse(&format!("192.0.2.{}:27015", (i % 254) + 1)).unwrap())
            .collect();

        let mut seen = 0usize;
        let mut increments = 0usize;
        let scanner = Scanner::new(32, 50);
        scanner
            .scan_with(endpoints, |_s| {
                seen += 1;
                increments += 1;
            })
            .await;

        assert_eq!(seen, 100);
        assert_eq!(increments, 100, "callback must fire once per result");
    }

    #[tokio::test]
    async fn cancel_stops_while_a_server_is_silent() {
        let silent = std::net::UdpSocket::bind("127.0.0.1:0").unwrap();
        let endpoint = Endpoint::parse(&silent.local_addr().unwrap().to_string()).unwrap();
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let scanner = Scanner::new(1, 5_000).with_cancel(Arc::clone(&cancel));
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            cancel.store(true, std::sync::atomic::Ordering::Relaxed);
        });

        let mut seen = 0;
        let done = tokio::time::timeout(
            Duration::from_secs(1),
            scanner.scan_with(vec![endpoint], |_| seen += 1),
        )
        .await
        .expect("stop must not wait for the query timeout");
        assert_eq!(done, 0);
        assert_eq!(seen, 0);
    }
}

#[cfg(test)]
mod history_annotation_tests {
    use super::*;
    use crate::history::NameHistory;
    use crate::model::{Os, ServerType, Vac};
    use chrono::Utc;

    fn measured(ip: &str, name: &str, ping: Option<u32>) -> ScannedServer {
        let endpoint = Endpoint::parse(&format!("{ip}:27015")).unwrap();
        ScannedServer {
            info: Some(ServerInfo {
                endpoint,
                protocol: 48,
                hostname: name.into(),
                map: "de_dust2".into(),
                gamedir: "cstrike".into(),
                game: Game::CS16,
                app_id: 10,
                game_desc: "Counter-Strike".into(),
                players: 5,
                max_players: 32,
                bots: 0,
                server_type: ServerType::Dedicated,
                os: Os::Linux,
                password: false,
                vac: Vac::Secured,
                version: "1.1.2.7".into(),
                ping_ms: ping,
                country: None,
                city: None,
                players_list: Vec::new(),
                ping_history: Vec::new(),
                bot_plugin: None,
                response_time_ms: ping.unwrap_or(0),
                last_seen: Utc::now(),
            }),
            analysis: Analysis {
                reasons: Vec::new(),
            },
            endpoint,
            query_ms: ping.unwrap_or(0),
            outcome: Outcome::Ok,
        }
    }

    /// A server changing its whole name every run eventually gets banned.
    #[test]
    fn churner_becomes_banned_after_the_threshold() {
        let mut history = NameHistory::default();
        let names = [
            "Dust2 Paradise Free VIP",
            "Zombie Survival Apocalypse",
            "Headshot Practice Arena",
            "FastDL Deathmatch City",
        ];
        let mut last = None;
        for name in names {
            let mut scans = vec![measured("203.0.113.1", name, Some(10))];
            annotate_history(&mut scans, &mut history);
            last = Some(scans[0].analysis.clone());
        }
        let a = last.unwrap();
        assert!(
            a.reasons.contains(&crate::detect::Reason::NameChurn),
            "repeated whole-name changes must be flagged: {:?}",
            a.reasons
        );
        assert!(a.is_fake(), "name churn at the threshold must be hard");
    }

    /// The timer case from the request: a server that only advances a clock in
    /// its name must never be flagged, however many runs it goes through.
    #[test]
    fn clock_only_changes_are_never_flagged() {
        let mut history = NameHistory::default();
        let mut last = None;
        for minute in 0..12 {
            let mut scans = vec![measured(
                "203.0.113.2",
                &format!("GameServerName [20:{minute:02}]"),
                Some(10),
            )];
            annotate_history(&mut scans, &mut history);
            last = Some(scans[0].analysis.clone());
        }
        let a = last.unwrap();
        assert!(
            !a.reasons.contains(&crate::detect::Reason::NameChurn),
            "a clock suffix must never count as a rename: {:?}",
            a.reasons
        );
        assert!(!a.is_fake());
    }

    /// Unmeasured rows carry Steam's listing rather than the server's own name;
    /// recording them would manufacture churn from sampling noise.
    #[test]
    fn unmeasured_rows_are_not_recorded() {
        let mut history = NameHistory::default();
        let mut scans = vec![
            measured("203.0.113.3", "Never Answered", None),
            measured("203.0.113.4", "Answered Fine", Some(15)),
        ];
        annotate_history(&mut scans, &mut history);

        assert!(
            history.entries.contains_key("203.0.113.4:27015"),
            "a measured row must be recorded"
        );
        assert!(
            !history.entries.contains_key("203.0.113.3:27015"),
            "an unmeasured row must not be recorded"
        );
    }

    /// A single rename is ordinary administration, not churn.
    #[test]
    fn one_rename_is_not_churn() {
        let mut history = NameHistory::default();
        annotate_history(
            &mut [measured("203.0.113.5", "Original Name Here", Some(10))],
            &mut history,
        );
        let mut scans = vec![measured(
            "203.0.113.5",
            "Rebranded Completely New",
            Some(10),
        )];
        annotate_history(&mut scans, &mut history);
        assert!(
            !scans[0]
                .analysis
                .reasons
                .contains(&crate::detect::Reason::NameChurn),
            "a single rename must not be a hard ban"
        );
    }
}

#[cfg(test)]
mod pacing_tests {
    use super::*;
    use crate::ratelimit::{QueryGate, MAX_PER_WINDOW, MIN_INTERVAL_MS};

    fn gate() -> Arc<parking_lot::Mutex<QueryGate>> {
        Arc::new(parking_lot::Mutex::new(QueryGate::new()))
    }

    /// A second scan of the same endpoints inside the anti-spam interval must
    /// send nothing and report `RateLimited` — not a timeout, which would make
    /// healthy servers look dead.
    #[tokio::test]
    async fn second_scan_of_same_endpoints_is_paced_not_timed_out() {
        let eps: Vec<Endpoint> = (1..=5)
            .map(|i| Endpoint::parse(&format!("192.0.2.{i}:27015")).unwrap())
            .collect();
        let g = gate();

        // First pass queries (and records) every endpoint.
        let scanner = Scanner::new(16, 40).with_gate(Arc::clone(&g));
        let first = scanner.scan_collect(eps.clone()).await;
        assert_eq!(first.len(), 5);
        assert!(
            first.iter().all(|s| s.outcome != Outcome::RateLimited),
            "the first pass must actually query everything"
        );

        // Immediate second pass: everything is inside the 5s gap.
        let scanner = Scanner::new(16, 40).with_gate(Arc::clone(&g));
        let second = scanner.scan_collect(eps.clone()).await;
        assert_eq!(second.len(), 5, "paced endpoints are still reported");
        assert!(
            second.iter().all(|s| s.outcome == Outcome::RateLimited),
            "every endpoint must be paced: {:?}",
            second.iter().map(|s| &s.outcome).collect::<Vec<_>>()
        );
        // Nothing was measured, so nothing may be claimed about the server.
        assert!(second.iter().all(|s| s.info.is_none()));
        assert!(
            second.iter().all(|s| s.analysis.reasons.is_empty()),
            "pacing must not manufacture fake-server reasons"
        );
    }

    /// Without a gate, scanning behaves exactly as before.
    #[tokio::test]
    async fn no_gate_means_no_pacing() {
        let eps: Vec<Endpoint> = (1..=3)
            .map(|i| Endpoint::parse(&format!("192.0.2.{i}:27015")).unwrap())
            .collect();
        let scanner = Scanner::new(16, 40);
        let out = scanner.scan_collect(eps).await;
        assert_eq!(out.len(), 3);
        assert!(out.iter().all(|s| s.outcome != Outcome::RateLimited));
    }

    /// Exhausting the rolling window must also stop queries, even spaced past
    /// the minimum gap.
    #[tokio::test]
    async fn window_cap_stops_repeated_refreshes() {
        let ep = Endpoint::parse("192.0.2.77:27015").unwrap();
        let g = gate();
        let scanner = Scanner::new(1, 30).with_gate(Arc::clone(&g));

        let mut allowed = 0usize;
        // Five back-to-back single-endpoint scans, simulating rapid `R` presses.
        for _ in 0..5 {
            let out = scanner.scan_collect(vec![ep]).await;
            if out[0].outcome != Outcome::RateLimited {
                allowed += 1;
            }
        }
        assert_eq!(
            allowed, 1,
            "back-to-back refreshes must collapse to one query (5s gap)"
        );

        // Spacing them past the gap but inside the minute still caps at
        // MAX_PER_WINDOW in total.
        let now = crate::ratelimit::now_ms();
        let mut extra = 0usize;
        for k in 1..=4u64 {
            let t = now + k * MIN_INTERVAL_MS;
            if g.lock().check(&ep.to_string(), t).is_allowed() {
                g.lock().try_acquire(&ep.to_string(), t);
                extra += 1;
            }
        }
        assert_eq!(
            extra,
            MAX_PER_WINDOW - 1,
            "within the same minute only {} extra queries may pass",
            MAX_PER_WINDOW - 1
        );
    }
}

#[cfg(test)]
mod ban_tests {
    use super::*;
    use crate::banlist::BanList;

    fn bans() -> Arc<parking_lot::Mutex<BanList>> {
        Arc::new(parking_lot::Mutex::new(BanList::new()))
    }

    fn eps(n: usize) -> Vec<Endpoint> {
        (1..=n)
            .map(|i| Endpoint::parse(&format!("192.0.2.{i}:27015")).unwrap())
            .collect()
    }

    /// A banned endpoint must be skipped without a query, so a sweep is not
    /// spent re-probing servers already known to be fake.
    #[tokio::test]
    async fn banned_endpoints_are_never_queried() {
        let list = bans();
        list.lock()
            .ban("192.0.2.1:27015", "Server", vec!["slots>32".into()], 1);
        list.lock()
            .ban("192.0.2.2:27015", "Server", vec!["slots>32".into()], 1);

        let scanner = Scanner::new(16, 40).with_bans(Arc::clone(&list));
        let out = scanner.scan_collect(eps(5)).await;

        assert_eq!(out.len(), 5, "banned rows are still reported");
        let banned: Vec<_> = out
            .iter()
            .filter(|s| s.outcome == Outcome::Banned)
            .map(|s| s.endpoint.to_string())
            .collect();
        assert_eq!(banned.len(), 2, "both banned endpoints must be skipped");
        assert!(
            out.iter()
                .filter(|s| s.outcome == Outcome::Banned)
                .all(|s| s.info.is_none() && s.analysis.reasons.is_empty()),
            "a skipped endpoint carries no measurement and no new reasons"
        );
    }

    /// Banning short-circuits pacing: a banned endpoint is skipped even if it
    /// would otherwise be queryable.
    #[tokio::test]
    async fn ban_takes_priority_over_pacing() {
        let list = bans();
        list.lock().ban("192.0.2.1:27015", "Server", vec![], 1);
        let gate = Arc::new(parking_lot::Mutex::new(crate::ratelimit::QueryGate::new()));

        let scanner = Scanner::new(4, 30)
            .with_bans(Arc::clone(&list))
            .with_gate(gate);
        let out = scanner.scan_collect(eps(1)).await;
        assert_eq!(out[0].outcome, Outcome::Banned, "ban wins over pacing");
    }

    /// Without a ban list, nothing is skipped.
    #[tokio::test]
    async fn no_ban_list_means_no_skips() {
        let scanner = Scanner::new(8, 30);
        let out = scanner.scan_collect(eps(3)).await;
        assert!(out.iter().all(|s| s.outcome != Outcome::Banned));
    }

    /// Clearing the list restores querying, which is what `X` in the TUI does.
    #[tokio::test]
    async fn clearing_bans_restores_querying() {
        let list = bans();
        list.lock().ban("192.0.2.1:27015", "Server", vec![], 1);

        let scanner = Scanner::new(4, 30).with_bans(Arc::clone(&list));
        assert_eq!(
            scanner.scan_collect(eps(1)).await[0].outcome,
            Outcome::Banned
        );

        list.lock().clear();
        let scanner = Scanner::new(4, 30).with_bans(Arc::clone(&list));
        assert_ne!(
            scanner.scan_collect(eps(1)).await[0].outcome,
            Outcome::Banned,
            "after a hard refresh the endpoint must be queried again"
        );
    }
}

#[cfg(test)]
mod churn_escalation_tests {
    use super::*;
    use crate::banlist::{RecheckQueue, Scheduled};
    use crate::history::NameHistory;
    use crate::model::{Os, ServerType, Vac};
    use chrono::Utc;

    fn measured(ip: &str, name: &str) -> ScannedServer {
        let endpoint = Endpoint::parse(&format!("{ip}:27015")).unwrap();
        ScannedServer {
            info: Some(ServerInfo {
                endpoint,
                protocol: 48,
                hostname: name.into(),
                map: "de_dust2".into(),
                gamedir: "cstrike".into(),
                game: Game::CS16,
                app_id: 10,
                game_desc: "Counter-Strike".into(),
                players: 3,
                max_players: 32,
                bots: 0,
                server_type: ServerType::Dedicated,
                os: Os::Linux,
                password: false,
                vac: Vac::Secured,
                version: "1.1.2.7".into(),
                ping_ms: Some(20),
                country: None,
                city: None,
                players_list: Vec::new(),
                ping_history: Vec::new(),
                bot_plugin: None,
                response_time_ms: 20,
                last_seen: Utc::now(),
            }),
            analysis: crate::detect::Analysis {
                reasons: Vec::new(),
            },
            endpoint,
            query_ms: 20,
            outcome: Outcome::Ok,
        }
    }

    /// Regression: soft-detected servers were never rechecked, so a
    /// `duplicate-name` server that keeps rebranding could never accumulate
    /// name-churn and become a hard ban.
    ///
    /// This walks the full path a suspect takes across scans: scheduled for
    /// recheck → re-queried → name recorded → churn counted → banned.
    #[test]
    fn suspected_server_escalates_to_hard_ban_via_rechecks() {
        let mut history = NameHistory::default();
        let mut queue = RecheckQueue::new();
        let ep = "203.0.113.9:27015";
        let mut now = 10_000i64;

        // Scan 1: suspicious (duplicate-name) but not banned -> queued.
        let mut scans = vec![measured("203.0.113.9", "Alpha Identity")];
        annotate_history(&mut scans, &mut history);
        assert!(
            !scans[0].analysis.is_fake(),
            "the first sighting must not be a hard ban"
        );
        assert_eq!(queue.schedule(ep, now), Scheduled::Added);

        // Each following scan re-queries the suspect with a *new* identity,
        // exactly as the recheck pass does.
        let identities = [
            "Zombie Survival Apocalypse",
            "Headshot Practice Arena",
            "FastDL Deathmatch City",
            "Awp Sniper Heaven",
        ];
        let mut banned_on = None;
        for (i, name) in identities.iter().enumerate() {
            // The recheck fires only once the queue says it is due. Repeat
            // rechecks are spaced, so advance the clock past the interval.
            now += crate::banlist::RECHECK_INTERVAL_SECS;
            assert_eq!(
                queue.due(now, 10),
                vec![ep.to_string()],
                "round {i}: suspect must be due for recheck"
            );

            let mut s = vec![measured("203.0.113.9", name)];
            annotate_history(&mut s, &mut history);
            queue.done(ep);

            if s[0].analysis.is_fake() {
                banned_on = Some(i);
                break;
            }
            // Still not banned: it stays eligible for another recheck.
            queue.schedule(ep, now);
        }

        let round = banned_on
            .expect("a server changing its whole name repeatedly must eventually be hard-banned");
        assert!(
            round >= 2,
            "escalation must take several renames, not one (banned on round {round})"
        );
        assert!(
            history.churn_for(ep).major_changes >= crate::history::NAME_CHURN_HARD_AT,
            "churn count must have reached the threshold"
        );
    }

    /// The clock-suffix case must NOT escalate, however many rechecks happen.
    #[test]
    fn clock_only_renames_never_escalate() {
        let mut history = NameHistory::default();
        let mut scans = vec![measured("203.0.113.10", "GameServerName [20:00]")];
        annotate_history(&mut scans, &mut history);

        for minute in 1..15 {
            let mut s = vec![measured(
                "203.0.113.10",
                &format!("GameServerName [20:{minute:02}]"),
            )];
            annotate_history(&mut s, &mut history);
            assert!(
                !s[0].analysis.is_fake(),
                "minute {minute}: a timer in the name must never ban a server"
            );
        }
        assert_eq!(history.churn_for("203.0.113.10:27015").major_changes, 0);
    }
}
