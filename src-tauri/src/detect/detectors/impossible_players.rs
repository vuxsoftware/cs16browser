//! The player counts cannot be true.

use crate::detect::{Detector, Reason, Subject, CS16_MAX_SLOTS};

pub struct ImpossiblePlayers;

impl Detector for ImpossiblePlayers {
    fn reason(&self) -> Reason {
        Reason::PlayersExceedMax
    }

    /// More players than the engine can hold, or no slots at all.
    ///
    /// Not `players > max_players`: reserved-slot plugins lower
    /// `sv_visiblemaxplayers`, so a full real server with an admin in the
    /// hidden slot reports `32/31`. Measured, every listed row with more
    /// players than slots claimed 33–60 players, so the engine cap loses
    /// none of them.
    fn fires(&self, s: &Subject<'_>) -> bool {
        let i = s.info;
        i.max_players == 0 || i.players > CS16_MAX_SLOTS
    }
}

#[cfg(test)]
mod tests {
    use crate::detect::testing::sample;
    use crate::detect::{analyze, Context, Reason};

    #[test]
    fn players_above_the_engine_cap_is_hard() {
        let mut s = sample("OK");
        s.players = 44;
        s.max_players = 32;
        let a = analyze(&s, &Context::default());
        assert!(a.reasons.contains(&Reason::PlayersExceedMax));
        assert!(a.is_fake());
    }

    #[test]
    fn a_reserved_slot_overflow_is_fine() {
        let mut s = sample("OK");
        s.players = 32;
        s.max_players = 31;
        let a = analyze(&s, &Context::default());
        assert!(a.reasons.is_empty(), "{:?}", a.reasons);
    }

    #[test]
    fn zero_slots_is_impossible() {
        let mut s = sample("OK");
        s.players = 0;
        s.max_players = 0;
        let a = analyze(&s, &Context::default());
        assert!(a.reasons.contains(&Reason::PlayersExceedMax));
    }
}
