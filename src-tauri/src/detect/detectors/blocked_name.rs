//! Hostnames the user never wants listed.

use crate::detect::{Detector, Reason, Subject};

/// Hostnames the user never wants listed, matched case-insensitively
/// anywhere in the name.
///
/// `FASTCUP.NET` runs matchmaking servers (15 on the live list, one per IP in
/// 91.202.247.0/24): joinable only through that platform's own client, so
/// they are noise in a public browser — and no farm rule can catch them.
pub const BLOCKED_NAMES: &[&str] = &["fastcup.net"];

pub struct BlockedName;

impl Detector for BlockedName {
    fn reason(&self) -> Reason {
        Reason::BlockedName
    }

    fn fires(&self, s: &Subject<'_>) -> bool {
        BLOCKED_NAMES.iter().any(|b| s.name_lower.contains(b))
    }
}

#[cfg(test)]
mod tests {
    use crate::detect::testing::sample;
    use crate::detect::{analyze, Context, Reason};

    #[test]
    fn fastcup_servers_are_skipped() {
        for name in [
            "FASTCUP.NET",
            "FASTCUP.NET | CSDM MOSCOW #2",
            "fastcup.net public",
        ] {
            let a = analyze(&sample(name), &Context::default());
            assert!(
                a.reasons.contains(&Reason::BlockedName) && a.is_fake(),
                "{name}"
            );
        }
        assert!(!Reason::label_bans_persistently("blocklist"));
    }
}
