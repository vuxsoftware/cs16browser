//! Cross-run server-name history, for detecting identity churn.
//!
//! ## The signal
//!
//! Fake / redirect servers rebrand constantly: the same endpoint advertises a
//! completely different identity from one scan to the next. A legitimate server
//! does not — and the one case that *looks* like a rename is not one:
//!
//! ```text
//! "GameServerName [20:00]"  ->  "GameServerName [20:01]"    NOT churn
//! "Dust2 Paradise"          ->  "Zombie Survival 24/7"       churn
//! ```
//!
//! The first pair differs only by a clock the server keeps in its name. So the
//! comparison happens on a *normalised* form that drops volatile decoration
//! (clock/counter brackets, bare numbers) before measuring similarity. Only a
//! low remaining similarity counts as a complete change.
//!
//! ## Persistence
//!
//! Churn is only observable across runs, so observations live inside the
//! combined state file (see `app::State`). Each endpoint keeps its few most
//! recent distinct names plus counters, keyed by `ip:port` so repeat sightings
//! of the same server update one entry rather than adding new ones. Entries
//! are pruned by age (not by count) on every save.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Similarity below which a name change counts as "complete".
///
/// Token-set Jaccard. 0.35 keeps cosmetic edits together (shuffled tags, an
/// added `[EU]`) while separating genuinely different identities. A pure clock
/// change normalises to identical names and scores 1.0, so it never trips this.
pub const NAME_CHANGE_THRESHOLD: f64 = 0.35;

/// Number of *complete* changes at which churn becomes a hard ban.
///
/// One rename is ordinary administration; repeatedly adopting a whole new
/// identity is what evasive servers do. Kept as a named constant so the rule is
/// visible rather than buried in a comparison.
pub const NAME_CHURN_HARD_AT: u32 = 3;

/// How many recent distinct names to remember per endpoint.
const MAX_NAMES_PER_ENDPOINT: usize = 6;

/// Entries not seen within this many days are pruned on save.
const RETENTION_DAYS: i64 = 30;

/// What we know about one endpoint's naming.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EndpointNames {
    /// Recent *raw* names, most recent last. Distinct by normalised form.
    pub names: Vec<String>,
    /// Normalised form of the most recent name, used for comparison.
    #[serde(default)]
    pub last_normalized: String,
    /// Times the name changed *completely* (not merely decoratively).
    #[serde(default)]
    pub major_changes: u32,
    /// Total observations recorded.
    #[serde(default)]
    pub observations: u32,
    /// Unix seconds of the last observation.
    #[serde(default)]
    pub last_seen: i64,
}

/// Churn verdict for one endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Churn {
    /// Count of complete name changes seen so far.
    pub major_changes: u32,
    /// Whether *this* observation was itself a complete change.
    pub changed_now: bool,
}

impl Churn {
    /// No history, or a stable name.
    pub fn is_clean(&self) -> bool {
        self.major_changes == 0
    }
}

/// Persistent name history, keyed by `ip:port`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NameHistory {
    /// Schema version, so an incompatible file can be discarded safely.
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub entries: HashMap<String, EndpointNames>,
}

fn default_version() -> u32 {
    1
}

