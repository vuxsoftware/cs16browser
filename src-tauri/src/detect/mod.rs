//! Fake-server detection.
//!
//! The CS 1.6 server list is full of "fake" or "redirect" servers. Every
//! rule here was measured against a complete Steam listing (36,506 rows,
//! ~2,500 of them real): a rule that fired on real communities was removed,
//! not softened.
//!
//! | Pattern            | Signal                                                         |
//! |--------------------|----------------------------------------------------------------|
//! | **Slot inflation** | `max_players` above 32, or more than 32 players (engine cap).  |
//! | **Server farm**    | One IP with a wall of cloned / machine-generated names.        |
//! | **Signpost**       | Hostname advertises another server's public IP ("NEW IP ->").  |
//! | **Garbage**        | Empty hostname, map or gamedir.                                |
//! | **Name churn**     | The endpoint keeps adopting completely new identities.         |
//! | **Blocklist**      | Platform-only servers (matchmaking) that are noise here.       |
//! | **Soft cues**      | Every slot a bot; the same name on many servers.               |
//!
//! Hostname text such as URLs, "join us", Discord invites or tab padding is
//! *not* a signal: measured, every real server that carried it was a
//! community advertising itself from its own address.
//!
//! # Layout
//!
//! Each heuristic is one [`Detector`] in its own file under [`detectors`],
//! and raises exactly one [`Reason`]. [`analyze`] runs them all in
//! registry order and collects the reasons that fire. [`Reason`] owns the
//! label and severity of every verdict.
//!
//! Adding a heuristic:
//!
//! 1. add a variant to [`Reason`], with its `label` and `severity`
//!    (the compiler reports any missing match arm);
//! 2. add `detectors/<name>.rs` with a unit struct implementing [`Detector`];
//! 3. list it in [`detectors::ALL`]. A test fails if any reason has no
//!    detector.
//!
//! Listing-wide verdicts ([`farm_ips`], repeated hostnames) are computed by
//! the caller over the whole listing and handed in through [`Context`].

mod detector;
pub mod detectors;
mod reason;

use crate::model::ServerInfo;

pub use detector::{Detector, Subject};
pub use detectors::blocked_name::BLOCKED_NAMES;
pub use detectors::server_farm::{
    farm_ips, is_random_token, ListedEndpoint, FARM_MIN_ENDPOINTS, FARM_VOLUME_ENDPOINTS,
};
pub use detectors::slot_cap::CS16_MAX_SLOTS;
pub use reason::{Reason, Severity};

/// Result of a fake-analysis.
#[derive(Debug, Clone)]
pub struct Analysis {
    pub reasons: Vec<Reason>,
}

impl Analysis {
    pub fn is_fake(&self) -> bool {
        self.reasons.iter().any(|r| r.severity().is_hard())
    }
}

/// Optional context the caller can pass in for the broader checks.
#[derive(Debug, Default, Clone)]
pub struct Context {
    /// How many listed servers (this one included) carry this hostname.
    pub hostname_repeat_count: usize,
    /// The server's IP is a farm, per [`farm_ips`] over the whole listing.
    pub on_farm_ip: bool,
    /// Complete name changes observed for this endpoint across runs.
    ///
    /// Cosmetic differences (including a clock suffix in the name) do not
    /// count — see `history::is_complete_change`. Filled by
    /// `scanner::Verifier`.
    pub name_major_changes: u32,
    /// Distinct identities seen for this endpoint (for the detail pane).
    pub name_variants: usize,
}

/// Run the analysis on a single server.
pub fn analyze(info: &ServerInfo, ctx: &Context) -> Analysis {
    let subject = Subject::new(info, ctx);
    let reasons = detectors::ALL
        .iter()
        .filter(|d| d.fires(&subject))
        .map(|d| d.reason())
        .collect();
    Analysis { reasons }
}

/// Shared fixtures for the detector tests.
#[cfg(test)]
pub(crate) mod testing {
    use crate::model::{Endpoint, Game, Os, ServerInfo, ServerType, Vac};
    use chrono::Utc;
    use std::net::Ipv4Addr;

    /// A clean CS 1.6 server on 1.2.3.4:27015 named `host`.
    pub fn sample(host: &str) -> ServerInfo {
        ServerInfo {
            endpoint: Endpoint::new(Ipv4Addr::new(1, 2, 3, 4), 27015),
            protocol: 48,
            hostname: host.into(),
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
            version: "1.1.2.6/Stdio".into(),
            ping_ms: Some(45),
            country: None,
            city: None,
            players_list: vec![],
            ping_history: vec![],
            bot_plugin: None,
            response_time_ms: 50,
            last_seen: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::sample;
    use super::*;

    #[test]
    fn clean_server_has_no_reasons() {
        let a = analyze(&sample("Clean CS 1.6 #1 | de_dust2"), &Context::default());
        assert!(a.reasons.is_empty(), "{:?}", a.reasons);
    }

    /// Measured false positives: real servers that answered A2S from their
    /// own address while promoting themselves in the name.
    #[test]
    fn self_promotion_is_not_flagged() {
        for name in [
            "CS.CSDARK.RO --> Happy Family!JOIN US NOW",
            "[WB]  Bunny http://kz.clanwb.net",
            "[NIGHT KILLERS] | ZOMBIE PLAGUE UPGRADE | https://csget.example",
            "\t\tPUSHKI + LAZERY #1 FRAGLIMIT.RU",
            "MoHicans CS 1.6 | cs1.mohican.xyz",
            "[zkill.top] Zombie CSO",
            "[DAG] Die Alte Garde | https://die-alte-gar.de",
            "Best server @discord.gg/xyz",
            "Bosna MW4\r",
        ] {
            let a = analyze(&sample(name), &Context::default());
            assert!(a.reasons.is_empty(), "{name:?} flagged: {:?}", a.reasons);
        }
    }
}
