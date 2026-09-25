//! Steam Web API client — the standalone server-list source.
//!
//! This module is the **only** server-list source. It replaces the previous
//! `steam_api.dll` matchmaking path: no Steam client, no DLL, no 32-bit build.
//!
//! ## Why this endpoint
//!
//! Reverse-engineering the live path (see `docs/steam-server-list.md`)
//! established that the in-game browser reaches the list through Steam's Game
//! Matching Service (GMS) over the CM websocket transport, carrying
//! `CMsgClientGMSServerQuery` (EMsg 6403) or the unified service method
//! `GameServers.GetServerList#1`. Both are gated behind an authenticated
//! account: an anonymous CM logon succeeds, but GMS answers with an empty
//! list for every appid (verified against appids 10/730/440/550/240).
//!
//! The same GMS data is exposed over HTTPS by `IGameServersService`, which is
//! what this module speaks:
//!
//! ```text
//! GET https://api.steampowered.com/IGameServersService/GetServerList/v1/
//!         ?key=<web api key>
//!         &filter=\appid\10\gamedir\cstrike
//!         &limit=5000
//! ```
//!
//! The `filter` parameter is byte-identical to the legacy HL1 master filter
//! built by `protocol::masterserver::build_filter` — same `\key\value` pairs,
//! same keys. Steam translates it to the same GMS query the game issues.
//!
//! ## Authentication
//!
//! A Steam Web API key is required (free, ~30 seconds at
//! <https://steamcommunity.com/dev/apikey>). It is entered in the Settings
//! dialog and stored by `crate::settings`; nothing is read from the
//! environment. A key is a per-account credential, not
//! a per-app one: any Steam account can mint one and it works for every appid.
//!
//! ## Response shape
//!
//! ```json
//! { "response": { "servers": [ {
//!     "addr": "1.2.3.4:27015",   // QUERY port, not the game port
//!     "gameport": 27015, "specport": null,
//!     "steamid": "90071996900000000", "name": "…", "appid": 10,
//!     "gamedir": "cstrike", "version": "1.1.2.7/Stdio", "product": "cstrike",
//!     "region": 255, "players": 12, "max_players": 32, "bots": 0,
//!     "map": "de_dust2", "secure": true, "dedicated": true,
//!     "os": "l", "gametype": "…"
//! } ] } }
//! ```
//!
//! Note `addr` carries the **query** port (`m_usQueryPort`), which is what A2S
//! is answered on. `gameport` is where players connect and is usually the same
//! number, but not always — the scanner wants the query port.

use crate::model::Endpoint;
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::time::Duration;

/// Default API host. Overridable for tests via `WebApi::with_base`.
pub const DEFAULT_API_BASE: &str = "https://api.steampowered.com";

/// Where a user gets a key — surfaced verbatim in the error message.
pub const API_KEY_URL: &str = "https://steamcommunity.com/dev/apikey";

/// Documented maximum for `IGameServersService/GetServerList`.
///
/// Steam's docs and node-steam-user both claim 20,000, but a live appid-10
/// fetch with `limit=20000` returned exactly **10,000** rows, so that is the
/// real hard cap. Requesting more is not an error — the API silently truncates.
/// Use `cs16browser limits` to re-measure against the current service.
pub const MAX_LIMIT: u32 = 10_000;

/// Most IPs one request can exclude with `\nor\N\gameaddr\<ip>…`.
///
/// Measured: 259 exclusions (a ~7 KB URL) were accepted; 333 were refused
/// with `HTTP 400`. 250 keeps a margin for a longer base filter.
pub const MAX_EXCLUDED_IPS: usize = 250;

/// `filter` plus a clause that drops every server on `ips`.
///
/// `\nor\N` means "none of the next N conditions", and `\gameaddr\<ip>`
/// matches every port on that IP (measured: five excluded farm IPs, zero of
/// their rows returned). This is what lets discovery see past the
/// [`MAX_LIMIT`] cap: without it, farm rows fill the whole window.
pub fn with_excluded_ips(filter: &str, ips: &[std::net::Ipv4Addr]) -> String {
    if ips.is_empty() {
        return filter.to_string();
    }
    let mut out = format!("{filter}\\nor\\{}", ips.len());
    for ip in ips {
        out.push_str(&format!("\\gameaddr\\{ip}"));
    }
    out
}

