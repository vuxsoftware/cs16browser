//! The contract every heuristic implements.

use super::{Context, Reason};
use crate::model::ServerInfo;

/// One fake-server heuristic.
///
/// A detector is a stateless unit struct that raises exactly one [`Reason`].
/// Its thresholds and pattern lists live next to it in its own file.
pub trait Detector: Sync {
    /// The reason this detector raises. No two detectors share one.
    fn reason(&self) -> Reason;

    /// Whether the server shows this detector's signal.
    fn fires(&self, s: &Subject<'_>) -> bool;
}

/// What a detector looks at: one server plus the caller's broader context.
pub struct Subject<'a> {
    pub info: &'a ServerInfo,
    pub ctx: &'a Context,
    /// `info.hostname` lower-cased once for all the text rules.
    pub name_lower: String,
}

impl<'a> Subject<'a> {
    pub fn new(info: &'a ServerInfo, ctx: &'a Context) -> Self {
        Self {
            info,
            ctx,
            name_lower: info.hostname.to_ascii_lowercase(),
        }
    }
}
