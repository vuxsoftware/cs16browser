//! Missing map name.

use crate::detect::{Detector, Reason, Subject};

pub struct EmptyMap;

impl Detector for EmptyMap {
    fn reason(&self) -> Reason {
        Reason::MapEmpty
    }

    fn fires(&self, s: &Subject<'_>) -> bool {
        s.info.map.is_empty()
    }
}
