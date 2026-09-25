//! Repeated complete identity changes across runs.

use crate::detect::{Detector, Reason, Subject};

pub struct NameChurn;

impl Detector for NameChurn {
    fn reason(&self) -> Reason {
        Reason::NameChurn
    }

    /// Only once the endpoint has adopted a completely new name several
    /// times. One rename is normal; `is_complete_change` already excludes
    /// clock suffixes and cosmetic edits.
    fn fires(&self, s: &Subject<'_>) -> bool {
        s.ctx.name_major_changes >= crate::history::NAME_CHURN_HARD_AT
    }
}
