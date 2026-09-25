//! Every slot is a bot.

use crate::detect::{Detector, Reason, Subject};

pub struct BotFlood;

impl Detector for BotFlood {
    fn reason(&self) -> Reason {
        Reason::FullOfBots
    }

    fn fires(&self, s: &Subject<'_>) -> bool {
        let i = s.info;
        i.max_players > 0 && i.bots as u16 >= i.max_players as u16
    }
}

#[cfg(test)]
mod tests {
    use crate::detect::testing::sample;
    use crate::detect::{analyze, Context, Reason};

    #[test]
    fn bot_flood_is_soft() {
        let mut s = sample("OK");
        s.players = 32;
        s.max_players = 32;
        s.bots = 32;
        let a = analyze(&s, &Context::default());
        assert!(a.reasons.contains(&Reason::FullOfBots));
        assert!(!a.is_fake()); // soft only
    }
}