/// Normalise a display name for churn comparison.
///
/// Strips the volatile parts a server bakes into its name so that they cannot
/// masquerade as a rename:
///
/// * bracketed groups containing **no letters** — `[20:00]`, `(1)`, `[#3]`,
///   `[12/32]` — are dropped entirely. Bracketed *words* (`[FFA]`, `[EU]`) are
///   kept, because those are identity.
/// * non-alphanumerics become spaces, so punctuation/formatting is irrelevant.
/// * tokens that are **entirely digits** are dropped (`Server 12` == `Server`).
/// * the result is lowercased with duplicate tokens collapsed.
///
/// Intra-token digits are preserved: `dust2` and `dust3` stay distinct, since
/// that difference is usually meaningful.
pub fn normalize_name(raw: &str) -> String {
    let lower = raw.to_lowercase();

    // Pass 1: fold bracketed groups, keeping only those with letters.
    let mut unfolded = String::with_capacity(lower.len());
    let mut group = String::new();
    let mut depth = 0u32;
    for ch in lower.chars() {
        match ch {
            '(' | '[' | '{' => {
                if depth == 0 {
                    group.clear();
                } else {
                    // Nested brackets are unusual; treat content as plain.
                    group.push(ch);
                }
                depth += 1;
            }
            ')' | ']' | '}' => {
                if depth > 0 {
                    depth -= 1;
                    if depth == 0 {
                        if group.chars().any(|c| c.is_alphabetic()) {
                            unfolded.push(' ');
                            unfolded.push_str(&group);
                        }
                        group.clear();
                    } else {
                        group.push(ch);
                    }
                }
            }
            _ if depth > 0 => group.push(ch),
            _ => unfolded.push(ch),
        }
    }
    // An unterminated bracket should not swallow the rest of the name.
    if !group.is_empty() {
        unfolded.push(' ');
        unfolded.push_str(&group);
    }

    // Pass 2: alphanumerics only, then drop pure-digit tokens.
    let cleaned: String = unfolded
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { ' ' })
        .collect();

    let mut seen: Vec<&str> = Vec::new();
    for token in cleaned.split_whitespace() {
        if token.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }
        if !seen.contains(&token) {
            seen.push(token);
        }
    }
    seen.join(" ")
}

/// Token-set Jaccard similarity of two **normalised** names, in `0.0..=1.0`.
pub fn similarity(a: &str, b: &str) -> f64 {
    let ta: std::collections::HashSet<&str> = a.split_whitespace().collect();
    let tb: std::collections::HashSet<&str> = b.split_whitespace().collect();
    match (ta.is_empty(), tb.is_empty()) {
        (true, true) => 1.0,
        // A name with nothing comparable left is not evidence of a rename.
        (true, false) | (false, true) => 1.0,
        _ => {
            let inter = ta.intersection(&tb).count() as f64;
            let union = ta.union(&tb).count() as f64;
            inter / union
        }
    }
}

/// Is the move from `previous` to `current` a *complete* identity change?
///
/// Both sides are raw names; normalisation happens here. Cosmetic differences
/// (including clock suffixes) return `false`.
pub fn is_complete_change(previous: &str, current: &str) -> bool {
    let a = normalize_name(previous);
    let b = normalize_name(current);
    // Nothing to compare against: not evidence of churn.
    if a.is_empty() || b.is_empty() {
        return false;
    }
    similarity(&a, &b) < NAME_CHANGE_THRESHOLD
}

impl NameHistory {
    /// Record one observation and return the churn state for that endpoint.
    ///
    /// The first observation never counts as a change (there is nothing to
    /// compare against), which is what makes `major_changes` mean "changes
    /// since we started watching" rather than "observations".
    pub fn observe(&mut self, endpoint: &str, name: &str, now: i64) -> Churn {
        let normalized = normalize_name(name);
        let entry = self.entries.entry(endpoint.to_string()).or_default();

        let changed_now = if entry.observations == 0 {
            false
        } else {
            !entry.last_normalized.is_empty()
                && is_complete_change(&entry.last_normalized, &normalized)
        };

        if changed_now {
            entry.major_changes = entry.major_changes.saturating_add(1);
        }

        // Remember the raw name when it is a *new* normalised form, so a
        // clock-only change does not flush the real name out of the window.
        let already = entry.names.iter().any(|n| normalize_name(n) == normalized);
        if !already && !normalized.is_empty() {
            entry.names.push(name.to_string());
            if entry.names.len() > MAX_NAMES_PER_ENDPOINT {
                let drop = entry.names.len() - MAX_NAMES_PER_ENDPOINT;
                entry.names.drain(0..drop);
            }
        }

        if !normalized.is_empty() {
            entry.last_normalized = normalized;
        }
        entry.observations = entry.observations.saturating_add(1);
        entry.last_seen = now;

        Churn {
            major_changes: entry.major_changes,
            changed_now,
        }
    }

    /// Churn already recorded for an endpoint, without observing.
    pub fn churn_for(&self, endpoint: &str) -> Churn {
        self.entries
            .get(endpoint)
            .map(|e| Churn {
                major_changes: e.major_changes,
                changed_now: false,
            })
            .unwrap_or_default()
    }