/// The value we send when the caller wants "everything".
///
/// Steam truncates over-large values instead of rejecting them, so this is
/// simply [`MAX_LIMIT`].
pub const REQUEST_LIMIT: u32 = MAX_LIMIT;

/// Environment variable overriding the API host.
///
/// Exists so the pipeline can be exercised end-to-end against a local stub
/// (see `tools/repro_hang.py`) without a real key or the live service.
pub const API_BASE_ENV: &str = "CS16BROWSER_API_BASE";

/// Resolve the API base: environment override, else the live host.
///
/// The override is an operational/testing hook; it is intentionally not a CLI
/// flag so it cannot be mistaken for a supported deployment target.
pub fn resolve_api_base() -> String {
    std::env::var(API_BASE_ENV)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| DEFAULT_API_BASE.to_string())
}

/// What a multi-round fetch actually did.
///
/// Surfaced so the CLI can report real numbers ("N unique from M rounds")
/// instead of implying the list was complete in one request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FetchStats {
    /// Rounds actually performed.
    pub rounds: u32,
    /// Rows returned by the most recent round.
    pub last_batch: usize,
    /// Endpoints first seen in the most recent round.
    pub new_this_round: usize,
    /// Rows dropped because their endpoint was already collected.
    pub duplicates_skipped: usize,
    /// Rows dropped because `addr` would not parse.
    pub unparseable: usize,
    /// Unique endpoints collected in total.
    pub total: usize,
}

impl FetchStats {
    /// True when the last round contributed nothing, i.e. we stopped because we
    /// ran out of new servers rather than out of rounds.
    pub fn converged(&self) -> bool {
        self.new_this_round == 0
    }
}

/// Envelope: `{ "response": { "servers": [...] } }`.
#[derive(Debug, Clone, Deserialize)]
struct GetServerListEnvelope {
    response: GetServerListBody,
}

#[derive(Debug, Clone, Deserialize)]
struct GetServerListBody {
    #[serde(default)]
    servers: Vec<ApiServer>,
}

/// One row of `IGameServersService/GetServerList`.
///
/// Every field is optional in the schema (proto2 `optional` upstream), so a
/// missing key must not fail the whole parse — a partially-populated row is
/// still a usable endpoint.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApiServer {
    /// The server's IP and **query** port, `"ip:port"`.
    pub addr: String,
    /// Port game clients connect to.
    #[serde(default)]
    pub gameport: Option<u32>,
    /// Spectator port, if any.
    #[serde(default)]
    pub specport: Option<u32>,
    /// The server's persistent SteamID, as a decimal string.
    #[serde(default)]
    pub steamid: Option<String>,
    /// Hostname shown in the browser.
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub appid: Option<u32>,
    #[serde(default)]
    pub gamedir: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    #[serde(default)]
    pub product: Option<String>,
    #[serde(default)]
    pub region: Option<i32>,
    #[serde(default)]
    pub players: Option<i32>,
    #[serde(default)]
    pub max_players: Option<i32>,
    #[serde(default)]
    pub bots: Option<i32>,
    #[serde(default)]
    pub map: Option<String>,
    #[serde(default)]
    pub secure: Option<bool>,
    #[serde(default)]
    pub dedicated: Option<bool>,
    #[serde(default)]
    pub os: Option<String>,
    #[serde(default)]
    pub gametype: Option<String>,
}

impl ApiServer {
    /// Parse `addr` into an `Endpoint` (query port).
    pub fn endpoint(&self) -> Option<Endpoint> {
        Endpoint::parse(self.addr.trim())
    }
}

/// A configured Steam Web API client.
#[derive(Debug, Clone)]
pub struct WebApi {
    key: String,
    base: String,
    timeout: Duration,
    /// Shared by every request from this client (clones included), so a
    /// multi-request discovery reuses one kept-alive HTTPS connection instead
    /// of paying a TCP + TLS handshake per request.
    agent: ureq::Agent,
}

