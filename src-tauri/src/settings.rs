//! User settings edited from the UI (the Settings dialog).
//!
//! Stored as `settings.json` in the data directory (beside the discovery
//! cache), written atomically. Only the Steam Web API key lives here today,
//! and this is its **only** source: no environment variable or `.env` file is
//! read, so the key the dialog shows is the key the app uses.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const SETTINGS_FILE: &str = "settings.json";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Write `debug`-level entries to the log file, not just `info`/`warn`/
    /// `error`. Off by default: debug logging is a diagnostic aid, not
    /// something every session should pay the extra file I/O for.
    #[serde(default)]
    pub debug_logs: bool,
}

impl Settings {
    pub fn path() -> PathBuf {
        crate::store::store_dir().join(SETTINGS_FILE)
    }

    /// Tolerant load: a missing or corrupt file yields defaults, never an error.
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default()
    }

    /// Write atomically (tmp + rename), creating the data directory if needed.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        }
        let json = serde_json::to_string_pretty(self).map_err(|e| format!("serialize: {e}"))?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(|e| format!("write {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("rename to {}: {e}", path.display())
        })
    }
}

/// Normalise and validate a key typed into the UI.
///
/// Steam Web API keys are 32 hexadecimal characters. Checking the shape here
/// catches a pasted URL or a truncated key immediately, instead of after a
/// metered request comes back `403`. An empty string means "clear the key".
pub fn normalize_api_key(input: &str) -> Result<Option<String>, String> {
    let key = input.trim();
    if key.is_empty() {
        return Ok(None);
    }
    if key.len() != 32 || !key.chars().all(|c| c.is_ascii_hexdigit()) {
        return Err(
            "A Steam Web API key is 32 characters of 0-9 and A-F. Copy the whole \
             \"Key\" value from steamcommunity.com/dev/apikey."
                .into(),
        );
    }
    Ok(Some(key.to_ascii_uppercase()))
}

/// `••••••••1A2B`: enough to recognise a key without displaying it.
pub fn mask_key(key: &str) -> String {
    let tail: String = key
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("••••••••{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_a_well_formed_key_and_uppercases_it() {
        let k = normalize_api_key("  0123456789abcdef0123456789ABCDEF \n").unwrap();
        assert_eq!(k.as_deref(), Some("0123456789ABCDEF0123456789ABCDEF"));
    }

    #[test]
    fn empty_input_clears_the_key() {
        assert_eq!(normalize_api_key("   ").unwrap(), None);
    }

    #[test]
    fn rejects_malformed_keys() {
        for bad in [
            "short",
            "0123456789ABCDEF0123456789ABCDEFAA",
            "https://steamcommunity.com/dev/apikey",
            "0123456789ABCDEF0123456789ABCDEZ",
        ] {
            assert!(normalize_api_key(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn mask_shows_only_the_last_four() {
        assert_eq!(mask_key("0123456789ABCDEF0123456789ABCDEF"), "••••••••CDEF");
    }

    #[test]
    fn round_trips_and_tolerates_a_corrupt_file() {
        let dir = std::env::temp_dir().join(format!("cs16-settings-{}", std::process::id()));
        let path = dir.join(SETTINGS_FILE);
        let s = Settings {
            api_key: Some("0123456789ABCDEF0123456789ABCDEF".into()),
            debug_logs: true,
        };
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path), s);
        std::fs::write(&path, "{not json").unwrap();
        assert_eq!(Settings::load(&path), Settings::default());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
