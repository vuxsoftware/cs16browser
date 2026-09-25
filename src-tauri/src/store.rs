//! Local discovery cache.
//!
//! Steam's server list is fetched **once** and written to a JSON file; every
//! later run works from that file. This keeps the tool usable without hitting
//! `IGameServersService` at all, which matters because:
//!
//! * each fetch is a metered API call (and can 502 during a Steam outage),
//! * the endpoint returns a *sample*, so re-fetching to "refresh" mostly
//!   re-downloads rows already known,
//! * a full sweep of ~10k endpoints costs minutes, and re-discovering before
//!   every look would make the tool unusable.
//!
//! Re-discovery is explicit: the TUI's `X` (hard refresh) or `--rediscover`.
//!
//! ## What is stored
//!
//! The raw rows Steam returned — endpoint plus the metadata it publishes
//! (name, map, players, gamedir, …). Deliberately **not** stored: live A2S
//! measurements (ping, real player counts, player lists). Those are per-run
//! observations; persisting them would make stale data look freshly measured.
//! `discovered_at` records when the data was captured so it can be aged.
//!
//! Also the redirect-farm IPs discovery condemned ([`KnownFarm`]). A
//! rediscovery excludes them from its **first** request, so Steam's capped
//! window holds real servers from the start: measured, that completes in 3
//! requests (~5 s) where peeling the farms again took 13 (~22 s).

use crate::client::webapi::ApiServer;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};

/// Schema version; a file from a different version is re-discovered rather than
/// misread.
///
/// 2: discovery excludes farm IPs until Steam's answer is complete. A
/// version-1 cache is a single truncated 10,000-row window, so it is refetched.
pub const STORE_VERSION: u32 = 2;

/// Directory holding the cache, created on first write.
pub const STORE_DIR: &str = "cs16browser-data";

/// File name inside [`STORE_DIR`].
pub const STORE_FILE: &str = "servers.json";

/// Environment override for the whole store directory.
pub const STORE_DIR_ENV: &str = "CS16BROWSER_DATA";

/// How long a farm IP stays excluded without being seen again.
///
/// An excluded IP returns no rows, so it cannot be re-condemned while it is
/// excluded. After this long it is left out of the first request once, and
/// its rows decide again: a farm is condemned afresh, an IP that turned into
/// a real host is listed normally.
pub const FARM_SEED_TTL_SECS: i64 = 7 * 86_400;

/// A redirect-farm IP condemned by [`crate::detect::farm_ips`] from its own
/// listed rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KnownFarm {
    pub ip: Ipv4Addr,
    /// Listed endpoints when it was condemned; bigger farms are excluded first.
    pub ports: u32,
    /// Unix seconds when its rows last condemned it.
    pub condemned_at: i64,
}

/// One discovery result, as persisted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerStore {
    #[serde(default = "default_version")]
    pub version: u32,
    /// Unix seconds when the list was fetched from Steam.
    #[serde(default)]
    pub discovered_at: i64,
    /// The master filter used, so a changed filter can invalidate the cache.
    #[serde(default)]
    pub filter: String,
    /// `gamedir` the list was fetched for.
    #[serde(default)]
    pub gamedir: String,
    /// Steam rows.
    #[serde(default)]
    pub servers: Vec<ApiServer>,
    /// Farm IPs, including ones the last discovery excluded (and so holds
    /// no rows for). See [`ServerStore::known_farms`].
    #[serde(default)]
    pub farms: Vec<KnownFarm>,
}

fn default_version() -> u32 {
    STORE_VERSION
}

impl ServerStore {
    pub fn new(
        filter: impl Into<String>,
        gamedir: impl Into<String>,
        servers: Vec<ApiServer>,
    ) -> Self {
        Self {
            version: STORE_VERSION,
            discovered_at: crate::history::now_unix(),
            filter: filter.into(),
            gamedir: gamedir.into(),
            servers,
            farms: Vec::new(),
        }
    }