/// An agent that leaves 4xx/5xx to the caller: status codes are read from
/// `response.status()`, so an error body is still available for the per-code
/// messages (a plain `Err` would have discarded it).
fn build_agent(timeout: Duration) -> ureq::Agent {
    let config = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(timeout))
        .build();
    ureq::Agent::new_with_config(config)
}

impl WebApi {
    /// Build a client from a Web API key.
    pub fn new(key: impl Into<String>) -> Self {
        let timeout = Duration::from_secs(20);
        Self {
            key: key.into(),
            base: resolve_api_base(),
            timeout,
            agent: build_agent(timeout),
        }
    }

    /// Override the API host (used by tests).
    pub fn with_base(mut self, base: impl Into<String>) -> Self {
        self.base = base.into();
        self
    }

    /// Override the per-request timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self.agent = build_agent(timeout);
        self
    }

    /// Redact the API key out of any text before it reaches a log or an error.
    ///
    /// The key travels in the query string, so any error that quotes the URL
    /// (transport errors do) would otherwise print it in full. Replace every
    /// occurrence with `key=<redacted>`.
    fn redact(&self, text: &str) -> String {
        if self.key.is_empty() {
            return text.to_string();
        }
        text.replace(&self.key, "<redacted>")
    }

    /// Build a redacted context string for a transport-level failure.
    fn transport_error(&self, e: impl std::fmt::Display) -> anyhow::Error {
        anyhow!(
            "Steam Web API request failed: {}. \
             (If this repeats, check connectivity to {}.)",
            self.redact(&e.to_string()),
            self.base
        )
    }

    /// Fetch the server list for `game` using `filter`.
    ///
    /// `filter` is the same `\key\value` string the legacy master protocol
    /// uses; build it with `protocol::masterserver::build_filter`.
    ///
    /// `limit` is clamped to [`MAX_LIMIT`]; the API truncates silently rather
    /// than erroring, so a larger value just wastes the request.
    pub fn fetch_server_list(&self, filter: &str, limit: u32) -> Result<Vec<ApiServer>> {
        let limit = limit.min(MAX_LIMIT);
        let url = format!("{}/IGameServersService/GetServerList/v1/", self.base);
        tracing::debug!(url = %url, filter = %filter, limit, "[webapi] GetServerList request");
        let started = std::time::Instant::now();
        let response = self
            .agent
            .get(&url)
            .query("key", &self.key)
            .query("filter", filter)
            .query("limit", limit.to_string())
            .call();

        let mut response = match response {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(
                    elapsed_ms = started.elapsed().as_millis() as u64,
                    error = %self.redact(&e.to_string()),
                    "[webapi] GetServerList transport error"
                );
                return Err(self.transport_error(e));
            }
        };

        let code = response.status().as_u16();
        if code >= 400 {
            let body = self.redact(&response.body_mut().read_to_string().unwrap_or_default());
            tracing::warn!(
                code,
                elapsed_ms = started.elapsed().as_millis() as u64,
                body = %truncate(&body),
                "[webapi] GetServerList returned an error status"
            );
            return Err(match code {
                401 | 403 => anyhow!(
                    "Steam Web API rejected the key (HTTP {code}). Keys are per-account; \
                     mint or replace yours at {API_KEY_URL}. Response: {}",
                    truncate(&body)
                ),
                429 => anyhow!("Steam Web API rate limit hit (HTTP 429). Retry shortly."),
                502..=504 => anyhow!(
                    "Steam Web API is unavailable (HTTP {code}). This is a Steam-side \
                     outage; retry in a few minutes. Response: {}",
                    truncate(&body)
                ),
                _ => anyhow!("Steam Web API returned HTTP {code}: {}", truncate(&body)),
            });
        }

        let envelope: GetServerListEnvelope = response
            .body_mut()
            .read_json()
            .context("Steam Web API returned a body that is not valid GetServerList JSON")?;
        tracing::debug!(
            elapsed_ms = started.elapsed().as_millis() as u64,
            rows = envelope.response.servers.len(),
            "[webapi] GetServerList response"
        );
        Ok(envelope.response.servers)
    }

    /// [`Self::fetch_server_list`] on the blocking pool.
    pub async fn fetch_server_list_async(&self, filter: &str) -> Result<Vec<ApiServer>> {
        let this = self.clone();
        let filter = filter.to_string();
        tokio::task::spawn_blocking(move || this.fetch_server_list(&filter, MAX_LIMIT))
            .await
            .context("Steam Web API task panicked")?
    }

    /// Fetch repeatedly with a per-round progress callback.
    ///
    /// The endpoint exposes only `filter` and `limit` — no offset or cursor —
    /// and its `limit` is a cap on a **non-deterministic sample**, not a page
    /// window, so fetching is repeated while each round still finds new
    /// endpoints (see the measurements in the module docs). This variant
    /// reports each round as it lands, so callers can display the first batch
    /// immediately and append later ones.
    pub fn fetch_all_streaming<F>(
        &self,
        filter: &str,
        limit: u32,
        rounds: u32,
        min_new_ratio: f64,
        mut on_round: F,
    ) -> Result<FetchStats>
    where
        F: FnMut(&[ApiServer], &FetchStats),
    {
        let limit = limit.clamp(1, MAX_LIMIT);
        let mut seen: HashSet<Endpoint> = HashSet::new();
        let mut fresh: Vec<ApiServer> = Vec::new();
        let mut stats = FetchStats::default();

        for round in 0..rounds.max(1) {
            let batch = self.fetch_server_list(filter, limit)?;
            stats.rounds = round + 1;
            stats.last_batch = batch.len();
            let fetched = batch.len();
            fresh.clear();

            for row in batch {
                let Some(endpoint) = row.endpoint() else {
                    stats.unparseable += 1;
                    continue;
                };
                if seen.insert(endpoint) {
                    fresh.push(row);
                }
            }

            stats.new_this_round = fresh.len();
            stats.duplicates_skipped += fetched.saturating_sub(fresh.len());
            stats.total = seen.len();

            // Report this round before deciding whether to continue, so the
            // caller sees data as early as possible.
            on_round(&fresh, &stats);

            if stats.last_batch == 0 || stats.new_this_round == 0 {
                break;
            }
            if fetched > 0 && (stats.new_this_round as f64) < (fetched as f64) * min_new_ratio {
                break;
            }
        }

        Ok(stats)
    }

    /// Async wrapper around [`Self::fetch_all_streaming`].
    ///
    /// Each round runs on the blocking pool; its new rows are handed to
    /// `on_round` as soon as that round completes, so the caller can render the
    /// first batch long before the last round finishes.
    pub async fn fetch_all_streaming_async<F>(
        &self,
        filter: &str,
        limit: u32,
        rounds: u32,
        min_new_ratio: f64,
        mut on_round: F,
    ) -> Result<FetchStats>
    where
        F: FnMut(Vec<ApiServer>, &FetchStats),
    {
        let limit = limit.clamp(1, MAX_LIMIT);
        let mut seen: HashSet<Endpoint> = HashSet::new();
        let mut stats = FetchStats::default();

        for round in 0..rounds.max(1) {
            let this = self.clone();
            let filter_owned = filter.to_string();
            let batch =
                tokio::task::spawn_blocking(move || this.fetch_server_list(&filter_owned, limit))
                    .await
                    .context("Steam Web API task panicked")??;

            stats.rounds = round + 1;
            stats.last_batch = batch.len();
            let fetched = batch.len();
            let mut fresh = Vec::with_capacity(fetched);

            for row in batch {
                let Some(endpoint) = row.endpoint() else {
                    stats.unparseable += 1;
                    continue;
                };
                if seen.insert(endpoint) {
                    fresh.push(row);
                }
            }

            stats.new_this_round = fresh.len();
            stats.duplicates_skipped += fetched.saturating_sub(fresh.len());
            stats.total = seen.len();

            // Report before deciding whether to continue, so data reaches the
            // UI as early as possible.
            on_round(fresh, &stats);

            if stats.last_batch == 0 || stats.new_this_round == 0 {
                break;
            }
            if fetched > 0 && (stats.new_this_round as f64) < (fetched as f64) * min_new_ratio {
                break;
            }
        }

        Ok(stats)
    }
}

