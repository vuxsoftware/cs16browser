//! Verdicts: why a server is flagged, its stable label and its severity.

use serde::{Deserialize, Serialize};

/// Why a server is flagged.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Reason {
    /// Hostname is empty or only whitespace.
    HostnameEmpty,
    /// More players than a CS 1.6 server can hold, or no slots at all.
    PlayersExceedMax,
    /// Map name is missing/empty.
    MapEmpty,
    /// gamedir is empty.
    GameDirEmpty,
    /// All slots are bots.
    FullOfBots,
    /// This hostname is repeated across many other servers (botnet).
    RepeatedHostname,
    /// max_players exceeds what CS 1.6 permits (32).
    ///
    /// GoldSrc reports player counts as single bytes, so a larger value cannot
    /// come from a real CS 1.6 server — it is either a spoofed/scam listing or
    /// a server for a different game pretending to be one.
    SlotsExceedMax,
    /// The hostname advertises a *different* server's IPv4 address
    /// ("Connect to 5.6.7.8:27015"): the listing is a signpost to somewhere
    /// else — the classic redirect.
    ForeignAddress,
    /// The hostname is on [`BLOCKED_NAMES`](super::BLOCKED_NAMES).
    BlockedName,
    /// The server's IP is a redirect / spam farm (see
    /// [`farm_ips`](super::farm_ips)). Needs the whole listing, so the caller
    /// supplies the verdict.
    ServerFarm,
    /// The endpoint has repeatedly adopted a completely different name.
    ///
    /// Cosmetic edits and clock suffixes (`Name [20:00]`) are explicitly *not*
    /// counted — see `history::is_complete_change`. Requires cross-run history.
    NameChurn,
}

impl Reason {
    /// Every reason, for label lookups.
    pub const ALL: [Reason; 11] = [
        Reason::HostnameEmpty,
        Reason::PlayersExceedMax,
        Reason::MapEmpty,
        Reason::GameDirEmpty,
        Reason::FullOfBots,
        Reason::RepeatedHostname,
        Reason::SlotsExceedMax,
        Reason::ForeignAddress,
        Reason::BlockedName,
        Reason::ServerFarm,
        Reason::NameChurn,
    ];

    /// Whether a stored ban reason still justifies a *persistent* ban under
    /// today's rules.
    ///
    /// Bans are saved as labels, so a rule later relaxed (from hard to soft)
    /// would otherwise keep its old victims banned — and a banned server is
    /// never queried again, so it could never prove itself. `server-farm` is
    /// not persistent either: it is re-derived from the listing every sweep,
    /// and an IP stops being a farm when its fake ports go away.
    pub fn label_bans_persistently(label: &str) -> bool {
        Reason::ALL
            .iter()
            .find(|r| r.label() == label)
            .is_some_and(|r| {
                // Both are re-derived from the listing every sweep: an IP stops
                // being a farm, a server can be renamed off the blocklist.
                r.severity().is_hard() && !matches!(r, Reason::ServerFarm | Reason::BlockedName)
            })
    }

    pub fn label(&self) -> &'static str {
        match self {
            Reason::ForeignAddress => "foreign-ip",
            Reason::BlockedName => "blocklist",
            Reason::HostnameEmpty => "empty-hostname",
            // Renamed from `players>max` when the rule stopped firing on
            // reserved-slot servers: the old label no longer bans persistently,
            // so bans it left behind are lifted on load.
            Reason::PlayersExceedMax => "players>32",
            Reason::MapEmpty => "empty-map",
            Reason::GameDirEmpty => "empty-gamedir",
            Reason::FullOfBots => "all-bots",
            Reason::RepeatedHostname => "duplicate-name",
            Reason::SlotsExceedMax => "slots>32",
            Reason::ServerFarm => "server-farm",
            Reason::NameChurn => "name-churn",
        }
    }

    pub fn severity(&self) -> Severity {
        match self {
            Reason::PlayersExceedMax => Severity::Hard,
            // Pointing at another server's address *is* a redirect.
            Reason::ForeignAddress => Severity::Hard,
            Reason::BlockedName => Severity::Hard,
            Reason::HostnameEmpty | Reason::MapEmpty | Reason::GameDirEmpty => Severity::Hard,
            Reason::FullOfBots => Severity::Soft,
            Reason::RepeatedHostname => Severity::Soft,
            Reason::SlotsExceedMax => Severity::Hard,
            Reason::ServerFarm => Severity::Hard,
            // Repeatedly adopting a whole new identity is what evasive servers
            // do; one or two renames is ordinary administration, so the
            // detector only raises this hard once the count reaches the
            // threshold (see `history::NAME_CHURN_HARD_AT`).
            Reason::NameChurn => Severity::Hard,
        }
    }
}

/// Hard = definite fake; Soft = suspicious, downrank.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Soft,
    Hard,
}

impl Severity {
    pub fn is_hard(&self) -> bool {
        matches!(self, Severity::Hard)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_current_hard_non_farm_reasons_ban_persistently() {
        assert!(Reason::label_bans_persistently("slots>32"));
        assert!(Reason::label_bans_persistently("foreign-ip"));
        assert!(Reason::label_bans_persistently("players>32"));
        // Relaxed rules, retired detectors and listing-derived verdicts do not.
        for label in [
            "redirect-hint",
            "control-chars",
            "url-spam",
            "server-farm",
            "ping>800",
            "players>max",
            "wrong-engine",
            "tcp-only",
            "nope",
        ] {
            assert!(!Reason::label_bans_persistently(label), "{label}");
        }
        // Every variant is listed, so a label lookup cannot silently miss one.
        let labels: std::collections::HashSet<&str> =
            Reason::ALL.iter().map(|r| r.label()).collect();
        assert_eq!(labels.len(), Reason::ALL.len());
    }
}
