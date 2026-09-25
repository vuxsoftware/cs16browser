//! Empty or whitespace-only hostname.

use crate::detect::{Detector, Reason, Subject};

pub struct EmptyHostname;

impl Detector for EmptyHostname {
    fn reason(&self) -> Reason {
        Reason::HostnameEmpty
    }

    fn fires(&self, s: &Subject<'_>) -> bool {
        s.info.hostname.trim().is_empty()
    }
}
