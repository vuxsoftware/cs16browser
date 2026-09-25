//! Missing gamedir.

use crate::detect::{Detector, Reason, Subject};

pub struct EmptyGameDir;

impl Detector for EmptyGameDir {
    fn reason(&self) -> Reason {
        Reason::GameDirEmpty
    }

    fn fires(&self, s: &Subject<'_>) -> bool {
        s.info.gamedir.is_empty()
    }
}