fn truncate(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() <= 200 {
        s.to_string()
    } else {
        s.chars().take(200).collect::<String>() + "…"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excluded_ips_become_one_nor_clause() {
        let ips = ["1.2.3.4".parse().unwrap(), "5.6.7.8".parse().unwrap()];
        assert_eq!(
            with_excluded_ips("\\appid\\10", &ips),
            "\\appid\\10\\nor\\2\\gameaddr\\1.2.3.4\\gameaddr\\5.6.7.8"
        );
        assert_eq!(with_excluded_ips("\\appid\\10", &[]), "\\appid\\10");
    }

    /// A representative `GetServerList` payload (appid 10).
    const SAMPLE: &str = r#"{
      "response": {
        "servers": [
          {
            "addr": "203.0.113.7:27015",
            "gameport": 27015,
            "specport": null,
            "steamid": "90071996900000007",
            "name": "Example CS 1.6 Server",
            "appid": 10,
            "gamedir": "cstrike",
            "version": "1.1.2.7/Stdio",
            "product": "cstrike",
            "region": 255,
            "players": 12,
            "max_players": 32,
            "bots": 0,
            "map": "de_dust2",
            "secure": true,
            "dedicated": true,
            "os": "l",
            "gametype": "public"
          },
          {
            "addr": "198.51.100.9:27016",
            "gameport": 27016,
            "name": null,
            "players": 0,
            "map": "cs_italy"
          }
        ]
      }
    }"#;

    #[test]
    fn parses_servers_and_uses_query_port() {
        let env: GetServerListEnvelope = serde_json::from_str(SAMPLE).unwrap();
        let servers = env.response.servers;
        assert_eq!(servers.len(), 2);

        let first = servers[0].endpoint().expect("addr must parse");
        assert_eq!(first.to_string(), "203.0.113.7:27015");
        // `addr` is the query port; the scanner must target that, not gameport.
        assert_eq!(first.port, 27015);
        assert_eq!(servers[0].players, Some(12));
        assert_eq!(servers[0].max_players, Some(32));
        assert_eq!(servers[0].secure, Some(true));
        assert_eq!(servers[0].gamedir.as_deref(), Some("cstrike"));

        // Sparse rows must still yield a usable endpoint.
        let second = servers[1].endpoint().expect("sparse addr must parse");
        assert_eq!(second.to_string(), "198.51.100.9:27016");
        assert_eq!(servers[1].bots, None);
        assert_eq!(servers[1].secure, None);
    }

    #[test]
    fn addr_may_differ_from_gameport() {
        // Real servers can answer A2S on a different port than they accept
        // players on; the endpoint must follow `addr`.
        let json = r#"{"response":{"servers":[
            {"addr":"203.0.113.7:27099","gameport":27015}
        ]}}"#;
        let env: GetServerListEnvelope = serde_json::from_str(json).unwrap();
        let ep = env.response.servers[0].endpoint().unwrap();
        assert_eq!(ep.port, 27099);
    }

    #[test]
    fn empty_response_is_not_an_error() {
        let env: GetServerListEnvelope =
            serde_json::from_str(r#"{"response":{"servers":[]}}"#).unwrap();
        assert!(env.response.servers.is_empty());
    }

    #[test]
    fn missing_servers_key_is_not_an_error() {
        let env: GetServerListEnvelope = serde_json::from_str(r#"{"response":{}}"#).unwrap();
        assert!(env.response.servers.is_empty());
    }

    #[test]
    fn malformed_addr_is_skipped_not_fatal() {
        let json = r#"{"response":{"servers":[
            {"addr":"not-an-endpoint"},
            {"addr":"203.0.113.7:27015"}
        ]}}"#;
        let env: GetServerListEnvelope = serde_json::from_str(json).unwrap();
        let endpoints: Vec<_> = env
            .response
            .servers
            .iter()
            .filter_map(|s| s.endpoint())
            .collect();
        assert_eq!(endpoints.len(), 1);
        assert_eq!(endpoints[0].to_string(), "203.0.113.7:27015");
    }

    #[test]
    fn empty_body_errors_clearly() {
        let err = serde_json::from_str::<GetServerListEnvelope>("")
            .expect_err("empty body must not parse");
        assert!(!err.to_string().is_empty());
    }
}