    /// Farm IPs the stored rows condemn, merged with the persisted list;
    /// entries older than [`FARM_SEED_TTL_SECS`] are dropped. Largest first.
    pub fn known_farms(&self, now: i64) -> Vec<KnownFarm> {
        let mut ports: HashMap<Ipv4Addr, u32> = HashMap::new();
        let mut names: Vec<(Ipv4Addr, &str)> = Vec::new();
        for s in &self.servers {
            if let Some(ep) = s.endpoint() {
                *ports.entry(ep.ip).or_default() += 1;
                names.push((ep.ip, s.name.as_deref().unwrap_or("")));
            }
        }
        let condemned = crate::detect::farm_ips(
            names
                .iter()
                .map(|&(ip, hostname)| crate::detect::ListedEndpoint { ip, hostname }),
        );
        let mut by_ip: HashMap<Ipv4Addr, KnownFarm> = HashMap::new();
        let from_rows = condemned.into_iter().map(|ip| KnownFarm {
            ip,
            ports: ports.get(&ip).copied().unwrap_or(0),
            condemned_at: self.discovered_at,
        });
        for f in self.farms.iter().copied().chain(from_rows) {
            by_ip
                .entry(f.ip)
                .and_modify(|e| {
                    e.ports = e.ports.max(f.ports);
                    e.condemned_at = e.condemned_at.max(f.condemned_at);
                })
                .or_insert(f);
        }
        let mut out: Vec<KnownFarm> = by_ip
            .into_values()
            .filter(|f| now.saturating_sub(f.condemned_at) < FARM_SEED_TTL_SECS)
            .collect();
        out.sort_by(|a, b| b.ports.cmp(&a.ports).then(a.ip.cmp(&b.ip)));
        out
    }

    /// The farm list to persist after a discovery that listed `self.servers`
    /// having excluded `excluded` from the start: every IP these rows condemn
    /// (as of now), plus each `prior` farm that was excluded and so could not
    /// be judged again. A prior farm whose rows came back uncondemned is gone.
    pub fn carry_farms(&mut self, prior: &[KnownFarm], excluded: &[Ipv4Addr]) {
        let listed: std::collections::HashSet<Ipv4Addr> = self
            .servers
            .iter()
            .filter_map(|s| s.endpoint().map(|ep| ep.ip))
            .collect();
        let mut farms = self.known_farms(self.discovered_at);
        farms.extend(
            prior
                .iter()
                .filter(|f| excluded.contains(&f.ip) && !listed.contains(&f.ip))
                .copied(),
        );
        self.farms = farms;
    }

    pub fn len(&self) -> usize {
        self.servers.len()
    }

    pub fn is_empty(&self) -> bool {
        self.servers.is_empty()
    }

    /// Age of the cached discovery.
    pub fn age_secs(&self) -> i64 {
        crate::history::now_unix().saturating_sub(self.discovered_at)
    }

    /// Human-readable age, for status lines.
    pub fn age_label(&self) -> String {
        let s = self.age_secs();
        match s {
            s if s < 90 => "just now".to_string(),
            s if s < 3600 => format!("{}m ago", s / 60),
            s if s < 86_400 => format!("{}h ago", s / 3600),
            s => format!("{}d ago", s / 86_400),
        }
    }

    /// Whether this cache can serve `filter`/`gamedir`.
    ///
    /// A different filter means a different query, so the rows are not
    /// reusable and the caller must re-discover.
    pub fn matches(&self, filter: &str, gamedir: &str) -> bool {
        self.version == STORE_VERSION
            && !self.servers.is_empty()
            && self.filter == filter
            && self.gamedir == gamedir
    }

    /// Load from `path`; `None` when absent, unreadable, or a different version.
    pub fn load(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        match serde_json::from_str::<ServerStore>(&text) {
            Ok(s) if s.version == STORE_VERSION => Some(s),
            Ok(s) => {
                tracing::warn!(
                    "[store] ignoring {} (version {}, expected {STORE_VERSION})",
                    path.display(),
                    s.version
                );
                None
            }
            Err(e) => {
                tracing::warn!("[store] ignoring unreadable {}: {e}", path.display());
                None
            }
        }
    }

