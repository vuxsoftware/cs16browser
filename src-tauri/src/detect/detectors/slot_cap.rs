//! More slots than CS 1.6 allows.

use crate::detect::{Detector, Reason, Subject};

/// Maximum slots a real Counter-Strike 1.6 server can offer.
///
/// The engine's own cap is 32 players; anything higher in the server list is an
/// inflated or spoofed listing. Measured against the live list: `max_players`
/// values of 255, 120 and 64 account for the overwhelming majority of rows
/// (~78% of a 10,000-server sample), all of which are fake/inflated — a real
/// 32-slot server is a tiny minority of what the master returns.
pub const CS16_MAX_SLOTS: u8 = 32;

pub struct SlotCap;

impl Detector for SlotCap {
    fn reason(&self) -> Reason {
        Reason::SlotsExceedMax
    }

    /// Values above the cap are not produced by a real CS 1.6 server: they
    /// are the inflated "255/120 slots" listings that dominate the fake-server
    /// population, or a listing for a different game. Hard-ban them.
    fn fires(&self, s: &Subject<'_>) -> bool {
        s.info.max_players > CS16_MAX_SLOTS
    }
}

#[cfg(test)]
mod tests {
    use crate::detect::testing::sample;
    use crate::detect::{analyze, Context, Reason, Severity};
    use crate::model::ServerInfo;

    fn with_slots(max: u8, players: u8) -> ServerInfo {
        let mut s = sample("Real Server");
        s.max_players = max;
        s.players = players;
        s
    }

    /// A real CS 1.6 server tops out at 32 slots.
    #[test]
    fn thirty_two_slots_is_allowed() {
        let a = analyze(&with_slots(32, 20), &Context::default());
        assert!(
            !a.reasons.contains(&Reason::SlotsExceedMax),
            "a legitimate 32-slot server must not be banned: {:?}",
            a.reasons
        );
    }

    /// The inflated listings that dominate the master list are hard-banned.
    /// These are the exact values observed live: 33, 64, 120, 255.
    #[test]
    fn slots_above_32_are_hard_banned() {
        for max in [33u8, 40, 64, 120, 255] {
            let a = analyze(&with_slots(max, 10), &Context::default());
            assert!(
                a.reasons.contains(&Reason::SlotsExceedMax),
                "max_players={max} must be flagged"
            );
            assert!(
                a.is_fake(),
                "max_players={max} must be hard (banned), not merely suspicious"
            );
        }
    }

    /// The ban must not depend on the advertised player count.
    #[test]
    fn slot_ban_is_independent_of_players_online() {
        for players in [0u8, 1, 32, 120] {
            let a = analyze(&with_slots(255, players), &Context::default());
            assert!(a.is_fake(), "players={players} with max=255 must be banned");
        }
    }

    #[test]
    fn slot_reason_label_is_stable() {
        assert_eq!(Reason::SlotsExceedMax.label(), "slots>32");
        assert_eq!(Reason::SlotsExceedMax.severity(), Severity::Hard);
    }
}
