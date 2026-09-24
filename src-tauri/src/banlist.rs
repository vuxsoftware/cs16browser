//! Hard bans and recheck scheduling for suspected servers.
//!
//! Two different responses to two different verdicts:
//!
//! | verdict | response |
//! |---|---|
//! | **hard** (`Analysis::is_fake`) | banned: never queried again, skipped without sending a packet, until a `--hard-refresh` clears the list |
//! | **soft** (suspicious only) | kept, and put on a [`RecheckQueue`] so it is re-queried later to decide which way it falls |
//!
//! This keeps a scan cheap where it should be (banning is permanent, so a
//! 20k-endpoint sweep is not spent re-probing known redirect farms) while still
//! giving a merely-suspicious server the chance to prove itself — a server that
//! was having a bad minute should not be blacklisted for it.
//!
//! Bans are deliberately manual to clear: `--hard-refresh`. Auto-expiring them
//! would let a server that earned a ban drift back into the list simply by
//! waiting.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// How long before a *repeat* recheck.
///
/// The **first** recheck is immediate (due on the next scan): a suspect should
/// be re-examined as soon as its name changes often enough to matter, and
/// waiting minutes would mean name-churn detection never fires within a
/// session. Later rechecks are spaced out so a server that keeps looking
/// suspicious is not hammered — and they are subject to the same per-endpoint
/// pacing limits as everything else.
pub const RECHECK_INTERVAL_SECS: i64 = 300;

/// A suspected server is rechecked at most this many times before it is left
/// alone. Without a cap, a permanently-degraded server would be re-queried
/// forever for no new information.
pub const MAX_RECHECKS: u32 = 3;

/// Why an endpoint is banned, and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BanEntry {
    /// The hostname as last seen before the ban, so the Banned tab can name
    /// the server even when it has since dropped out of Steam's listing.
    /// `#[serde(default)]` so a ban list written before this field existed
    /// still loads (as an empty name rather than a failed deserialize).
    #[serde(default)]
    pub hostname: String,
    /// Reason labels that triggered the ban, for display.
    #[serde(default)]
    pub reasons: Vec<String>,
    /// Unix seconds of the ban.
    #[serde(default)]
    pub banned_at: i64,
    /// How many times the endpoint was rechecked *before* being banned.
    #[serde(default)]
    pub rechecks: u32,
}

/// Endpoints that must never be queried again.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BanList {
    #[serde(default)]
    entries: HashMap<String, BanEntry>,
}

impl BanList {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Is this endpoint banned?
    pub fn contains(&self, endpoint: &str) -> bool {
        self.entries.contains_key(endpoint)
    }

    /// Ban details, if any.
    pub fn get(&self, endpoint: &str) -> Option<&BanEntry> {
        self.entries.get(endpoint)
    }

    /// Ban an endpoint. Re-banning keeps the original timestamp so the age of
    /// the ban stays meaningful, but refreshes the hostname: a server can
    /// rename itself between sightings, and the Banned tab should show the
    /// name it was last seen under.
    pub fn ban(&mut self, endpoint: &str, hostname: &str, reasons: Vec<String>, now: i64) -> bool {
        match self.entries.get_mut(endpoint) {
            Some(e) => {
                if !hostname.is_empty() {
                    e.hostname = hostname.to_string();
                }
                // Refresh the reason list but keep when it was first banned.
                if !reasons.is_empty() {
                    e.reasons = reasons;
                }
                false
            }
            None => {
                self.entries.insert(
                    endpoint.to_string(),
                    BanEntry {
                        hostname: hostname.to_string(),
                        reasons,
                        banned_at: now,
                        rechecks: 0,
                    },
                );
                true
            }
        }
    }

    /// Remove one ban.
    /// Keep only the bans `keep` accepts; returns how many were dropped.
    pub fn retain(&mut self, mut keep: impl FnMut(&str, &BanEntry) -> bool) -> usize {
        let before = self.entries.len();
        self.entries.retain(|ep, e| keep(ep, e));
        before - self.entries.len()
    }