    /// Distinct names recorded for an endpoint, oldest first.
    pub fn names_for(&self, endpoint: &str) -> &[String] {
        self.entries
            .get(endpoint)
            .map(|e| e.names.as_slice())
            .unwrap_or(&[])
    }

    /// Drop entries not seen within [`RETENTION_DAYS`]. Every endpoint kept is
    /// keyed by `ip:port`, so repeat observations of the same server are
    /// merged in place rather than piling up new rows.
    pub fn prune(&mut self) {
        let cutoff = now_unix() - RETENTION_DAYS * 86_400;
        self.entries.retain(|_, e| e.last_seen >= cutoff);
    }
}

/// Current unix time in seconds.
pub fn now_unix() -> i64 {
    chrono::Utc::now().timestamp()
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- normalisation: the timer case the user cares about ----------------

    /// A clock kept in the server name must not read as a rename.
    #[test]
    fn clock_suffix_is_not_a_change() {
        assert!(!is_complete_change(
            "GameServerName [20:00]",
            "GameServerName [20:01]"
        ));
        // ...and the same across the hour/day boundary.
        assert!(!is_complete_change("Serv [23:59]", "Serv [00:00]"));
    }

    #[test]
    fn other_volatile_decorations_are_not_changes() {
        for (a, b) in [
            ("Deathmatch (1)", "Deathmatch (2)"),
            ("Server [#3]", "Server [#17]"),
            ("Arena [12/32]", "Arena [31/32]"),
            ("Party 100", "Party 250"),
            ("Frag Zone [20:00]", "Frag Zone [21:30]"),
        ] {
            assert!(
                !is_complete_change(a, b),
                "{a:?} vs {b:?} must not count as a complete change"
            );
        }
    }

    /// Bracketed *words* are identity and must survive normalisation.
    #[test]
    fn semantic_brackets_are_kept() {
        assert!(normalize_name("[FFA] Dust2").contains("ffa"));
        assert!(normalize_name("Server (EU)").contains("eu"));
        // ...so a real rename that swaps them is still detected.
        assert!(is_complete_change(
            "[ZOMBIE] Infected Land",
            "[AWP] Sniper Paradise"
        ));
    }

    // ---- complete changes -------------------------------------------------

    #[test]
    fn whole_name_change_is_detected() {
        assert!(is_complete_change("Dust2 Paradise", "Zombie Survival 24/7"));
        assert!(is_complete_change(
            "CS 1.6 Server",
            "Best Headshot Practice Arena"
        ));
    }

    /// Cosmetic edits are not identity changes.
    #[test]
    fn cosmetic_edits_are_not_changes() {
        assert!(!is_complete_change(
            "Dust2 Only | Free VIP",
            "Dust2 Only | Free VIP & FastDL"
        ));
        assert!(!is_complete_change("Frag Zone", "FRAG ZONE"));
        assert!(!is_complete_change("Frag  Zone", "frag zone "));
    }

    /// Nothing to compare against must never be reported as churn.
    #[test]
    fn empty_names_are_not_changes() {
        assert!(!is_complete_change("", "Anything"));
        assert!(!is_complete_change("Anything", ""));
        assert!(!is_complete_change("   ", "   "));
        assert!(!is_complete_change("[20:00]", "Real Server"));
    }

    #[test]
    fn similarity_is_symmetric_and_bounded() {
        let a = normalize_name("Dust2 Only | Free VIP");
        let b = normalize_name("Zombie Survival");
        assert!((similarity(&a, &b) - similarity(&b, &a)).abs() < f64::EPSILON);
        assert!((0.0..=1.0).contains(&similarity(&a, &b)));
        assert_eq!(similarity(&a, &a), 1.0);
    }

    // ---- history lifecycle ------------------------------------------------

    /// The first observation establishes a baseline and is not a change.
    #[test]
    fn first_observation_is_not_a_change() {
        let mut h = NameHistory::default();
        let c = h.observe("1.2.3.4:27015", "Dust2 Paradise", 1000);
        assert!(!c.changed_now);
        assert_eq!(c.major_changes, 0);
        assert_eq!(h.names_for("1.2.3.4:27015").len(), 1);
    }

    /// A clock update is recorded as an observation but never a change, and it
    /// must not churn the stored name list either.
    #[test]
    fn clock_updates_do_not_accumulate_changes() {
        let mut h = NameHistory::default();
        h.observe("1.2.3.4:27015", "GameServerName [20:00]", 1000);
        for minute in 1..10 {
            let c = h.observe(
                "1.2.3.4:27015",
                &format!("GameServerName [20:{minute:02}]"),
                1000 + minute,
            );
            assert!(!c.changed_now, "minute {minute} must not be a change");
        }
        assert_eq!(h.churn_for("1.2.3.4:27015").major_changes, 0);
        // Only the original spelling is remembered.
        assert_eq!(h.names_for("1.2.3.4:27015").len(), 1);
        // But every poll was counted as an observation.
        assert_eq!(h.entries["1.2.3.4:27015"].observations, 10);
    }

    /// Repeated whole-name changes accumulate and eventually reach the ban
    /// threshold.
    #[test]
    fn repeated_renames_accumulate_to_the_hard_threshold() {
        let mut h = NameHistory::default();
        h.observe("1.2.3.4:27015", "Dust2 Paradise", 1);
        h.observe("1.2.3.4:27015", "Zombie Survival 24/7", 2);
        h.observe("1.2.3.4:27015", "Headshot Practice Arena", 3);
        let c = h.observe("1.2.3.4:27015", "FastDL Deathmatch City", 4);
        assert!(c.changed_now);
        assert_eq!(c.major_changes, NAME_CHURN_HARD_AT);
        assert!(!c.is_clean());
        // All four identities are retained for inspection.
        assert_eq!(h.names_for("1.2.3.4:27015").len(), 4);
    }

    /// Endpoints are tracked independently.
    #[test]
    fn history_is_per_endpoint() {
        let mut h = NameHistory::default();
        h.observe("1.1.1.1:27015", "Alpha", 1);
        h.observe("2.2.2.2:27015", "Beta", 1);
        assert!(h.churn_for("1.1.1.1:27015").is_clean());
        assert!(h.churn_for("2.2.2.2:27015").is_clean());
        assert!(h.churn_for("3.3.3.3:27015").is_clean(), "unknown endpoint");
    }

    #[test]
    fn prunes_stale_entries() {
        let mut h = NameHistory::default();
        let old = now_unix() - (RETENTION_DAYS + 1) * 86_400;
        h.entries.insert(
            "9.9.9.9:27015".into(),
            EndpointNames {
                names: vec!["Old".into()],
                last_normalized: "old".into(),
                observations: 1,
                last_seen: old,
                ..Default::default()
            },
        );
        h.observe("1.2.3.4:27015", "Fresh", now_unix());
        h.prune();
        assert!(
            !h.entries.contains_key("9.9.9.9:27015"),
            "stale must be pruned"
        );
        assert!(
            h.entries.contains_key("1.2.3.4:27015"),
            "fresh must survive"
        );
    }

    /// The stored name window stays bounded.
    #[test]
    fn name_window_is_bounded() {
        let mut h = NameHistory::default();
        for i in 0..20 {
            h.observe(
                "1.2.3.4:27015",
                &format!("Distinct Identity Number {i} Alpha"),
                i,
            );
        }
        assert!(h.names_for("1.2.3.4:27015").len() <= MAX_NAMES_PER_ENDPOINT);
    }

    /// An empty name from a broken server must not reset a tracked identity.
    #[test]
    fn empty_name_does_not_reset_history() {
        let mut h = NameHistory::default();
        h.observe("1.2.3.4:27015", "Dust2 Paradise", 1);
        let c = h.observe("1.2.3.4:27015", "", 2);
        assert!(!c.changed_now, "an empty name is not a rename");
        // The real name is still what we compare against next time.
        let c2 = h.observe("1.2.3.4:27015", "Zombie Survival Arena", 3);
        assert!(
            c2.changed_now,
            "comparison must use the last non-empty name"
        );
    }
}
