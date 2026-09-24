//! The application pipeline.
//!
//! Configuration, the discovery cache, the scan pipeline, the persisted state
//! file (history + pacing + bans + rechecks), and the flat DTO that crosses
//! the IPC boundary all live here; `gui` is only a Tauri translation layer
//! over it, so a rule fixed here is fixed everywhere.
//!
//! The pipeline is **callback-driven**, never channel-driven. `Scanner::scan_with`
//! documents why: a bounded channel deadlocked callers that drained after
//! awaiting the scan. `scan_streaming` preserves that property — the sink is a
//! plain `FnMut`, and the GUI's implementation of it is a non-blocking
//! `AppHandle::emit`.

use crate::banlist::{BanList, RecheckQueue, Scheduled};
use crate::client::webapi::{self, ApiServer, WebApi};
use crate::detect::Context as DetCtx;
use crate::model::{Endpoint, Game, Os, ServerInfo, ServerType, Vac};
use crate::protocol::masterserver::{
    build_filter, build_webapi_filter, query_all_flat_with, DEFAULT_MASTERS,
};
use crate::ratelimit::QueryGate;
use crate::scanner::{Outcome, ScannedServer, Scanner, Verifier};
use crate::store::{store_path, ServerStore};
use anyhow::Result;
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Request budget for discovery when the caller asks for everything
/// (`fetch_all`): enough to exclude [`webapi::MAX_EXCLUDED_IPS`] farms in
/// batches and still resample.
pub const FETCH_ALL_ROUNDS: u32 = 16;

/// A server that answered this recently is not queried again by a sweep: its
/// row is reused as-is. "Refresh all" pressed repeatedly then costs the
/// servers nothing, instead of re-querying thousands of them every time the
/// 5 s per-endpoint gap has passed. A single-server refresh still queries.
pub const FRESH_FOR_MS: u64 = 60_000;

/// Shown instead of a list when no Steam Web API key is saved.
pub const NO_API_KEY_MESSAGE: &str = "No Steam Web API key: open Settings (the gear in the \
     title bar) and paste one. Servers are listed once a key is saved.";

/// How many due rechecks to run in one pass. Small on purpose: a recheck is a
/// courtesy to a maybe-innocent server, not a mass re-sweep.
pub const RECHECK_BATCH: usize = 64;

/// Name of the combined local state file (history + query pacing).
pub const STATE_FILE: &str = "cs16browser-state.json";
pub const STATE_PATH_ENV: &str = "CS16BROWSER_STATE";

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Discovery filters — the values that change *what Steam is asked for*.
///
/// These are the four the in-game filter dialog sends on the wire (see
/// `protocol::masterserver::build_filter`). Changing any of them invalidates
/// the cached server list, because the cache is keyed on the filter string.
///
/// `rename_all = "camelCase"` matters: `ScanConfig` is camelCase on the wire,
/// and without this the nested filter object would demand `no_full` from a
/// frontend that sends `noFull` — a mismatch that surfaces at runtime as
/// `missing field`, not as a compile error.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub struct Filters {
    pub secure: bool,
    pub no_full: bool,
    pub no_empty: bool,
    pub no_password: bool,
}

/// Everything the pipeline needs to run. Built from the frontend's
/// `ScanConfig` by the GUI.
#[derive(Debug, Clone)]
pub struct Config {
    pub game: Game,
    pub api_key: Option<String>,
    /// Ignore the discovery cache and re-fetch from Steam.
    pub rediscover: bool,
    /// Most Web API requests one discovery may make (see `fetch_steam_list`).
    pub fetch_rounds: u32,
    /// Use the larger [`FETCH_ALL_ROUNDS`] budget.
    pub fetch_all: bool,
    pub concurrency: usize,
    pub query_timeout_ms: u64,
    pub master_timeout_ms: u64,
    pub filters: Filters,
    /// Client-side substring filter on hostname / map / endpoint (owned by the
    /// UI; carried here only so `start_scan` can preserve it across sweeps).
    pub text_filter: Option<String>,
    pub hide_fakes: bool,
    pub show_fakes: bool,
    pub fake_only: bool,
    /// Disables the whole state file (history, pacing, bans).
    pub no_history: bool,
    pub max_servers: Option<usize>,
    pub verbose: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            game: Game::CS16,
            api_key: None,
            rediscover: false,
            // Measured: secure + not empty + not full completes in 2
            // requests, secure alone (the first-run default) in 8 — with all
            // 250 exclusions used. 12 leaves room for the list to grow.
            fetch_rounds: 12,
            fetch_all: false,
            // Measured, not guessed: latency is only meaningful while the
            // sweep leaves the uplink uncongested. At 256 in flight every
            // reply queued behind the burst and the median "ping" read
            // 150–240 ms against a true ~30 ms, so the UI's default
            // `latency < 100` filter hid the entire list. 48 keeps pings
            // within a few ms of an idle link and still sweeps ~500
            // servers/s; beyond ~64 throughput stops rising, only the
            // measured latency does.
            concurrency: 48,
            query_timeout_ms: 900,
            master_timeout_ms: 4000,
            filters: Filters::default(),
            text_filter: None,
            hide_fakes: true,
            show_fakes: false,
            fake_only: false,
            no_history: false,
            max_servers: None,
            verbose: false,
        }
    }
}

impl Config {
    /// The filter string for **Steam's Web API** (the default source).
    ///
    /// Uses [`build_webapi_filter`], whose key set differs from the legacy
    /// master protocol — sending the master grammar here returns an empty list
    /// rather than an error.
    pub fn filter_string(&self) -> String {
        build_webapi_filter(
            self.game,
            self.filters.secure,
            self.filters.no_full,
            self.filters.no_empty,
            self.filters.no_password,
        )
    }

    /// The filter string for the legacy **HL1 master servers** (fallback path).
    pub fn master_filter_string(&self) -> String {
        build_filter(
            self.game,
            self.filters.secure,
            self.filters.no_full,
            self.filters.no_empty,
            self.filters.no_password,
        )
    }

    /// The built-in master list for the HL1 fallback.
    fn master_list(&self) -> Vec<(String, u16)> {
        DEFAULT_MASTERS
            .iter()
            .map(|(h, p)| (h.to_string(), *p))
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Persisted state
// ---------------------------------------------------------------------------

/// Everything persisted between runs.
///
/// Name history and query pacing share one file because both are keyed by
/// endpoint and both are per-machine state; a single atomic write keeps them
/// consistent and avoids two file handles.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub history: crate::history::NameHistory,
    #[serde(default)]
    pub gate: QueryGate,
    /// Endpoints hard-banned by a previous run; never re-queried.
    #[serde(default)]
    pub bans: BanList,
    /// Endpoints explicitly trusted by the user, even when a heuristic fires.
    #[serde(default)]
    pub whitelist: HashSet<String>,
    /// Suspected (not banned) endpoints awaiting a recheck.
    #[serde(default)]
    pub rechecks: RecheckQueue,
}

impl State {
    /// Path of the state file, honouring `no_history` and `CS16BROWSER_STATE`.
    ///
    /// Resolved against an existing state file in the current directory or
    /// beside the executable (see [`crate::store::base_dirs`]), so a GUI
    /// launched with an arbitrary working directory still finds the bans,
    /// history and pacing a previous run wrote.
    pub fn path(cfg: &Config) -> Option<PathBuf> {
        if cfg.no_history {
            return None;
        }
        if let Ok(p) = std::env::var(STATE_PATH_ENV) {
            let p = p.trim();
            if !p.is_empty() {
                return Some(PathBuf::from(p));
            }
        }
        let bases = crate::store::base_dirs();
        bases
            .iter()
            .map(|b| b.join(STATE_FILE))
            .find(|p| p.is_file())
            .or_else(|| bases.into_iter().next().map(|b| b.join(STATE_FILE)))
    }

    /// Tolerant load: a missing or corrupt file yields empty state rather than
    /// an error, so a bad write can never brick the browser.
    pub fn load(path: &Path) -> Self {
        match std::fs::read_to_string(path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_default(),
            Err(_) => Self::default(),
        }
    }

    /// Write atomically; a failure is reported, never fatal.
    pub fn save(&self, path: &Path) -> Option<String> {
        let json = match serde_json::to_string(self) {
            Ok(j) => j,
            Err(e) => return Some(format!("serialize: {e}")),
        };
        let tmp = path.with_extension("json.tmp");
        if let Err(e) = std::fs::write(&tmp, json) {
            return Some(format!("write {}: {e}", tmp.display()));
        }
        if let Err(e) = std::fs::rename(&tmp, path) {
            let _ = std::fs::remove_file(&tmp);
            return Some(format!("rename to {}: {e}", path.display()));
        }
        None
    }
}

// ---------------------------------------------------------------------------
// IPC DTOs
// ---------------------------------------------------------------------------

/// One player, as shown in the detail pane.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct PlayerRow {
    pub index: u8,
    pub name: String,
    pub score: i32,
    pub duration_seconds: f32,
}

/// A flat, serialisable projection of [`ScannedServer`].
///
/// `ScannedServer` / `Analysis` / `Outcome` deliberately do not derive
/// `Serialize` — they are detection internals. This is the single projection
/// they cross the wire through.
#[derive(Debug, Clone, Default, Serialize, Deserialize, specta::Type)]
pub struct ServerRow {
    /// `"1.2.3.4:27015"`.
    pub endpoint: String,
    pub hostname: String,
    pub map: String,
    pub gamedir: String,
    /// The server's own game description (the in-game Game column):
    /// `"Counter-Strike"` unless a mod reports something else.
    pub game: String,
    pub players: u8,
    pub max_players: u8,
    pub bots: u8,
    /// The bot plugin the server's rules name (`"YaPB 4.4.957"`), which can
    /// hide its bots from `bots`. `None` when none was found or not asked.
    pub bot_plugin: Option<String>,
    /// `None` when the server was listed by Steam but never answered A2S.
    pub ping_ms: Option<u32>,
    pub secure: bool,
    pub password: bool,
    /// `0` windows, `1` linux, `2` mac — the in-game browser's icon column.
    pub os: u8,
    /// Answered A2S on this run, so every field is measured rather than listed.
    pub live: bool,
    /// `"ok" | "listed" | "timeout" | "paced" | "banned" | "bad-header" |
    /// "refused" | "err"`.
    pub outcome: String,
    pub banned: bool,
    /// Any hard reason — the row is a definite fake.
    pub fake: bool,
    /// Fully verified: the server answered A2S itself (this sweep, or earlier
    /// this session when it was paced and nothing was sent) **and** passed
    /// every check, including the listing-wide ones (redirect farms, cloned
    /// names) and name history. The default view shows only these rows.
    pub verified: bool,
    pub reasons: Vec<String>,
    pub soft_reasons: Vec<String>,
    pub country: Option<String>,
    pub version: String,
    pub players_list: Vec<PlayerRow>,
}