    /// Write atomically, so an interrupted run cannot corrupt the cache.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            if !dir.as_os_str().is_empty() {
                std::fs::create_dir_all(dir)
                    .with_context(|| format!("create {}", dir.display()))?;
            }
        }
        let json = serde_json::to_string(self).context("serialize server store")?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json).with_context(|| format!("write {}", tmp.display()))?;
        std::fs::rename(&tmp, path)
            .with_context(|| format!("rename {} -> {}", tmp.display(), path.display()))?;
        Ok(())
    }
}

/// Base directories searched for existing local data, in priority order.
///
/// The current directory comes first, so every documented CLI workflow is
/// unchanged. The executable's directory is second because a GUI started from
/// Explorer or a shortcut has an arbitrary cwd — often unrelated to where its
/// data lives — and looking only at the cwd makes it miss its own cache.
pub fn base_dirs() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        v.push(cwd);
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let d = dir.to_path_buf();
            if !v.contains(&d) {
                v.push(d);
            }
        }
    }
    if v.is_empty() {
        v.push(PathBuf::from("."));
    }
    v
}

/// First `base/dir/` that already contains `file`, if any.
fn existing_dir(bases: &[PathBuf], dir: &str, file: &str) -> Option<PathBuf> {
    bases
        .iter()
        .map(|b| b.join(dir))
        .find(|d| d.join(file).is_file())
}

