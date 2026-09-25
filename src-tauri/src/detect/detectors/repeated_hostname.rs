//! The same hostname on many other servers (botnet).

use crate::detect::{Detector, Reason, Subject};

/// Other servers sharing the name before it counts.
const MIN_REPEATS: usize = 5;

pub struct RepeatedHostname;

impl Detector for RepeatedHostname {
    fn reason(&self) -> Reason {
        Reason::RepeatedHostname
    }

    fn fires(&self, s: &Subject<'_>) -> bool {
        s.ctx.hostname_repeat_count >= MIN_REPEATS
    }
}