/// `Outcome` as the wire string.
pub fn outcome_str(o: &Outcome) -> &'static str {
    match o {
        Outcome::Ok => "ok",
        Outcome::RateLimited => "paced",
        Outcome::Banned => "banned",
        Outcome::Timeout => "timeout",
        Outcome::BadHeader => "bad-header",
        Outcome::ConnectRefused => "refused",
        Outcome::UnknownError(_) => "err",
    }
}

/// A ban-list row for the `B` overlay.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type)]
pub struct BanRow {
    pub endpoint: String,
    /// The hostname as last seen before the ban; empty if never measured.
    pub hostname: String,
    /// `%Y-%m-%d %H:%M`.
    pub banned_at: String,
    pub reasons: Vec<String>,
    pub rechecks: u32,
}

/// Counters for the status line.
///
/// `u32`, not `usize`: this type crosses into the webview, where a JSON number
/// is an f64 and Specta refuses the pointer-width integers outright. A server
/// count cannot approach 2^32.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, specta::Type)]
pub struct ScanSummary {
    pub shown: u32,
    pub banned: u32,
    pub hidden: u32,
    pub paced: u32,
    pub listed: u32,
    pub measured: u32,
    pub rechecks: u32,
    pub total_before: u32,
}

/// Progress phase.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    #[default]
    Idle,
    Fetching,
    Scanning,
    Done,
}

/// Everything the UI is told during a scan.
///
/// One enum behind one event name: the frontend needs exactly one listener.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[tauri_specta(event_name = "scan")]
pub enum Event {
    Phase {
        phase: Phase,
    },
    /// Bulk rows — from the cache, or one fetch round. Sent before scanning so
    /// the list is browsable immediately.
    Listed {
        rows: Vec<ServerRow>,
    },
    /// One scan result. `done`/`total` count the sweep, not the list.
    Row {
        done: u32,
        total: u32,
        row: ServerRow,
    },
    /// A suspect was re-queried and replaces its earlier row.
    Refreshed {
        row: ServerRow,
    },
    /// A refresh was refused by the per-endpoint pacing gate.
    Paced {
        endpoint: String,
        reason: String,
    },
    Summary {
        summary: ScanSummary,
    },
    Error {
        message: String,
    },
}

// ---------------------------------------------------------------------------
// Conversions
// ---------------------------------------------------------------------------

/// Project a scanned server into the wire form.
pub fn row_from_scanned(s: &ScannedServer) -> ServerRow {
    let (mut reasons, mut soft) = (Vec::new(), Vec::new());
    for r in &s.analysis.reasons {
        if r.severity().is_hard() {
            reasons.push(r.label().to_string());
        } else {
            soft.push(r.label().to_string());
        }
    }
    let banned = s.outcome == Outcome::Banned;
    let measured = s.info.as_ref().is_some_and(|i| i.ping_ms.is_some());
    let live = s.outcome == Outcome::Ok && measured;
    let fake = s.analysis.is_fake();
    // A paced row only carries a latency when it adopted this session's
    // earlier measurement (see `adopt_prior`), and with it that verdict.
    let verified = measured && !fake && matches!(s.outcome, Outcome::Ok | Outcome::RateLimited);
    // Steam's listing arrives as `Outcome::Ok` with nothing measured; say so.
    let outcome = if s.outcome == Outcome::Ok && !measured {
        "listed"
    } else {
        outcome_str(&s.outcome)
    };

    match &s.info {
        Some(i) => ServerRow {
            endpoint: s.endpoint.to_string(),
            hostname: i.hostname.clone(),
            map: i.map.clone(),
            gamedir: i.gamedir.clone(),
            game: if i.game_desc.trim().is_empty() {
                i.game.description().to_string()
            } else {
                i.game_desc.clone()
            },
            players: i.players,
            max_players: i.max_players,
            bots: i.bots,
            bot_plugin: i.bot_plugin.clone(),
            ping_ms: i.ping_ms,
            secure: i.vac == Vac::Secured,
            password: i.password,
            os: os_code(i.os),
            live,
            outcome: outcome.to_string(),
            banned,
            fake,
            verified,
            reasons,
            soft_reasons: soft,
            country: i
                .country
                .clone()
                .or_else(|| crate::country::from_ip(s.endpoint.ip).map(str::to_string)),
            version: i.version.clone(),
            players_list: i
                .players_list
                .iter()
                .map(|p| PlayerRow {
                    index: p.index,
                    name: p.name.clone(),
                    score: p.score,
                    duration_seconds: p.duration_seconds,
                })
                .collect(),
        },
        None => ServerRow {
            endpoint: s.endpoint.to_string(),
            country: crate::country::from_ip(s.endpoint.ip).map(str::to_string),
            outcome: outcome.to_string(),
            banned,
            fake,
            reasons,
            soft_reasons: soft,
            ..Default::default()
        },
    }
}

fn os_code(o: Os) -> u8 {
    match o {
        Os::Windows => 0,
        Os::Linux => 1,
        Os::Mac => 2,
    }
}

/// Convert one Web API row into a `ScannedServer` prefilled with Steam's data.
pub fn row_from_api(s: &ApiServer, game: Game) -> Option<ScannedServer> {
    let endpoint = s.endpoint()?;

    let clamp_u8 = |v: Option<i32>| v.unwrap_or(0).clamp(0, u8::MAX as i32) as u8;

    let info = ServerInfo {
        endpoint,
        // GoldSrc A2S_INFO protocol; replaced by the live scan when it answers.
        protocol: 48,
        hostname: s.name.clone().unwrap_or_default(),
        map: s.map.clone().unwrap_or_default(),
        gamedir: s
            .gamedir
            .clone()
            .unwrap_or_else(|| game.gamedir().to_string()),
        game,
        app_id: s.appid.unwrap_or_else(|| game.app_id()) as u16,
        // Not in the Web API; replaced by the server's own when it answers.
        game_desc: game.description().to_string(),
        players: clamp_u8(s.players),
        max_players: clamp_u8(s.max_players),
        bots: clamp_u8(s.bots),
        server_type: if s.dedicated.unwrap_or(true) {
            ServerType::Dedicated
        } else {
            ServerType::Listen
        },
        os: match s.os.as_deref() {
            Some("l") => Os::Linux,
            Some("m") | Some("o") => Os::Mac,
            _ => Os::Windows,
        },
        // Not exposed by the Web API.
        password: false,
        vac: if s.secure.unwrap_or(false) {
            Vac::Secured
        } else {
            Vac::Unsecured
        },
        version: s.version.clone().unwrap_or_default(),
        // Requires a live query.
        ping_ms: None,
        country: crate::country::from_ip(endpoint.ip).map(str::to_string),
        city: None,
        players_list: Vec::new(),
        ping_history: Vec::new(),
        bot_plugin: None,
        response_time_ms: 0,
        last_seen: chrono::Utc::now(),
    };

    // The same heuristics the scanner applies, so a faked row in Steam's own
    // list is flagged even when the server never answers A2S.
    let analysis = crate::detect::analyze(&info, &DetCtx::default());

    Some(ScannedServer {
        info: Some(info),
        analysis,
        endpoint,
        query_ms: 0,
        outcome: Outcome::Ok,
    })
}

/// Merge a scan result with the listed row for the same endpoint.
///
/// The scanner sends **nothing** for a paced or banned endpoint, and its
/// documented contract is that "the caller keeps whatever it already knew about
/// the server". A live result with no measurement therefore adopts the listed
/// metadata while keeping its own outcome, so the row stays presentable instead
/// of blanking out in the UI.
pub fn with_listed_fallback(
    mut scan: ScannedServer,
    listed: Option<&ScannedServer>,
) -> ScannedServer {
    if let Some(listed) = listed {
        if scan.info.is_none() {
            adopt_prior(&mut scan, listed.clone());
        }
    }
    scan
}

/// Give an unmeasured result the metadata of an earlier row, keeping its own
/// outcome so it is not presented as freshly probed.
///
/// The latency is kept only when **nothing was sent** (paced / banned): the
/// earlier measurement is then still the best thing we know. A server that
/// was queried and did not answer has no latency, and carrying an old one
/// forward would present a dead server as reachable.
fn adopt_prior(scan: &mut ScannedServer, prior: ScannedServer) {
    let outcome = scan.outcome.clone();
    let query_ms = scan.query_ms;
    *scan = prior;
    if !matches!(outcome, Outcome::RateLimited | Outcome::Banned) {
        if let Some(info) = scan.info.as_mut() {
            info.ping_ms = None;
        }
    }
    scan.outcome = outcome;
    scan.query_ms = query_ms;
}

/// Merge live A2S results with listed rows, live data winning per endpoint.
pub fn merge_scans(live: Vec<ScannedServer>, listed: Vec<ScannedServer>) -> Vec<ScannedServer> {
    let mut by_endpoint: HashMap<Endpoint, ScannedServer> =
        live.into_iter().map(|s| (s.endpoint, s)).collect();

    for row in listed {
        match by_endpoint.get_mut(&row.endpoint) {
            Some(existing) => {
                if existing.info.is_none() {
                    // Did not answer. Adopt the listed metadata but remember the
                    // failure, so the row is not presented as freshly probed.
                    adopt_prior(existing, row);
                }
            }
            None => {
                by_endpoint.insert(row.endpoint, row);
            }
        }
    }

    by_endpoint.into_values().collect()
}

// ---------------------------------------------------------------------------
// App
// ---------------------------------------------------------------------------