    pub fn unban(&mut self, endpoint: &str) -> bool {
        self.entries.remove(endpoint).is_some()
    }

    /// Clear every ban. Used by `--hard-refresh`.
    pub fn clear(&mut self) -> usize {
        let n = self.entries.len();
        self.entries.clear();
        n
    }

    /// All banned endpoints, for a report.
    pub fn endpoints(&self) -> impl Iterator<Item = (&str, &BanEntry)> {
        self.entries.iter().map(|(k, v)| (k.as_str(), v))
    }
}

/// Endpoints to re-query because they are *suspected* but not banned.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RecheckQueue {
    /// endpoint -> earliest unix second at which it may be rechecked.
    #[serde(default)]
    due: HashMap<String, i64>,
    /// endpoint -> how many rechecks have already happened.
    #[serde(default)]
    counts: HashMap<String, u32>,
}

/// What happened when scheduling a recheck.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scheduled {
    /// Added to the queue.
    Added,
    /// Already queued; the earlier due time is kept.
    AlreadyQueued,
    /// Rechecked `MAX_RECHECKS` times already: stop.
    Exhausted,
}

impl RecheckQueue {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.due.len()
    }

    pub fn is_empty(&self) -> bool {
        self.due.is_empty()
    }

    /// Schedule (or leave scheduled) a recheck for a suspect.
    ///
    /// A suspect that has never been rechecked is due *immediately*, so it is
    /// re-examined on the next scan. Repeat rechecks wait
    /// [`RECHECK_INTERVAL_SECS`].
    pub fn schedule(&mut self, endpoint: &str, now: i64) -> Scheduled {
        let done = self.counts.get(endpoint).copied().unwrap_or(0);
        if done >= MAX_RECHECKS {
            return Scheduled::Exhausted;
        }
        if self.due.contains_key(endpoint) {
            return Scheduled::AlreadyQueued;
        }
        let delay = if done == 0 { 0 } else { RECHECK_INTERVAL_SECS };
        self.due.insert(endpoint.to_string(), now + delay);
        Scheduled::Added
    }

    /// Endpoints whose recheck is due at `now`, soonest first.
    ///
    /// Does not remove them: the caller re-queries and then calls
    /// [`Self::done`], so a recheck that fails to send is retried rather than
    /// silently consumed.
    pub fn due(&self, now: i64, limit: usize) -> Vec<String> {
        let mut out: Vec<(String, i64)> = self
            .due
            .iter()
            .filter(|(_, t)| **t <= now)
            .map(|(k, t)| (k.clone(), *t))
            .collect();
        out.sort_by_key(|(_, t)| *t);
        out.into_iter().take(limit).map(|(k, _)| k).collect()
    }

    /// Mark a recheck finished: dequeue and count it.
    ///
    /// The count is retained at [`MAX_RECHECKS`] so an exhausted endpoint stays
    /// recognisably exhausted rather than reporting zero rechecks.
    pub fn done(&mut self, endpoint: &str) {
        self.due.remove(endpoint);
        let c = self.counts.entry(endpoint.to_string()).or_insert(0);
        *c = (*c).saturating_add(1).min(MAX_RECHECKS);
    }

    /// Whether an endpoint is currently queued.
    pub fn is_queued(&self, endpoint: &str) -> bool {
        self.due.contains_key(endpoint)
    }

    /// Forget an endpoint entirely (e.g. it became clean or was banned).
    pub fn remove(&mut self, endpoint: &str) {
        self.due.remove(endpoint);
        self.counts.remove(endpoint);
    }

    /// Rechecks performed so far.
    pub fn rechecks_done(&self, endpoint: &str) -> u32 {
        self.counts.get(endpoint).copied().unwrap_or(0)
    }

    /// Drop endpoints that are neither queued nor holding a recheck count.
    ///
    /// An endpoint that is merely not-queued with a live count is kept: that
    /// count is what enforces the recheck cap.
    pub fn prune(&mut self) {
        self.due.retain(|_, t| *t > 0);
        self.counts.retain(|_, c| *c > 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EP: &str = "203.0.113.7:27015";

    // ---- ban list ---------------------------------------------------------

    #[test]
    fn ban_records_reason_and_time() {
        let mut b = BanList::new();
        assert!(b.ban(EP, "Server", vec!["slots>32".into()], 1_000));
        assert!(b.contains(EP));
        let e = b.get(EP).unwrap();
        assert_eq!(e.reasons, vec!["slots>32".to_string()]);
        assert_eq!(e.banned_at, 1_000);
    }

    #[test]
    fn banning_twice_keeps_the_original_timestamp() {
        let mut b = BanList::new();
        b.ban(EP, "Server", vec!["slots>32".into()], 1_000);
        assert!(
            !b.ban(EP, "Server", vec!["name-churn".into()], 5_000),
            "not a new ban"
        );
        let e = b.get(EP).unwrap();
        assert_eq!(e.banned_at, 1_000, "age of the ban must stay meaningful");
        assert_eq!(e.reasons, vec!["name-churn".to_string()], "reasons update");
    }

    /// Re-banning an already-known endpoint must never grow the list: the
    /// map is keyed by endpoint (`ip:port`), so a repeat sighting of the same
    /// server is a no-op, not a new row.
    #[test]
    fn rebanning_the_same_endpoint_does_not_grow_the_list() {
        let mut b = BanList::new();
        for t in 0..50 {
            b.ban(EP, "Server", vec!["slots>32".into()], t);
        }
        assert_eq!(b.len(), 1);
    }

    #[test]
    fn clear_removes_every_ban() {
        let mut b = BanList::new();
        b.ban("1.1.1.1:27015", "Server", vec![], 1);
        b.ban("2.2.2.2:27015", "Server", vec![], 1);
        assert_eq!(b.clear(), 2);
        assert!(b.is_empty());
        assert!(!b.contains("1.1.1.1:27015"));
    }

    #[test]
    fn unban_removes_one() {
        let mut b = BanList::new();
        b.ban("1.1.1.1:27015", "Server", vec![], 1);
        b.ban("2.2.2.2:27015", "Server", vec![], 1);
        assert!(b.unban("1.1.1.1:27015"));
        assert_eq!(b.len(), 1);
        assert!(!b.unban("3.3.3.3:27015"), "unknown endpoint");
    }

    #[test]
    fn bans_survive_json() {
        let mut b = BanList::new();
        b.ban(EP, "Cloned CS 1.6", vec!["slots>32".into()], 1_000);
        let json = serde_json::to_string(&b).unwrap();
        let back: BanList = serde_json::from_str(&json).unwrap();
        assert!(back.contains(EP), "a ban must persist across runs");
        assert_eq!(back.get(EP).unwrap().banned_at, 1_000);
        assert_eq!(back.get(EP).unwrap().hostname, "Cloned CS 1.6");
    }

    /// A ban list written before `hostname` existed must still load, as an
    /// empty name rather than a failed deserialize.
    #[test]
    fn ban_without_hostname_field_still_loads() {
        let json = format!(r#"{{"entries":{{"{EP}":{{"reasons":["slots>32"],"banned_at":1000}}}}}}"#);
        let b: BanList = serde_json::from_str(&json).unwrap();
        assert_eq!(b.get(EP).unwrap().hostname, "");
    }

    // ---- recheck queue ----------------------------------------------------

    /// A never-rechecked suspect is due immediately: name-churn can only fire if
    /// a suspect is re-examined promptly, and waiting minutes would stall it.
    #[test]
    fn first_recheck_is_due_immediately() {
        let mut q = RecheckQueue::new();
        assert_eq!(q.schedule(EP, 1_000), Scheduled::Added);
        assert!(q.is_queued(EP));
        assert_eq!(
            q.due(1_000, 10),
            vec![EP.to_string()],
            "the first recheck must be due at once"
        );
    }

    /// Once a suspect has been rechecked, further rechecks are spaced out.
    #[test]
    fn repeat_rechecks_are_spaced() {
        let mut q = RecheckQueue::new();
        q.schedule(EP, 1_000);
        q.done(EP);
        assert_eq!(q.schedule(EP, 2_000), Scheduled::Added);
        assert!(q.due(2_000, 10).is_empty(), "must not be immediate again");
        assert_eq!(
            q.due(2_000 + RECHECK_INTERVAL_SECS, 10),
            vec![EP.to_string()]
        );
    }

    #[test]
    fn scheduling_twice_does_not_duplicate() {
        let mut q = RecheckQueue::new();
        q.schedule(EP, 1_000);
        assert_eq!(q.schedule(EP, 1_100), Scheduled::AlreadyQueued);
        assert_eq!(q.len(), 1);
        // The earlier due time is kept, so it is not pushed back by re-sighting.
        assert_eq!(q.due(1_000, 10), vec![EP.to_string()]);
    }

    /// A suspect is rechecked a bounded number of times, then left alone.
    #[test]
    fn rechecks_are_capped() {
        let mut q = RecheckQueue::new();
        let mut now = 1_000;
        for i in 0..MAX_RECHECKS {
            assert_eq!(q.schedule(EP, now), Scheduled::Added, "round {i}");
            now += RECHECK_INTERVAL_SECS;
            assert_eq!(q.due(now, 10), vec![EP.to_string()]);
            q.done(EP);
        }
        assert_eq!(q.rechecks_done(EP), MAX_RECHECKS);
        assert_eq!(
            q.schedule(EP, now),
            Scheduled::Exhausted,
            "must stop after {MAX_RECHECKS} rechecks"
        );
    }

    /// A recheck that has not been confirmed done stays queued, so a failed
    /// query is retried rather than silently consumed.
    #[test]
    fn due_is_non_destructive_until_done() {
        let mut q = RecheckQueue::new();
        q.schedule(EP, 1_000);
        let t = 1_000 + RECHECK_INTERVAL_SECS;
        assert_eq!(q.due(t, 10), vec![EP.to_string()]);
        assert_eq!(q.due(t, 10), vec![EP.to_string()], "still queued");
        q.done(EP);
        assert!(q.due(t, 10).is_empty(), "consumed only after done()");
    }

    #[test]
    fn due_respects_the_limit_and_orders_by_time() {
        let mut q = RecheckQueue::new();
        q.schedule("a:1", 1_000);
        q.schedule("b:1", 900);
        q.schedule("c:1", 950);
        let t = 1_000 + RECHECK_INTERVAL_SECS;
        assert_eq!(q.due(t, 2), vec!["b:1".to_string(), "c:1".to_string()]);
        assert_eq!(q.due(t, 10).len(), 3);
    }

    #[test]
    fn remove_forgets_an_endpoint_entirely() {
        let mut q = RecheckQueue::new();
        q.schedule(EP, 1_000);
        q.done(EP);
        q.remove(EP);
        assert!(!q.is_queued(EP));
        assert_eq!(q.rechecks_done(EP), 0, "counter cleared too");
    }

    /// Pruning must not forget a recheck count, because that count is what
    /// enforces the cap.
    #[test]
    fn prune_keeps_recheck_counts() {
        let mut q = RecheckQueue::new();
        q.schedule("a:1", 1_000);
        q.done("a:1");
        q.prune();
        assert!(!q.is_queued("a:1"), "no longer pending");
        assert_eq!(
            q.rechecks_done("a:1"),
            1,
            "the count must survive pruning, or the cap resets"
        );
    }

    #[test]
    fn queue_survives_json() {
        let mut q = RecheckQueue::new();
        q.schedule(EP, 1_000);
        let json = serde_json::to_string(&q).unwrap();
        let back: RecheckQueue = serde_json::from_str(&json).unwrap();
        assert!(back.is_queued(EP), "a pending recheck must persist");
    }

    #[test]
    fn empty_json_is_usable() {
        let b: BanList = serde_json::from_str("{}").unwrap();
        let q: RecheckQueue = serde_json::from_str("{}").unwrap();
        assert!(b.is_empty() && q.is_empty());
    }
}