/// Directory holding the cache, created on first write.
///
/// Honours `CS16BROWSER_DATA`, then reuses whichever base directory already
/// holds a store, and otherwise defaults to the current directory.
pub fn store_dir() -> PathBuf {
    if let Ok(p) = std::env::var(STORE_DIR_ENV) {
        let p = p.trim();
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    let bases = base_dirs();
    existing_dir(&bases, STORE_DIR, STORE_FILE).unwrap_or_else(|| bases[0].join(STORE_DIR))
}

/// Path of the cache file.
pub fn store_path() -> PathBuf {
    store_dir().join(STORE_FILE)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(addr: &str, name: &str) -> ApiServer {
        serde_json::from_str(&format!(
            r#"{{"addr":"{addr}","name":"{name}","appid":10,"gamedir":"cstrike",
                 "map":"de_dust2","players":5,"max_players":32}}"#
        ))
        .unwrap()
    }

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("cs16store_{tag}_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn saves_and_loads_rows() {
        let dir = tmpdir("rt");
        let path = dir.join(STORE_FILE);
        let store = ServerStore::new(
            "\\appid\\10",
            "cstrike",
            vec![row("203.0.113.1:27015", "A")],
        );

        store.save(&path).expect("save must succeed");
        let back = ServerStore::load(&path).expect("must reload");

        assert_eq!(back.len(), 1);
        assert_eq!(back.servers[0].addr, "203.0.113.1:27015");
        assert_eq!(back.servers[0].name.as_deref(), Some("A"));
        assert_eq!(back.filter, "\\appid\\10");
        assert_eq!(back.gamedir, "cstrike");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A missing cache is not an error: the caller discovers instead.
    #[test]
    fn missing_file_is_none() {
        let path = std::env::temp_dir().join("cs16store_absent_xyz.json");
        let _ = std::fs::remove_file(&path);
        assert!(ServerStore::load(&path).is_none());
    }

    #[test]
    fn corrupt_file_is_none_rather_than_fatal() {
        let dir = tmpdir("bad");
        let path = dir.join(STORE_FILE);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, b"{not json").unwrap();
        assert!(ServerStore::load(&path).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn future_version_is_rejected() {
        let dir = tmpdir("ver");
        let path = dir.join(STORE_FILE);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(&path, br#"{"version":99,"servers":[{"addr":"1.2.3.4:1"}]}"#).unwrap();
        assert!(ServerStore::load(&path).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// The cache is only reusable for the query that produced it.
    #[test]
    fn filter_and_gamedir_must_match() {
        let s = ServerStore::new(
            "\\appid\\10\\gamedir\\cstrike",
            "cstrike",
            vec![row("203.0.113.1:27015", "A")],
        );
        assert!(s.matches("\\appid\\10\\gamedir\\cstrike", "cstrike"));
        assert!(
            !s.matches("\\appid\\10\\gamedir\\czero", "czero"),
            "different filter"
        );
        assert!(
            !s.matches("\\appid\\10\\gamedir\\cstrike", "czero"),
            "different gamedir"
        );
    }

    /// An empty cache must not be treated as usable, or the tool would show an
    /// empty list forever without re-discovering.
    #[test]
    fn empty_store_does_not_match() {
        let s = ServerStore::new("f", "cstrike", vec![]);
        assert!(!s.matches("f", "cstrike"));
    }

    #[test]
    fn age_is_reported_readably() {
        let mut s = ServerStore::new("f", "cstrike", vec![row("1.2.3.4:1", "x")]);
        s.discovered_at = crate::history::now_unix();
        assert_eq!(s.age_label(), "just now");

        s.discovered_at = crate::history::now_unix() - 600;
        assert!(s.age_label().ends_with("m ago"), "{}", s.age_label());

        s.discovered_at = crate::history::now_unix() - 7_200;
        assert!(s.age_label().ends_with("h ago"), "{}", s.age_label());

        s.discovered_at = crate::history::now_unix() - 3 * 86_400;
        assert!(s.age_label().ends_with("d ago"), "{}", s.age_label());
    }

    fn farm_rows(ip: &str, n: usize) -> Vec<ApiServer> {
        (0..n)
            .map(|p| row(&format!("{ip}:{}", 27000 + p), "CS 1.6"))
            .collect()
    }

    #[test]
    fn known_farms_come_from_rows_and_expire() {
        let mut rows = farm_rows("198.51.100.7", 20);
        rows.push(row("203.0.113.1:27015", "Real"));
        let mut s = ServerStore::new("f", "cstrike", rows);
        let now = s.discovered_at;
        let farms = s.known_farms(now);
        assert_eq!(farms.len(), 1, "only the cloned-name IP is a farm");
        assert_eq!(farms[0].ip, "198.51.100.7".parse::<Ipv4Addr>().unwrap());
        assert_eq!(farms[0].ports, 20);

        s.discovered_at = now - FARM_SEED_TTL_SECS;
        assert!(s.known_farms(now).is_empty(), "stale farms are not seeded");
    }

    #[test]
    fn carried_farms_keep_excluded_ips_and_drop_reformed_ones() {
        let old = |ip: &str| KnownFarm {
            ip: ip.parse().unwrap(),
            ports: 100,
            condemned_at: 1,
        };
        let prior = [
            old("198.51.100.1"),
            old("198.51.100.2"),
            old("198.51.100.3"),
        ];
        let excluded: Vec<Ipv4Addr> = prior[..2].iter().map(|f| f.ip).collect();
        // .1 stayed excluded (no rows); .2 was excluded too but is listed
        // anyway; .3 was not excluded and its rows came back clean.
        let mut s = ServerStore::new(
            "f",
            "cstrike",
            vec![
                row("198.51.100.2:27015", "Real A"),
                row("198.51.100.3:27015", "Real B"),
            ],
        );
        s.carry_farms(&prior, &excluded);
        let ips: Vec<Ipv4Addr> = s.farms.iter().map(|f| f.ip).collect();
        assert_eq!(ips, vec!["198.51.100.1".parse::<Ipv4Addr>().unwrap()]);
        assert_eq!(s.farms[0].condemned_at, 1, "an unseen farm keeps its age");
    }

    #[test]
    fn a_cache_without_farms_still_loads() {
        let dir = tmpdir("nofarms");
        let path = dir.join(STORE_FILE);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            &path,
            format!(r#"{{"version":{STORE_VERSION},"servers":[{{"addr":"1.2.3.4:1"}}]}}"#),
        )
        .unwrap();
        assert!(ServerStore::load(&path).expect("loads").farms.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Live measurements must not leak into the cache.
    #[test]
    fn stored_rows_carry_no_measurements() {
        let dir = tmpdir("nofields");
        let path = dir.join(STORE_FILE);
        ServerStore::new("f", "cstrike", vec![row("203.0.113.1:27015", "A")])
            .save(&path)
            .unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        for leaked in ["ping_ms", "last_seen", "response_time_ms", "players_list"] {
            assert!(
                !text.contains(leaked),
                "{leaked} must not be persisted: stale data would look measured"
            );
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