/// The shared pipeline. Holds the config and the long-lived shared state
/// (pacing gate, ban list, persisted history/rechecks) for one browsing
/// session.
pub struct App {
    /// Interior-mutable: a UI can change the scan parameters between sweeps
    /// (a different filter, a different concurrency) and the *same* gate and ban
    /// list must keep being used. Two `App`s would mean two pacing gates, which
    /// is exactly the invariant the per-endpoint limits exist to protect — a
    /// single-server refresh would then be free to re-query an endpoint the
    /// sweep had just hit.
    cfg: RwLock<Config>,
    gate: Arc<parking_lot::Mutex<QueryGate>>,
    bans: Arc<parking_lot::Mutex<BanList>>,
    whitelist: parking_lot::Mutex<HashSet<String>>,
    cancel: Arc<AtomicBool>,
    /// The last row the UI was given per endpoint (in memory only — live
    /// measurements are never persisted). A paced endpoint is sent nothing,
    /// so this is what it falls back to instead of Steam's bare listing:
    /// without it, a Refresh inside the pacing window turned every measured
    /// row back into an unmeasured one and emptied the list.
    known: parking_lot::Mutex<HashMap<Endpoint, ScannedServer>>,
    /// Endpoints that keep timing out (see [`crate::ratelimit::TimeoutBackoff`]).
    backoff: parking_lot::Mutex<crate::ratelimit::TimeoutBackoff>,
    /// The discovery-cache write started by the last fetch, if it may still
    /// be running (see [`App::flush_store`]).
    store_save: parking_lot::Mutex<Option<tokio::task::JoinHandle<()>>>,
}

impl App {
    /// The Steam Web API key from Settings, if one is saved.
    pub fn api_key(&self) -> Option<String> {
        self.cfg
            .read()
            .api_key
            .as_deref()
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .map(str::to_string)
    }

    /// Build an app, seeding pacing and bans from the persisted state.
    pub fn new(cfg: Config) -> Self {
        let (gate, bans, whitelist) = match State::path(&cfg) {
            Some(path) => {
                let mut state = State::load(&path);
                let dropped = state.bans.retain(|_, e| {
                    e.reasons.iter().any(|r| r == "manual")
                        || e.reasons
                            .iter()
                            .any(|l| crate::detect::Reason::label_bans_persistently(l))
                });
                if dropped > 0 {
                    tracing::info!(
                        dropped,
                        "[bans] lifted ban(s) whose reasons no longer ban persistently"
                    );
                }
                (
                    Arc::new(parking_lot::Mutex::new(state.gate)),
                    Arc::new(parking_lot::Mutex::new(state.bans)),
                    parking_lot::Mutex::new(state.whitelist),
                )
            }
            None => (
                Arc::new(parking_lot::Mutex::new(QueryGate::new())),
                Arc::new(parking_lot::Mutex::new(BanList::new())),
                parking_lot::Mutex::new(HashSet::new()),
            ),
        };
        Self {
            cfg: RwLock::new(cfg),
            gate,
            bans,
            whitelist,
            cancel: Arc::new(AtomicBool::new(false)),
            known: parking_lot::Mutex::new(HashMap::new()),
            backoff: parking_lot::Mutex::new(crate::ratelimit::TimeoutBackoff::new()),
            store_save: parking_lot::Mutex::new(None),
        }
    }

    /// Snapshot of the current config. Cheap enough to call per operation.
    pub fn cfg(&self) -> Config {
        self.cfg.read().clone()
    }

    /// Replace the scan parameters, keeping the shared gate and ban list.
    ///
    /// Callers that only touch client-side view state (`hide_fakes`,
    /// `text_filter`) must not change the *discovery* filters, or the next
    /// sweep will re-fetch rather than serve the cache.
    pub fn set_cfg(&self, cfg: Config) {
        *self.cfg.write() = cfg;
    }

    /// Mutate the config in place, keeping shared state.
    pub fn update_cfg(&self, f: impl FnOnce(&mut Config)) {
        f(&mut self.cfg.write());
    }

    pub fn gate(&self) -> Arc<parking_lot::Mutex<QueryGate>> {
        Arc::clone(&self.gate)
    }

    pub fn bans(&self) -> Arc<parking_lot::Mutex<BanList>> {
        Arc::clone(&self.bans)
    }

    /// Persist a user ban and remove any previous trust decision.
    pub fn ban_server(&self, endpoint: &str, hostname: &str) -> Result<(), String> {
        Endpoint::parse(endpoint).ok_or_else(|| format!("bad endpoint: {endpoint}"))?;
        self.whitelist.lock().remove(endpoint);
        self.bans.lock().ban(
            endpoint,
            hostname,
            vec!["manual".into()],
            crate::history::now_unix(),
        );
        self.save_server_lists()
    }

    /// Trust one endpoint and remove its existing ban. Detection still runs,
    /// but this explicit choice wins when rows are projected and persisted.
    pub fn whitelist_server(&self, endpoint: &str) -> Result<(), String> {
        let ep = Endpoint::parse(endpoint).ok_or_else(|| format!("bad endpoint: {endpoint}"))?;
        self.whitelist.lock().insert(endpoint.to_string());
        self.bans.lock().unban(endpoint);
        if let Some(row) = self.known.lock().get_mut(&ep) {
            row.analysis.reasons.clear();
            if row.outcome == Outcome::Banned {
                row.outcome = Outcome::RateLimited;
            }
        }
        self.save_server_lists()
    }

    fn save_server_lists(&self) -> Result<(), String> {
        let Some(path) = State::path(&self.cfg()) else {
            return Ok(());
        };
        let mut state = State::load(&path);
        state.bans = self.bans.lock().clone();
        state.whitelist = self.whitelist.lock().clone();
        state.save(&path).map_or(Ok(()), Err)
    }

    fn apply_whitelist(&self, row: &mut ScannedServer) {
        if self.whitelist.lock().contains(&row.endpoint.to_string()) {
            row.analysis.reasons.clear();
        }
    }

    fn apply_manual_ban(&self, row: &mut ScannedServer) {
        if self
            .bans
            .lock()
            .get(&row.endpoint.to_string())
            .is_some_and(|entry| entry.reasons.iter().any(|reason| reason == "manual"))
        {
            row.outcome = Outcome::Banned;
        }
    }

    /// Cancel flag shared with every scanner this app builds.
    pub fn cancel_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancel)
    }

    /// Ask the in-flight sweep to stop. No-op when nothing is running.
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub(crate) fn reset_cancel(&self) {
        self.cancel.store(false, Ordering::Relaxed);
    }

    /// Banned endpoints as rows, newest first.
    pub fn ban_rows(&self) -> Vec<BanRow> {
        let mut rows: Vec<BanRow> = self
            .bans
            .lock()
            .endpoints()
            .map(|(ep, e)| BanRow {
                endpoint: ep.to_string(),
                hostname: e.hostname.clone(),
                banned_at: chrono::DateTime::from_timestamp(e.banned_at, 0)
                    .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_else(|| "?".into()),
                reasons: e.reasons.clone(),
                rechecks: e.rechecks,
            })
            .collect();
        rows.sort_by(|a, b| b.banned_at.cmp(&a.banned_at));
        rows
    }

    /// Clear the ban list only, leaving pacing and the recheck queue alone.
    ///
    /// Distinct from [`hard_refresh`](Self::hard_refresh), which also drops
    /// pacing: clearing bans has no reason to forget which endpoints were just
    /// queried, and doing so would let an unban immediately re-hammer servers.
    pub fn clear_bans(&self) -> usize {
        let cleared = self.bans.lock().clear();
        if let Some(path) = State::path(&self.cfg()) {
            let mut state = State::load(&path);
            state.bans.clear();
            if let Some(e) = state.save(&path) {
                tracing::warn!(path = %path.display(), error = %e, "[state] could not save");
            }
        }
        cleared
    }

    /// Clear bans, pending rechecks and pacing, so the next scan re-queries
    /// everything from scratch.
    ///
    /// History is deliberately preserved — a hard refresh re-queries servers,
    /// it does not forget who they are.
    pub fn hard_refresh(&self) -> String {
        {
            let mut b = self.bans.lock();
            b.clear();
        }
        // Pacing must be reset too, or "re-query everything" would still be
        // throttled by intervals recorded in the previous run.
        {
            let mut g = self.gate.lock();
            *g = QueryGate::new();
        }
        // "Re-query everything" includes servers we were backing off from.
        self.backoff.lock().clear();

        let Some(path) = State::path(&self.cfg()) else {
            return "hard refresh: bans and pacing cleared (history disabled)".into();
        };
        let mut state = State::load(&path);
        let cleared = state.bans.clear();
        let queued = state.rechecks.len();
        state.rechecks = RecheckQueue::new();
        state.gate = QueryGate::new();
        match state.save(&path) {
            None => {
                format!("hard refresh: cleared {cleared} ban(s), {queued} recheck(s), pacing reset")
            }
            Some(e) => format!("hard refresh failed: {e}"),
        }
    }

    /// Persist a finished sweep: the name history the [`Verifier`] updated,
    /// then bans/rechecks from the (already final) verdicts, then one atomic
    /// write.
    ///
    /// The ban/recheck policy lives here and only here, so a second
    /// implementation cannot drift from it.
    pub fn persist_verdicts(
        &self,
        servers: &[ScannedServer],
        history: crate::history::NameHistory,
    ) {
        let Some(path) = State::path(&self.cfg()) else {
            return;
        };
        let mut state = State::load(&path);
        let before = state.history.entries.len();
        state.history = history;

        // Replace the stored gate with ours (fresher), pruning before writing.
        {
            let mut g = self.gate.lock();
            g.prune(crate::ratelimit::now_ms());
            state.gate = g.clone();
        }
        // ...and adopt any bans recorded during the sweep.
        state.bans = self.bans.lock().clone();
        state.whitelist = self.whitelist.lock().clone();

        let now = crate::history::now_unix();
        let mut newly_banned = 0usize;
        let mut scheduled = 0usize;
        for s in servers.iter() {
            let ep = s.endpoint.to_string();
            if state.whitelist.contains(&ep) {
                state.bans.unban(&ep);
                state.rechecks.remove(&ep);
                continue;
            }

            // A hard ban is permanent and means "never query this again".
            // Farm verdicts are not stored: they are re-derived from the
            // listing every sweep (and skip the query just the same).
            if s.analysis.is_fake() {
                let reasons: Vec<String> = s
                    .analysis
                    .reasons
                    .iter()
                    .map(|r| r.label())
                    .filter(|l| crate::detect::Reason::label_bans_persistently(l))
                    .map(str::to_string)
                    .collect();
                let hostname = s.info.as_ref().map(|i| i.hostname.as_str()).unwrap_or("");
                if !reasons.is_empty() && state.bans.ban(&ep, hostname, reasons, now) {
                    newly_banned += 1;
                }
                // Definitely fake: a pending recheck is pointless.
                state.rechecks.remove(&ep);
                continue;
            }

            // Suspected but not banned: queue a recheck so it can prove itself.
            if !s.analysis.reasons.is_empty() {
                if state.rechecks.schedule(&ep, now) == Scheduled::Added {
                    scheduled += 1;
                }
            } else {
                // Clean: drop any stale queue entry.
                state.rechecks.remove(&ep);
            }
        }

        state.history.prune();
        state.rechecks.prune();

        // Keep the in-memory ban list in step with what we just wrote.
        {
            let mut b = self.bans.lock();
            *b = state.bans.clone();
        }

        if let Some(err) = state.save(&path) {
            tracing::warn!(path = %path.display(), error = %err, "[state] could not save");
        } else {
            tracing::debug!(
                tracked = state.history.entries.len(),
                before,
                paced = state.gate.tracked(),
                banned = state.bans.len(),
                newly_banned,
                queued_for_recheck = state.rechecks.len(),
                scheduled,
                path = %path.display(),
                "[state] verdicts persisted"
            );
        }
    }

    /// Wait for a discovery-cache write still in flight. A sweep does not wait
    /// for its own write before scanning, only before it finishes, and the
    /// next discovery waits before reading the file.
    pub async fn flush_store(&self) {
        let pending = self.store_save.lock().take();
        if let Some(handle) = pending {
            let _ = handle.await;
        }
    }

    /// Obtain the server list, preferring the local discovery cache.
    ///
    /// Steam is queried **once** and the rows are written to
    /// `cs16browser-data/servers.json`; every later run works from that file,
    /// so ordinary browsing does not spend metered API calls.
    ///
    /// `on_round` streams newly-arrived rows, so a caller can paint
    /// progressively in both the cached and the fetching case.
    pub async fn discover<F>(
        &self,
        filter: &str,
        mut on_round: F,
    ) -> Result<Option<Vec<ScannedServer>>>
    where
        F: FnMut(Vec<ScannedServer>),
    {
        self.flush_store().await;
        let path = store_path();
        let cfg = self.cfg();
        let gamedir = cfg.game.gamedir();

        let prior = ServerStore::load(&path);
        if !cfg.rediscover {
            if let Some(store) = &prior {
                if store.matches(filter, gamedir) {
                    let rows: Vec<ScannedServer> = store
                        .servers
                        .iter()
                        .filter_map(|s| row_from_api(s, cfg.game))
                        .collect();
                    tracing::info!(
                        rows = rows.len(),
                        path = %path.display(),
                        age = %store.age_label(),
                        "[store] serving cached servers"
                    );
                    on_round(rows.clone());
                    return Ok(Some(rows));
                }
                tracing::debug!(
                    "[store] cached list was built for a different filter; re-fetching"
                );
            }
        } else {
            tracing::info!("[store] rediscovery requested: fetching a fresh list from Steam");
        }

        // Farms condemned last time are excluded from the first request, so
        // Steam's capped window is not spent on them again (see
        // `store::KnownFarm`).
        let known = prior
            .as_ref()
            .map(|s| s.known_farms(crate::history::now_unix()))
            .unwrap_or_default();
        let seed: Vec<std::net::Ipv4Addr> = known
            .iter()
            .take(webapi::MAX_EXCLUDED_IPS)
            .map(|f| f.ip)
            .collect();
        drop(prior);

        let raw = Arc::new(parking_lot::Mutex::new(Vec::<ApiServer>::new()));
        let raw_sink = Arc::clone(&raw);
        let rows = self
            .fetch_steam_list(filter, &seed, move |converted, raw_rows| {
                raw_sink.lock().extend(raw_rows);
                on_round(converted);
            })
            .await?;

        // A stopped discovery is only a partial Steam listing. Keep the rows
        // already shown in this sweep, but never replace a complete cache.
        if self.cancel.load(Ordering::Relaxed) {
            return Ok(rows);
        }

        let raw = std::mem::take(&mut *raw.lock());
        // Only overwrite the cache when we actually got rows: a failed or empty
        // fetch must not destroy a good cached list.
        if !raw.is_empty() {
            let mut store = ServerStore::new(filter, gamedir, raw);
            // Off the scan's critical path: condemning farms over tens of
            // thousands of rows and writing them takes about a second, and
            // this sweep scans from the rows in memory.
            let handle = tokio::task::spawn_blocking(move || {
                store.carry_farms(&known, &seed);
                match store.save(&path) {
                    Ok(()) => tracing::info!(
                        servers = store.len(),
                        farms = store.farms.len(),
                        path = %path.display(),
                        "[store] saved discovery cache"
                    ),
                    Err(e) => {
                        tracing::warn!(path = %path.display(), error = %format!("{e:#}"), "[store] could not save")
                    }
                }
            });
            *self.store_save.lock() = Some(handle);
        }

        Ok(rows)
    }

    /// Fetch the server list from Steam's Web API.
    ///
    /// ## Why one request is not enough
    ///
    /// `GetServerList` returns at most [`webapi::MAX_LIMIT`] (10,000) rows and
    /// has no paging. Measured on appid 10: **every** request comes back
    /// exactly full, because redirect farms advertise tens of thousands of
    /// fake servers (a six-request sample held 51,000 rows on 333 farm IPs).
    /// A plain request is therefore a truncated slice in which real servers
    /// are crowded out: it held 135 of them, where the complete list for the
    /// same filter holds ~770.
    ///
    /// So each request after the first excludes the farm IPs found so far
    /// (`\nor\N\gameaddr\<ip>…`, largest farms first), which frees the
    /// window for servers not yet seen. `seed` (farms a previous discovery
    /// condemned, see `store::KnownFarm`) is excluded from the first request
    /// on: measured, that completes in 3 requests where starting bare took
    /// 13. A response under the cap is
    /// complete. Measured: with the default filters this completes in two
    /// requests. Only IPs [`crate::detect::farm_ips`] condemns are excluded,
    /// so a real host running many servers keeps all of them.
    ///
    /// `on_round` fires once per request with that request's *new* rows, so
    /// the first batch can be rendered immediately. Returns `Ok(None)` when no
    /// API key is configured, so the caller can fall back to the HL1 masters.
    pub async fn fetch_steam_list<F>(
        &self,
        filter: &str,
        seed: &[std::net::Ipv4Addr],
        mut on_round: F,
    ) -> Result<Option<Vec<ScannedServer>>>
    where
        F: FnMut(Vec<ScannedServer>, Vec<ApiServer>),
    {
        let Some(key) = self.api_key() else {
            return Ok(None);
        };

        let cfg = self.cfg();
        let started = std::time::Instant::now();
        let timeout = Duration::from_millis(cfg.master_timeout_ms.max(5_000));
        let rounds = if cfg.fetch_all {
            FETCH_ALL_ROUNDS
        } else {
            cfg.fetch_rounds.max(1)
        };
        let api = WebApi::new(key).with_timeout(timeout);
        let game = cfg.game;

        let mut all: Vec<ScannedServer> = Vec::new();
        let mut seen: HashSet<Endpoint> = HashSet::new();
        let mut excluded: Vec<std::net::Ipv4Addr> = seed
            .iter()
            .take(webapi::MAX_EXCLUDED_IPS)
            .copied()
            .collect();
        let mut complete = false;
        let mut unparseable = 0usize;
        let mut requests = 0u32;

        for _ in 0..rounds {
            if self.cancel.load(Ordering::Relaxed) {
                break;
            }
            let request_filter = webapi::with_excluded_ips(filter, &excluded);
            let batch = tokio::select! {
                result = api.fetch_server_list_async(&request_filter) => result?,
                _ = async {
                    while !self.cancel.load(Ordering::Relaxed) {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                } => break,
            };
            requests += 1;
            let returned = batch.len();

            let mut fresh_raw: Vec<ApiServer> = Vec::new();
            for row in batch {
                match row.endpoint() {
                    Some(ep) if seen.insert(ep) => fresh_raw.push(row),
                    Some(_) => {}
                    None => unparseable += 1,
                }
            }
            let fresh: Vec<ScannedServer> = fresh_raw
                .iter()
                .filter_map(|s| row_from_api(s, game))
                .collect();
            tracing::debug!(
                requests,
                returned,
                new = fresh.len(),
                excluded_ips = excluded.len(),
                "[steam] request completed"
            );
            let found_new = !fresh.is_empty();
            on_round(fresh.clone(), fresh_raw);
            all.extend(fresh);

            if self.cancel.load(Ordering::Relaxed) {
                break;
            }

            if returned < webapi::MAX_LIMIT as usize {
                complete = true;
                break;
            }

            // Still capped: exclude the farms found so far, biggest first.
            let farms = crate::detect::farm_ips(all.iter().filter_map(|s| {
                s.info.as_ref().map(|i| crate::detect::ListedEndpoint {
                    ip: s.endpoint.ip,
                    hostname: &i.hostname,
                })
            }));
            let mut size: HashMap<std::net::Ipv4Addr, usize> = HashMap::new();
            for s in &all {
                if farms.contains(&s.endpoint.ip) {
                    *size.entry(s.endpoint.ip).or_default() += 1;
                }
            }
            let mut new_farms: Vec<(std::net::Ipv4Addr, usize)> = size
                .into_iter()
                .filter(|(ip, _)| !excluded.contains(ip))
                .collect();
            new_farms.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
            let room = webapi::MAX_EXCLUDED_IPS.saturating_sub(excluded.len());
            if new_farms.is_empty() || room == 0 {
                // Nothing more can be excluded (none left, or the URL is at
                // its limit). The capped answer is a varying sample, so an
                // identical request still uncovers rows — worth repeating
                // while the last one found something new.
                if !found_new {
                    break;
                }
                continue;
            }
            excluded.extend(new_farms.into_iter().take(room).map(|(ip, _)| ip));
        }

        tracing::info!(
            unique_servers = seen.len(),
            requests,
            elapsed_s = started.elapsed().as_secs_f32(),
            farm_ips_excluded = excluded.len(),
            farm_ips_seeded = seed.len().min(webapi::MAX_EXCLUDED_IPS),
            complete,
            "[steam] discovery finished{}",
            if complete {
                ""
            } else {
                " — INCOMPLETE: Steam's 10,000-row cap was still reached; narrow the filters"
            }
        );
        if unparseable > 0 {
            tracing::warn!(
                unparseable,
                "[steam] row(s) had an unparseable addr and were dropped"
            );
        }

        Ok(Some(all))
    }

    /// HL1 master fallback, used only when Steam produced nothing.
    ///
    /// Deliberately does **not** call `discover`: discovery is metered, so it
    /// must happen exactly once per sweep (see `scan_streaming`).
    async fn hl1_fallback(&self, filter: &str) -> Vec<Endpoint> {
        let masters = self.cfg().master_list();
        let (hl1, errors) =
            query_all_flat_with(&masters, filter, self.cfg().master_timeout_ms).await;
        for (host, port, err) in &errors {
            let s = err.to_string();
            let short = if s.contains("Nieznany host")
                || s.contains("no such host")
                || s.contains("Name or service not known")
            {
                "DNS"
            } else if s.contains("timeout") || s.contains("deadline") {
                "TIMEOUT"
            } else if s.contains("refused") {
                "REFUSED"
            } else {
                "ERR"
            };
            tracing::debug!(host = %host, port = %port, kind = short, error = %s, "[masters] request failed");
        }
        hl1
    }

    /// Re-query suspects whose recheck interval has elapsed.
    ///
    /// This is what lets a soft finding (`duplicate-name`) escalate: a server
    /// that keeps changing its whole name accumulates churn counts and becomes
    /// a hard ban. Without it, churn could never fire in a long-lived session.
    ///
    /// `measured` holds the endpoints this sweep already got an answer from.
    /// Those are resolved from that answer rather than queried again: the
    /// sweep hit them seconds ago, so the pacing gate would refuse a second
    /// query anyway, and the recheck would stay queued forever.
    ///
    /// Each completed recheck is handed to `on_row`; a paced or failed one is
    /// not, so it stays queued and never blanks a row the UI is showing.
    async fn run_rechecks<F>(&self, measured: &HashSet<Endpoint>, mut on_row: F)
    where
        F: FnMut(ScannedServer),
    {
        let Some(path) = State::path(&self.cfg()) else {
            return;
        };
        let mut q = State::load(&path).rechecks;
        let due = q.due(crate::history::now_unix(), RECHECK_BATCH);
        if due.is_empty() {
            return;
        }

        let mut eps: Vec<Endpoint> = Vec::new();
        for key in &due {
            match Endpoint::parse(key) {
                Some(_) if self.bans.lock().contains(key) => q.remove(key),
                Some(ep) if measured.contains(&ep) => q.done(key),
                Some(ep) => eps.push(ep),
                None => {}
            }
        }

        if !eps.is_empty() {
            tracing::info!(
                count = eps.len(),
                "[recheck] re-querying suspected endpoint(s)"
            );
            let scanner = Scanner::new(self.cfg().concurrency, self.cfg().query_timeout_ms)
                .with_gate(self.gate())
                .with_cancel(self.cancel_flag());
            for r in scanner.scan_collect(eps).await {
                if r.outcome == Outcome::Ok {
                    q.done(&r.endpoint.to_string());
                    on_row(r);
                }
            }
        }

        let mut st = State::load(&path);
        st.rechecks = q;
        if let Some(e) = st.save(&path) {
            tracing::warn!(path = %path.display(), error = %e, "[state] could not save");
        }
    }

    /// Persist the pacing gate so a later run (or a single-server refresh)
    /// honours the limits this sweep just consumed.
    pub fn persist_gate(&self) {
        let Some(path) = State::path(&self.cfg()) else {
            return;
        };
        let mut state = State::load(&path);
        let mut g = self.gate.lock();
        g.prune(crate::ratelimit::now_ms());
        state.gate = g.clone();
        let _ = state.save(&path);
    }

    /// Full sweep, streaming every result through `on_event`.
    ///
    /// The sink runs on the scanning task, so it must be non-blocking — the
    /// GUI passes an `AppHandle::emit`, which is exactly that.
    ///
    /// Returns the merged view: live A2S results where a server answered, the
    /// Steam-listed row where it did not, with rechecked suspects overriding
    /// their stale rows. That is the same set the UI ends up displaying.
    pub async fn scan_streaming<F>(&self, on_event: &mut F) -> Result<Vec<ScannedServer>>
    where
        F: FnMut(Event),
    {
        {
            let cfg = self.cfg();
            tracing::debug!(
                game = ?cfg.game,
                rediscover = cfg.rediscover,
                concurrency = cfg.concurrency,
                query_timeout_ms = cfg.query_timeout_ms,
                filters = ?cfg.filters,
                max_servers = ?cfg.max_servers,
                "[scan] sweep starting"
            );
        }

        // No key, no discovery: not the cached list (built with a key the
        // user may since have cleared), not the legacy HL1 masters. The
        // Settings dialog is where the key goes; say so and stop.
        if self.api_key().is_none() {
            on_event(Event::Error {
                message: NO_API_KEY_MESSAGE.to_string(),
            });
            on_event(Event::Phase { phase: Phase::Done });
            return Ok(Vec::new());
        }

        let filter = self.cfg().filter_string();
        on_event(Event::Phase {
            phase: Phase::Fetching,
        });

        // Discovery streams rows straight to the UI, so the list fills in
        // progressively instead of appearing after the last round. It runs
        // exactly once per sweep — the Web API is metered.
        let mut listed = 0usize;
        let mut cached: Vec<ScannedServer> = Vec::new();
        let mut seen: HashSet<Endpoint> = HashSet::new();
        let mut endpoints: Vec<Endpoint> = Vec::new();
        {
            let res = self
                .discover(&filter, |rows| {
                    listed += rows.len();
                    for r in &rows {
                        if seen.insert(r.endpoint) {
                            cached.push(r.clone());
                        }
                    }
                    let projected: Vec<ServerRow> = rows.iter().map(row_from_scanned).collect();
                    on_event(Event::Listed { rows: projected });
                })
                .await;
            match res {
                Ok(Some(rows)) => endpoints = rows.iter().map(|r| r.endpoint).collect(),
                // No Web API key configured: fall through to the HL1 masters.
                Ok(None) => {}
                Err(e) => on_event(Event::Error {
                    message: format!("{e:#}"),
                }),
            }
        }

        if self.cancel.load(Ordering::Relaxed) {
            return Ok(cached);
        }

        if endpoints.is_empty() {
            tracing::info!("[masters] falling back to HL1 master servers (legacy protocol)");
            // The legacy masters speak the master grammar, not the Web API's.
            endpoints = self.hl1_fallback(&self.cfg().master_filter_string()).await;
        }

        // An empty result is the one failure a user cannot diagnose from the
        // UI. Say which stage produced it and what to do, rather than leaving
        // an empty list that looks like a broken scan.
        if endpoints.is_empty() {
            let message = "No servers found: Steam returned an empty list and no HL1 master \
                 responded. Try `Change filters` (an over-restrictive filter can \
                 legitimately match nothing), or check network access to \
                 api.steampowered.com."
                .to_string();
            tracing::warn!("[scan] {message}");
            on_event(Event::Error { message });
        }

        // `max_servers` after every source has contributed, so the budget is
        // spent on distinct endpoints.
        if let Some(max) = self.cfg().max_servers {
            endpoints.truncate(max);
        }
        // Keep only listed rows we are actually going to scan.
        if !cached.is_empty() {
            let keep: HashSet<Endpoint> = endpoints.iter().copied().collect();
            cached.retain(|r| keep.contains(&r.endpoint));
        }

        // Verification context from the whole listing, built before any row
        // is shown, so every verdict below is final when it reaches the UI.
        let history = State::path(&self.cfg())
            .map(|p| State::load(&p).history)
            .unwrap_or_default();
        let mut verifier = Verifier::new(&endpoints, &cached, history);

        // Listing-level verdicts first. A server that is already a definite
        // fake from what it advertises — a redirect-farm IP, more than 32
        // slots, a redirect name — is never queried: no reply could clear
        // it, and on the live list that is most of the ~16k rows, so the
        // sweep only spends queries (and pacing budget) on real candidates.
        let mut prescreened: Vec<ServerRow> = Vec::new();
        let mut skip: HashSet<Endpoint> = HashSet::new();
        for row in cached.iter_mut() {
            verifier.verify(row);
            self.apply_whitelist(row);
            self.apply_manual_ban(row);
            if row.analysis.is_fake() || row.outcome == Outcome::Banned {
                skip.insert(row.endpoint);
                prescreened.push(row_from_scanned(row));
            }
        }
        if !skip.is_empty() {
            endpoints.retain(|e| !skip.contains(e));
            tracing::info!(
                count = skip.len(),
                "[verify] listed server(s) are definite fakes; not queried"
            );
            on_event(Event::Listed { rows: prescreened });
        }

        on_event(Event::Phase {
            phase: Phase::Scanning,
        });
        tracing::info!(servers = endpoints.len(), "[scan] scanning");

        let scanner = Scanner::new(self.cfg().concurrency, self.cfg().query_timeout_ms)
            .with_gate(self.gate())
            .with_bans(self.bans())
            .with_cancel(self.cancel_flag());

        let total = endpoints.len();
        let mut done = 0usize;
        let mut paced = 0usize;
        let mut measured = 0usize;
        let mut live: Vec<ScannedServer> = Vec::with_capacity(total);

        // Listed rows by endpoint, so a scan result that carries no measurement
        // can still be shown with the metadata Steam gave us. `Scanner` states
        // the contract: a paced or banned endpoint sends nothing, and "the
        // caller keeps whatever it already knew about the server". Emitting the
        // bare result would blank the row in the UI.
        let listed_by_ep: HashMap<Endpoint, ScannedServer> =
            cached.iter().map(|r| (r.endpoint, r.clone())).collect();

        // Before any packet: reuse what answered within `FRESH_FOR_MS`, and
        // skip endpoints we are backing off from. Neither is a verdict — a
        // reused row keeps the verdict it earned when it answered, and a
        // backed-off one is reported as not sent (`paced`), never as dead.
        let now = crate::ratelimit::now_ms();
        let now_utc = chrono::Utc::now();
        let (mut reused, mut backed_off) = (0usize, 0usize);
        let mut to_query: Vec<Endpoint> = Vec::with_capacity(endpoints.len());
        for ep in endpoints {
            // A new manual ban must win even when this session has a fresh,
            // previously verified measurement to reuse, or a backoff to skip.
            if self.bans.lock().contains(&ep.to_string()) {
                to_query.push(ep);
                continue;
            }
            let fresh = self
                .known
                .lock()
                .get(&ep)
                .filter(|k| {
                    k.outcome == Outcome::Ok
                        && k.info.as_ref().is_some_and(|i| {
                            i.ping_ms.is_some()
                                && (now_utc - i.last_seen).num_milliseconds() < FRESH_FOR_MS as i64
                        })
                })
                .cloned();
            let skipped = match fresh {
                Some(row) => {
                    reused += 1;
                    Some(row)
                }
                None if self.backoff.lock().is_backing_off(&ep.to_string(), now) => {
                    backed_off += 1;
                    let prior = self.known.lock().get(&ep).cloned();
                    let unsent = ScannedServer {
                        info: None,
                        analysis: crate::detect::Analysis { reasons: vec![] },
                        endpoint: ep,
                        query_ms: 0,
                        outcome: Outcome::RateLimited,
                    };
                    Some(with_listed_fallback(
                        unsent,
                        prior.as_ref().or(listed_by_ep.get(&ep)),
                    ))
                }
                None => None,
            };
            match skipped {
                Some(row) => {
                    done += 1;
                    on_event(Event::Row {
                        done: done as u32,
                        total: total as u32,
                        row: row_from_scanned(&row),
                    });
                    live.push(row);
                }
                None => to_query.push(ep),
            }
        }
        if reused + backed_off > 0 {
            tracing::debug!(
                reused,
                backed_off,
                fresh_for_s = FRESH_FOR_MS / 1000,
                "[scan] not queried (reused or backing off after repeated timeouts)"
            );
        }
        let endpoints = to_query;

        scanner
            .scan_with(endpoints, |server| {
                if server.outcome == Outcome::RateLimited {
                    paced += 1;
                }
                if server.outcome == Outcome::Ok {
                    measured += 1;
                }
                // Only a query that was actually sent feeds the back-off.
                match server.outcome {
                    Outcome::Ok => {
                        self.backoff
                            .lock()
                            .record(&server.endpoint.to_string(), true, now)
                    }
                    Outcome::Timeout | Outcome::ConnectRefused => {
                        self.backoff
                            .lock()
                            .record(&server.endpoint.to_string(), false, now)
                    }
                    _ => {}
                }
                done += 1;

                // No measurement? Re-attach what we already knew — this
                // session's last row first (a paced server keeps its measured
                // latency), else Steam's listing — keeping the failure outcome
                // so the row is not presented as freshly probed.
                let mut emit = if server.info.is_some() {
                    // Answered: verify with the full context before the row
                    // is shown. The old `0x6D` reply shape carries no game
                    // version, so the listing's value stands in for it.
                    let mut server = server;
                    if let (Some(info), Some(listed)) = (
                        server.info.as_mut(),
                        listed_by_ep
                            .get(&server.endpoint)
                            .and_then(|l| l.info.as_ref()),
                    ) {
                        if info.version.is_empty() {
                            info.version = listed.version.clone();
                        }
                    }
                    verifier.verify(&mut server);
                    self.apply_whitelist(&mut server);
                    server
                } else {
                    let ep = server.endpoint;
                    let prior = self.known.lock().get(&ep).cloned();
                    with_listed_fallback(server, prior.as_ref().or(listed_by_ep.get(&ep)))
                };
                self.apply_manual_ban(&mut emit);

                on_event(Event::Row {
                    // The wire DTO is u32 for the webview's sake; the sweep's
                    // own counters stay usize.
                    done: done as u32,
                    total: total as u32,
                    row: row_from_scanned(&emit),
                });
                live.push(emit);
            })
            .await;

        if paced > 0 {
            tracing::info!(
                paced,
                max_per_window = crate::ratelimit::MAX_PER_WINDOW,
                window_s = crate::ratelimit::WINDOW_MS / 1000,
                min_gap_ms = crate::ratelimit::MIN_INTERVAL_MS,
                "[scan] endpoint(s) skipped by the per-server rate limit"
            );
        }

        // Recheck suspects; each result replaces its row in the UI.
        let answered: HashSet<Endpoint> = live
            .iter()
            .filter(|s| s.outcome == Outcome::Ok)
            .map(|s| s.endpoint)
            .collect();
        let mut rechecked: Vec<ScannedServer> = Vec::new();
        if !self.cancel.load(Ordering::Relaxed) {
            self.run_rechecks(&answered, |mut server| {
                verifier.verify(&mut server);
                self.apply_whitelist(&mut server);
                self.apply_manual_ban(&mut server);
                on_event(Event::Refreshed {
                    row: row_from_scanned(&server),
                });
                rechecked.push(server);
            })
            .await;
        }
        let rechecks = rechecked.len();

        self.persist_gate();
        self.flush_store().await;

        // Keep the live A2S row when a server answered; otherwise fall back to
        // the row Steam gave us, so unreachable-but-listed servers still
        // appear. A rechecked suspect overrides its earlier (stale) row.
        let mut merged = if cached.is_empty() {
            live
        } else {
            merge_scans(live, cached)
        };
        if !rechecked.is_empty() {
            merged = merge_scans(rechecked, merged);
        }

        // Bans and rechecks from the final verdicts, plus the history the
        // verifier recorded.
        self.persist_verdicts(&merged, verifier.history);

        *self.known.lock() = merged.iter().map(|s| (s.endpoint, s.clone())).collect();

        let banned = merged
            .iter()
            .filter(|s| s.analysis.is_fake() || s.outcome == Outcome::Banned)
            .count();
        on_event(Event::Summary {
            summary: ScanSummary {
                total_before: merged.len() as u32,
                shown: merged.len() as u32,
                banned: banned as u32,
                listed: listed as u32,
                measured: measured as u32,
                paced: paced as u32,
                rechecks: rechecks as u32,
                hidden: 0,
            },
        });
        on_event(Event::Phase { phase: Phase::Done });
        Ok(merged)
    }

    /// Rows currently in the discovery cache, projected for the UI. Lets the
    /// list paint before any network work happens.
    pub fn cached_rows(&self) -> Vec<ServerRow> {
        // Without a key nothing is listed, the cache included.
        if self.api_key().is_none() {
            return Vec::new();
        }
        let cfg = self.cfg();
        let filter = cfg.filter_string();
        let gamedir = cfg.game.gamedir();
        ServerStore::load(&store_path())
            .filter(|s| s.matches(&filter, gamedir))
            .map(|store| {
                store
                    .servers
                    .iter()
                    .filter_map(|s| row_from_api(s, cfg.game))
                    .map(|s| row_from_scanned(&s))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Query only one endpoint's player list, for Game Info.
    ///
    /// The sweep measured everything else in the row moments ago but never
    /// asks for players, so opening the dialog sends A2S_PLAYER alone rather
    /// than re-running the whole query. It goes through the same interactive
    /// gate as a single-server refresh; a refusal sends nothing.
    pub async fn players_one(&self, endpoint: &str) -> Result<Vec<PlayerRow>, String> {
        let ep = Endpoint::parse(endpoint).ok_or_else(|| format!("bad endpoint: {endpoint}"))?;
        if self.bans.lock().contains(endpoint) {
            return Err("server is banned".into());
        }
        {
            let verdict = self
                .gate
                .lock()
                .try_acquire_interactive(endpoint, crate::ratelimit::now_ms());
            if !verdict.is_allowed() {
                return Err(verdict.message());
            }
        }
        let reply = crate::protocol::a2s::query_players_udp(ep, self.cfg().query_timeout_ms)
            .await
            .map_err(|e| e.to_string())?;
        if let Some(info) = self.known.lock().get_mut(&ep).and_then(|s| s.info.as_mut()) {
            info.players_list = reply.players.clone();
        }
        Ok(reply
            .players
            .iter()
            .map(|p| PlayerRow {
                index: p.index,
                name: p.name.clone(),
                score: p.score,
                duration_seconds: p.duration_seconds,
            })
            .collect())
    }

    /// Re-query a single endpoint, respecting the shared pacing gate.
    ///
    /// Returns `Err(reason)` when the gate refuses — nothing is sent, so the
    /// caller must not present the server as dead.
    pub async fn refresh_one(
        &self,
        endpoint: &str,
        interactive: bool,
    ) -> Result<ScannedServer, String> {
        let ep = Endpoint::parse(endpoint).ok_or_else(|| format!("bad endpoint: {endpoint}"))?;
        if self.bans.lock().contains(endpoint) {
            return Err("server is banned".into());
        }
        {
            let now = crate::ratelimit::now_ms();
            let mut gate = self.gate.lock();
            // One server the user is looking at gets the gentler gate; a bulk
            // refresh of many rows keeps the sweep's limits.
            let verdict = if interactive {
                gate.try_acquire_interactive(endpoint, now)
            } else {
                gate.try_acquire(endpoint, now)
            };
            if !verdict.is_allowed() {
                return Err(verdict.message());
            }
        }
        // One server on demand, so the player list is worth its extra
        // exchange here (the sweep skips it); the info dialog shows it.
        let server = crate::scanner::scan_one_with(ep, self.cfg().query_timeout_ms, true).await;
        if server.outcome == Outcome::Ok {
            self.backoff
                .lock()
                .record(endpoint, true, crate::ratelimit::now_ms());
        }
        // A server that did not answer keeps its name and map rather than
        // turning into a blank row; `adopt_prior` drops the stale latency.
        let mut known = self.known.lock();
        let server = with_listed_fallback(server, known.get(&ep));
        known.insert(ep, server.clone());
        Ok(server)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cached_sweep_streams_listed_prescreened_and_timeout_rows() {
        // Exercise the complete orchestration without Steam or a public UDP
        // endpoint. A high-slot listing is rejected before any packet; the
        // other endpoint times out and retains its advertised metadata.
        let dir = std::env::temp_dir().join(format!("cs16browser-sweep-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::env::set_var("CS16BROWSER_DATA", &dir);
        let cfg = Config {
            api_key: Some("0123456789abcdef0123456789abcdef".into()),
            query_timeout_ms: 20,
            no_history: true,
            ..Config::default()
        };
        let rows: Vec<ApiServer> = serde_json::from_str(
            r#"[
              {"addr":"8.8.8.8:27015","name":"community","appid":10,"gamedir":"cstrike","map":"de_dust2","players":4,"max_players":16,"secure":true},
              {"addr":"9.9.9.9:27015","name":"impossible slots","appid":10,"gamedir":"cstrike","map":"de_dust2","players":4,"max_players":64,"secure":true}
            ]"#,
        ).unwrap();
        ServerStore::new(cfg.filter_string(), cfg.game.gamedir(), rows)
            .save(&store_path())
            .unwrap();
        let app = App::new(cfg);
        let mut events = Vec::new();
        let merged = app.scan_streaming(&mut |e| events.push(e)).await.unwrap();
        assert_eq!(merged.len(), 2);
        assert!(events
            .iter()
            .any(|e| matches!(e, Event::Listed { rows } if rows.len() == 2)));
        assert!(events.iter().any(|e| matches!(e, Event::Row { .. })));
        assert!(events.iter().any(|e| matches!(e, Event::Summary { .. })));
        assert!(matches!(
            events.last(),
            Some(Event::Phase { phase: Phase::Done })
        ));
        assert_eq!(app.cached_rows().len(), 2);

        // A forced rediscovery uses the same pipeline, streams the new batch,
        // and atomically replaces the cache only after a successful response.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        std::env::set_var("CS16BROWSER_API_BASE", base);
        let server = std::thread::spawn(move || {
            use std::io::{Read, Write};
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0u8; 4096];
            let _ = stream.read(&mut request);
            let body = r#"{"response":{"servers":[{"addr":"8.8.4.4:27015","name":"new server","appid":10,"gamedir":"cstrike","map":"cs_office","players":2,"max_players":16,"secure":true}]}}"#;
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
        });
        app.update_cfg(|cfg| cfg.rediscover = true);
        let mut batches = Vec::new();
        let fresh = app
            .discover(&app.cfg().filter_string(), |r| batches.push(r))
            .await
            .unwrap()
            .unwrap();
        server.join().unwrap();
        app.flush_store().await;
        assert_eq!(fresh.len(), 1);
        assert_eq!(batches.len(), 1);
        assert_eq!(app.cached_rows().len(), 1);

        let state_path = dir.join("state.json");
        std::env::set_var("CS16BROWSER_STATE", &state_path);
        app.update_cfg(|cfg| cfg.no_history = false);
        app.persist_verdicts(&merged, crate::history::NameHistory::default());
        let state = State::load(&state_path);
        assert_eq!(
            state.bans.len(),
            1,
            "the prescreened hard fake is persisted"
        );
        assert_eq!(app.ban_rows().len(), 1);
        app.persist_gate();
        assert_eq!(app.clear_bans(), 1);
        assert_eq!(State::load(&state_path).bans.len(), 0);
        assert!(app.hard_refresh().contains("pacing reset"));
        std::env::remove_var("CS16BROWSER_STATE");
        std::env::remove_var("CS16BROWSER_API_BASE");
        std::env::remove_var("CS16BROWSER_DATA");
        let _ = std::fs::remove_dir_all(dir);
    }

    fn cfg_no_state() -> Config {
        Config {
            no_history: true,
            ..Default::default()
        }
    }

    #[test]
    fn default_config_matches_documented_defaults() {
        let c = Config::default();
        assert_eq!(c.fetch_rounds, 12);
        assert_eq!(c.concurrency, 48);
        assert_eq!(c.query_timeout_ms, 900);
        assert_eq!(c.master_timeout_ms, 4000);
        assert!(c.hide_fakes, "fakes are hidden unless asked for");
        assert!(!c.show_fakes);
        assert!(!c.fake_only);
    }

    #[test]
    fn webapi_filter_omits_keys_the_api_rejects() {
        // Measured: `password\1` and `notempty\1` make the Web API return zero
        // rows, so the Web API filter must never contain them — that bug made
        // every filtered request in the GUI come back empty.
        let c = Config {
            filters: Filters {
                secure: true,
                no_full: true,
                no_empty: true,
                no_password: true,
            },
            ..Default::default()
        };
        let f = c.filter_string();
        assert!(f.contains("\\appid\\10"), "{f}");
        assert!(f.contains("\\gamedir\\cstrike"), "{f}");
        assert!(f.contains("\\secure\\1"), "{f}");
        assert!(f.contains("\\full\\1"), "{f}");
        assert!(f.contains("\\empty\\1"), "{f}");
        assert!(
            !f.contains("password"),
            "Web API rejects a password key: {f}"
        );
        assert!(!f.contains("notempty"), "{f}");
    }

    #[test]
    fn master_filter_keeps_the_legacy_grammar() {
        // The HL1 protocol really does accept `password`; only the Web API
        // doesn't. Both builders must keep their own correct key sets.
        let c = Config {
            filters: Filters {
                no_password: true,
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(c.master_filter_string().contains("\\password\\1"));
    }

    #[test]
    fn outcome_strings_are_stable() {
        // The CLI's --json contract; scripts parse these.
        assert_eq!(outcome_str(&Outcome::Ok), "ok");
        assert_eq!(outcome_str(&Outcome::RateLimited), "paced");
        assert_eq!(outcome_str(&Outcome::Banned), "banned");
        assert_eq!(outcome_str(&Outcome::Timeout), "timeout");
        assert_eq!(outcome_str(&Outcome::BadHeader), "bad-header");
        assert_eq!(outcome_str(&Outcome::ConnectRefused), "refused");
        assert_eq!(outcome_str(&Outcome::UnknownError("x".into())), "err");
    }

    #[test]
    fn paced_row_keeps_the_listed_metadata() {
        use crate::detect::Analysis;
        let ep = Endpoint::parse("8.8.8.8:27015").unwrap();

        // What the scanner emits for a paced endpoint: nothing measured.
        let paced = ScannedServer {
            info: None,
            analysis: Analysis { reasons: vec![] },
            endpoint: ep,
            query_ms: 0,
            outcome: Outcome::RateLimited,
        };
        // What Steam already told us about it.
        let listed = ScannedServer {
            info: Some(listed_info(ep, "Known Name")),
            analysis: Analysis { reasons: vec![] },
            endpoint: ep,
            query_ms: 0,
            outcome: Outcome::Ok,
        };

        let merged = with_listed_fallback(paced, Some(&listed));
        assert_eq!(
            merged.info.as_ref().map(|i| i.hostname.as_str()),
            Some("Known Name"),
            "a paced row must not blank out the metadata we already had"
        );
        // ...and it must still be reported as paced, not as a fresh success.
        assert_eq!(merged.outcome, Outcome::RateLimited);

        // The row the UI renders must therefore carry a name and a hostname.
        let row = row_from_scanned(&merged);
        assert_eq!(row.hostname, "Known Name");
        assert_eq!(row.outcome, "paced");
    }

    #[test]
    fn fallback_keeps_latency_only_when_nothing_was_sent() {
        use crate::detect::Analysis;
        let ep = Endpoint::parse("8.8.8.8:27015").unwrap();
        let mut info = listed_info(ep, "Measured Earlier");
        info.ping_ms = Some(31);
        let prior = ScannedServer {
            info: Some(info),
            analysis: Analysis { reasons: vec![] },
            endpoint: ep,
            query_ms: 31,
            outcome: Outcome::Ok,
        };
        let unmeasured = |outcome| ScannedServer {
            info: None,
            analysis: Analysis { reasons: vec![] },
            endpoint: ep,
            query_ms: 0,
            outcome,
        };

        // Paced: no packet went out, so the earlier measurement still stands
        // and the row stays visible behind the UI's latency filter.
        let paced = with_listed_fallback(unmeasured(Outcome::RateLimited), Some(&prior));
        assert_eq!(paced.info.as_ref().unwrap().ping_ms, Some(31));
        assert_eq!(paced.outcome, Outcome::RateLimited);

        // Timed out: it was asked and did not answer, so it has no latency.
        let dead = with_listed_fallback(unmeasured(Outcome::Timeout), Some(&prior));
        assert_eq!(dead.info.as_ref().unwrap().hostname, "Measured Earlier");
        assert_eq!(dead.info.as_ref().unwrap().ping_ms, None);
        assert_eq!(dead.outcome, Outcome::Timeout);
    }

    #[test]
    fn game_column_shows_the_servers_own_description() {
        use crate::detect::Analysis;
        let ep = Endpoint::parse("8.8.8.8:27015").unwrap();
        let game_of = |desc: &str| {
            let mut info = listed_info(ep, "Srv");
            info.game_desc = desc.into();
            row_from_scanned(&ScannedServer {
                info: Some(info),
                analysis: Analysis { reasons: vec![] },
                endpoint: ep,
                query_ms: 0,
                outcome: Outcome::Ok,
            })
            .game
        };
        assert_eq!(game_of("Zombie Plague"), "Zombie Plague");
        assert_eq!(game_of("Counter-Strike"), "Counter-Strike");
        // Blank (a listed-only row, or a server that sends none): the stock one.
        assert_eq!(game_of("  "), "Counter-Strike");
    }

    #[test]
    fn only_measured_clean_rows_are_verified() {
        use crate::detect::{Analysis, Reason};
        let ep = Endpoint::parse("8.8.8.8:27015").unwrap();
        let row = |ping: Option<u32>, outcome: Outcome, reasons: Vec<Reason>| {
            let mut info = listed_info(ep, "Srv");
            info.ping_ms = ping;
            row_from_scanned(&ScannedServer {
                info: Some(info),
                analysis: Analysis { reasons },
                endpoint: ep,
                query_ms: 0,
                outcome,
            })
        };

        let live = row(Some(20), Outcome::Ok, vec![]);
        assert!(live.verified && live.live);
        assert_eq!(live.outcome, "ok");

        // Steam's listing alone proves nothing about the server.
        let listed = row(None, Outcome::Ok, vec![]);
        assert!(!listed.verified && !listed.live);
        assert_eq!(listed.outcome, "listed");

        // Paced, carrying this session's earlier verified measurement.
        assert!(row(Some(20), Outcome::RateLimited, vec![]).verified);
        // Asked and silent: not verified.
        assert!(!row(None, Outcome::Timeout, vec![]).verified);
        // Answered, but a definite fake.
        assert!(!row(Some(20), Outcome::Ok, vec![Reason::ServerFarm]).verified);
        // A soft finding alone does not block verification.
        assert!(row(Some(20), Outcome::Ok, vec![Reason::RepeatedHostname]).verified);
    }

    #[test]
    fn verifier_flags_every_endpoint_of_a_farm_ip_from_the_listing() {
        use crate::detect::{Analysis, Reason, FARM_MIN_ENDPOINTS};
        // A farm clones one name across its ports; the real host below it
        // runs as many servers on one IP, each with its own name.
        let listing: Vec<ScannedServer> = (0..FARM_MIN_ENDPOINTS as u16)
            .map(|i| {
                let ep = Endpoint::parse(&format!("203.0.113.9:{}", 27015 + i)).unwrap();
                ScannedServer {
                    info: Some(listed_info(ep, "CS 1.6")),
                    analysis: Analysis { reasons: vec![] },
                    endpoint: ep,
                    query_ms: 0,
                    outcome: Outcome::Ok,
                }
            })
            .chain((0..FARM_MIN_ENDPOINTS as u16).map(|i| {
                let ep = Endpoint::parse(&format!("198.51.100.1:{}", 27015 + i)).unwrap();
                ScannedServer {
                    info: Some(listed_info(ep, &format!("Real Community #{i} | Public"))),
                    analysis: Analysis { reasons: vec![] },
                    endpoint: ep,
                    query_ms: 0,
                    outcome: Outcome::Ok,
                }
            }))
            .collect();
        let eps: Vec<Endpoint> = listing.iter().map(|s| s.endpoint).collect();
        let mut v = Verifier::new(&eps, &listing, Default::default());
        let mut rows = listing.clone();
        for r in rows.iter_mut() {
            v.verify(r);
        }
        let (farm, real) = rows.split_at(FARM_MIN_ENDPOINTS);
        assert!(farm
            .iter()
            .all(|r| r.analysis.reasons.contains(&Reason::ServerFarm) && r.analysis.is_fake()));
        assert!(
            real.iter().all(|r| !r.analysis.is_fake()),
            "{:?}",
            real[0].analysis.reasons
        );
        // Listing rows are not measured, so nothing enters name history.
        assert!(v.history.entries.is_empty());
    }

    #[test]
    fn measured_scan_result_is_never_overwritten_by_the_listing() {
        use crate::detect::Analysis;
        let ep = Endpoint::parse("8.8.8.8:27015").unwrap();
        let mut live_info = listed_info(ep, "Live Name");
        live_info.ping_ms = Some(20);

        let live = ScannedServer {
            info: Some(live_info),
            analysis: Analysis { reasons: vec![] },
            endpoint: ep,
            query_ms: 20,
            outcome: Outcome::Ok,
        };
        let listed = ScannedServer {
            info: Some(listed_info(ep, "Steam Name")),
            analysis: Analysis { reasons: vec![] },
            endpoint: ep,
            query_ms: 0,
            outcome: Outcome::Ok,
        };

        let merged = with_listed_fallback(live, Some(&listed));
        assert_eq!(merged.info.as_ref().unwrap().hostname, "Live Name");
    }

    #[test]
    fn no_listed_row_leaves_the_scan_result_untouched() {
        use crate::detect::Analysis;
        let ep = Endpoint::parse("8.8.8.8:27015").unwrap();
        let timed_out = ScannedServer {
            info: None,
            analysis: Analysis { reasons: vec![] },
            endpoint: ep,
            query_ms: 900,
            outcome: Outcome::Timeout,
        };
        let merged = with_listed_fallback(timed_out, None);
        assert!(merged.info.is_none());
        assert_eq!(merged.outcome, Outcome::Timeout);
        assert_eq!(merged.query_ms, 900);
    }

    #[tokio::test]
    async fn without_a_key_nothing_is_discovered() {
        let app = App::new(cfg_no_state());
        let mut events = Vec::new();
        let rows = app.scan_streaming(&mut |e| events.push(e)).await.unwrap();
        assert!(rows.is_empty());
        assert!(app.cached_rows().is_empty());
        // One explanation, then done — no fetch phase, no rows.
        assert!(matches!(&events[0], Event::Error { message } if message == NO_API_KEY_MESSAGE));
        assert!(matches!(events[1], Event::Phase { phase: Phase::Done }));
        assert_eq!(events.len(), 2);
    }

    #[test]
    fn clear_bans_leaves_pacing_intact() {
        let app = App::new(cfg_no_state());
        app.bans
            .lock()
            .ban("1.2.3.4:27015", "Server", vec!["slots>32".into()], 0);
        // Consume a pacing slot so we can tell whether it survived.
        let _ = app
            .gate
            .lock()
            .try_acquire("9.9.9.9:27015", crate::ratelimit::now_ms());
        assert_eq!(app.gate.lock().tracked(), 1);

        assert_eq!(app.clear_bans(), 1);
        assert_eq!(app.bans.lock().len(), 0);
        // Pacing must survive: clearing bans is not a licence to re-hammer.
        assert_eq!(app.gate.lock().tracked(), 1);
    }

    #[test]
    fn manual_ban_and_whitelist_change_one_endpoint() {
        let app = App::new(cfg_no_state());
        let ep = "8.8.8.8:27015";
        assert!(app.ban_server("bad endpoint", "Server").is_err());
        app.ban_server(ep, "Community server").unwrap();
        assert_eq!(app.bans.lock().get(ep).unwrap().reasons, ["manual"]);
        app.whitelist_server(ep).unwrap();
        assert!(!app.bans.lock().contains(ep));
        assert!(app.whitelist.lock().contains(ep));

        let endpoint = Endpoint::parse(ep).unwrap();
        let mut row = ScannedServer {
            info: Some(listed_info(endpoint, "Community server")),
            analysis: crate::detect::Analysis {
                reasons: vec![crate::detect::Reason::ServerFarm],
            },
            endpoint,
            query_ms: 0,
            outcome: Outcome::Ok,
        };
        app.apply_whitelist(&mut row);
        assert!(!row.analysis.is_fake());
    }

    #[test]
    fn config_updates_keep_the_shared_gate_and_ban_list() {
        // The invariant a UI depends on: changing scan parameters between
        // sweeps must not hand the new sweep a fresh pacing gate or ban list,
        // or a single-server refresh could double-query an endpoint.
        let app = App::new(cfg_no_state());
        app.bans
            .lock()
            .ban("1.2.3.4:27015", "Server", vec!["slots>32".into()], 0);
        let _ = app
            .gate
            .lock()
            .try_acquire("9.9.9.9:27015", crate::ratelimit::now_ms());

        app.set_cfg(Config {
            concurrency: 999,
            rediscover: true,
            ..cfg_no_state()
        });

        assert_eq!(app.cfg().concurrency, 999, "the new config must apply");
        assert_eq!(
            app.bans.lock().len(),
            1,
            "bans must survive a config change"
        );
        assert_eq!(
            app.gate.lock().tracked(),
            1,
            "pacing must survive a config change"
        );
    }

    #[test]
    fn hard_refresh_clears_bans_and_pacing_but_not_history() {
        let app = App::new(cfg_no_state());
        app.bans
            .lock()
            .ban("1.2.3.4:27015", "Server", vec!["slots>32".into()], 0);
        assert_eq!(app.bans.lock().len(), 1);

        let msg = app.hard_refresh();
        assert!(msg.starts_with("hard refresh"), "{msg}");
        assert_eq!(app.bans.lock().len(), 0);
        assert_eq!(app.gate.lock().tracked(), 0);
    }

    #[test]
    fn ban_rows_are_newest_first_and_carry_reasons() {
        let app = App::new(cfg_no_state());
        app.bans.lock().ban(
            "1.1.1.1:27015",
            "Farm Alpha",
            vec!["slots>32".into()],
            1_700_000_000,
        );
        app.bans.lock().ban(
            "2.2.2.2:27015",
            "Farm Beta",
            vec!["url-spam".into()],
            1_800_000_000,
        );

        let rows = app.ban_rows();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].endpoint, "2.2.2.2:27015");
        assert_eq!(rows[0].hostname, "Farm Beta");
        assert_eq!(rows[0].reasons, vec!["url-spam"]);
        assert_eq!(rows[1].endpoint, "1.1.1.1:27015");
    }

    #[test]
    fn row_from_scanned_splits_hard_and_soft_reasons() {
        use crate::detect::{Analysis, Reason};
        let s = ScannedServer {
            info: None,
            analysis: Analysis {
                reasons: vec![Reason::SlotsExceedMax, Reason::FullOfBots],
            },
            endpoint: Endpoint::parse("1.2.3.4:27015").unwrap(),
            query_ms: 0,
            outcome: Outcome::Ok,
        };
        let row = row_from_scanned(&s);
        assert!(row.fake, "slots>32 is a hard reason");
        assert_eq!(row.reasons, vec!["slots>32"]);
        assert_eq!(row.soft_reasons, vec!["all-bots"]);
    }

    fn listed_info(ep: Endpoint, hostname: &str) -> ServerInfo {
        ServerInfo {
            endpoint: ep,
            protocol: 48,
            hostname: hostname.to_string(),
            map: "de_dust2".into(),
            gamedir: "cstrike".into(),
            game: Game::CS16,
            app_id: 10,
            game_desc: "Counter-Strike".into(),
            players: 4,
            max_players: 32,
            bots: 0,
            server_type: ServerType::Dedicated,
            os: Os::Windows,
            password: false,
            vac: Vac::Secured,
            version: "48".into(),
            ping_ms: None,
            country: None,
            city: None,
            players_list: Vec::new(),
            ping_history: Vec::new(),
            bot_plugin: None,
            response_time_ms: 0,
            last_seen: chrono::Utc::now(),
        }
    }

    #[test]
    fn merge_prefers_live_but_keeps_failure_outcome() {
        use crate::detect::Analysis;
        let ep = Endpoint::parse("8.8.8.8:27015").unwrap();
        let dead = ScannedServer {
            info: None,
            analysis: Analysis { reasons: vec![] },
            endpoint: ep,
            query_ms: 42,
            outcome: Outcome::Timeout,
        };
        let listed = ScannedServer {
            info: Some(listed_info(ep, "listed")),
            analysis: Analysis { reasons: vec![] },
            endpoint: ep,
            query_ms: 0,
            outcome: Outcome::Ok,
        };
        let merged = merge_scans(vec![dead], vec![listed]);
        assert_eq!(merged.len(), 1);
        // Metadata adopted from the listing...
        assert_eq!(merged[0].info.as_ref().unwrap().hostname, "listed");
        // ...but the failure is remembered, so it is not shown as freshly probed.
        assert_eq!(merged[0].outcome, Outcome::Timeout);
        assert_eq!(merged[0].query_ms, 42);
    }

    #[test]
    fn merge_keeps_live_measurement_when_a_server_answered() {
        use crate::detect::Analysis;
        let ep = Endpoint::parse("8.8.8.8:27015").unwrap();
        let mut live_info = listed_info(ep, "live name");
        live_info.ping_ms = Some(23);
        live_info.players = 9;
        let live = ScannedServer {
            info: Some(live_info),
            analysis: Analysis { reasons: vec![] },
            endpoint: ep,
            query_ms: 23,
            outcome: Outcome::Ok,
        };
        let listed = ScannedServer {
            info: Some(listed_info(ep, "steam name")),
            analysis: Analysis { reasons: vec![] },
            endpoint: ep,
            query_ms: 0,
            outcome: Outcome::Ok,
        };
        let merged = merge_scans(vec![live], vec![listed]);
        assert_eq!(merged.len(), 1);
        // The measured row must win outright, including players and ping.
        let info = merged[0].info.as_ref().unwrap();
        assert_eq!(info.hostname, "live name");
        assert_eq!(info.players, 9);
        assert_eq!(info.ping_ms, Some(23));
    }
}
