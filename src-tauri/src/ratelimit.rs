//! Per-endpoint A2S query pacing.
//!
//! Game servers run anti-flood protection: repeated A2S queries from one source
//! get the source dropped or permanently banned. Two limits are enforced per
//! `ip:port`, both required:
//!
//! | limit | value |
//! |---|---|
//! | minimum gap between queries of the same endpoint | [`MIN_INTERVAL_MS`] (5 s) |
//! | queries within any rolling window | [`MAX_PER_WINDOW`] (3) per [`WINDOW_MS`] (60 s) |
//!
//! The gap alone would permit 12 queries/minute, so the window is what actually
//! bounds the load. Both are checked because a server may tolerate a burst but
//! still object to being polled every 6 seconds.
//!
//! A refresh the user asks for on **one** server (the Game Info dialog, the
//! row's context menu) is checked by [`QueryGate::try_acquire_interactive`]
//! instead: only an [`INTERACTIVE_MIN_INTERVAL_MS`] (2 s) gap, no window. It
//! is one server at human pace, the same as the game's own info dialog, and
//! at 2 s apart it stays under the few-queries-per-second per-IP limits that
//! hosts' flood rules use. It is still recorded, so a sweep right after it
//! does not query that server again.
//!
//! ## Scope
//!
//! A single scan queries each endpoint once, so this only bites on the paths
//! that legitimately repeat work: pressing `r` in the TUI, and running the tool
//! again shortly after. State is persisted (inside the same local state file as
//! the name history) so a second *process* cannot hammer servers the first one
//! just queried — that case is exactly what triggers a ban in practice.
//!
//! ## What a refusal costs
//!
//! Nothing is sent. The endpoint is reported with
//! [`Outcome::RateLimited`](crate::scanner::Outcome::RateLimited) and whatever
//! data is already known (a previous run's result, or Steam's listing) is kept,
//! so a paced scan still shows a full list — just with some rows not refreshed.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Minimum gap between two queries of the same endpoint.
pub const MIN_INTERVAL_MS: u64 = 5_000;

/// Minimum gap for a user-requested single-server refresh
/// (see [`QueryGate::try_acquire_interactive`]).
pub const INTERACTIVE_MIN_INTERVAL_MS: u64 = 2_000;

/// Maximum queries of one endpoint inside a rolling [`WINDOW_MS`].
pub const MAX_PER_WINDOW: usize = 3;

/// Length of the rolling window.
pub const WINDOW_MS: u64 = 60_000;

/// Must equal [`MAX_PER_WINDOW`]; kept literal because serde implements
/// `Serialize`/`Deserialize` for `[T; N]` only for literal sizes.
const RING: usize = 3;

/// Entries untouched for this long are dropped on save.
const STALE_AFTER_MS: u64 = 60 * 60 * 1000;

/// Query times for one endpoint: the most recent [`MAX_PER_WINDOW`], oldest
/// first, compacted to `len`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct QueryStamp {
    #[serde(default)]
    times: [u64; RING],
    #[serde(default)]
    len: u8,
}

impl Default for QueryStamp {
    fn default() -> Self {
        Self {
            times: [0; RING],
            len: 0,
        }
    }
}

impl QueryStamp {
    /// Most recent query time, if any.
    pub fn last_ms(&self) -> Option<u64> {
        if self.len == 0 {
            None
        } else {
            Some(self.times[(self.len as usize) - 1])
        }
    }

    /// Oldest retained query time, if any.
    fn oldest_ms(&self) -> Option<u64> {
        if self.len == 0 {
            None
        } else {
            Some(self.times[0])
        }
    }

    fn push(&mut self, now_ms: u64) {
        let n = self.len as usize;
        if n < RING {
            self.times[n] = now_ms;
            self.len += 1;
        } else {
            // Full: drop the oldest and append.
            self.times.rotate_left(1);
            self.times[RING - 1] = now_ms;
        }
    }

    /// Forget everything older than the window, so a stale entry does not make
    /// an endpoint look busier than it is.
    fn expire(&mut self, now_ms: u64) {
        let mut kept = [0u64; RING];
        let mut n = 0usize;
        for i in 0..(self.len as usize) {
            let t = self.times[i];
            if now_ms.saturating_sub(t) < WINDOW_MS {
                kept[n] = t;
                n += 1;
            }
        }
        self.times = kept;
        self.len = n as u8;
    }
}

/// Whether a query may be sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    /// Nothing recorded for this endpoint recently: send it.
    Allow,
    /// Queried too recently (`MIN_INTERVAL_MS` not elapsed).
    TooSoon { retry_after_ms: u64 },
    /// Already at `MAX_PER_WINDOW` inside the rolling window.
    TooMany { retry_after_ms: u64 },
}

impl Verdict {
    pub fn is_allowed(self) -> bool {
        matches!(self, Verdict::Allow)
    }

    /// How long until a retry could succeed.
    pub fn retry_after_ms(self) -> u64 {
        match self {
            Verdict::Allow => 0,
            Verdict::TooSoon { retry_after_ms } | Verdict::TooMany { retry_after_ms } => {
                retry_after_ms
            }
        }
    }

    /// Refusal message carrying the wait, e.g. `recently queried; retry in 2 s`.
    pub fn message(self) -> String {
        format!(
            "{}; retry in {} s",
            self.label(),
            self.retry_after_ms().div_ceil(1000).max(1)
        )
    }

    /// Short label for the UI.
    pub fn label(self) -> &'static str {
        match self {
            Verdict::Allow => "ok",
            Verdict::TooSoon { .. } => "recently queried",
            Verdict::TooMany { .. } => "query rate limit",
        }
    }
}

/// Per-endpoint pacing state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct QueryGate {
    #[serde(default)]
    stamps: HashMap<String, QueryStamp>,
}

impl QueryGate {
    pub fn new() -> Self {
        Self::default()
    }

    /// Number of endpoints currently tracked.
    pub fn tracked(&self) -> usize {
        self.stamps.len()
    }

    /// Whether `endpoint` may be queried at `now_ms`. Does not record.
    ///
    /// `now_ms` is unix milliseconds; callers pass [`now_ms`].
    pub fn check(&self, endpoint: &str, now_ms: u64) -> Verdict {
        let Some(stamp) = self.stamps.get(endpoint).copied() else {
            return Verdict::Allow;
        };

        // Most recent query must be at least MIN_INTERVAL_MS ago.
        if let Some(last) = stamp.last_ms() {
            let since = now_ms.saturating_sub(last);
            if since < MIN_INTERVAL_MS {
                return Verdict::TooSoon {
                    retry_after_ms: MIN_INTERVAL_MS - since,
                };
            }
        }

        // Rolling-window cap: only binding once the ring is full.
        if (stamp.len as usize) >= MAX_PER_WINDOW {
            if let Some(oldest) = stamp.oldest_ms() {
                let since = now_ms.saturating_sub(oldest);
                if since < WINDOW_MS {
                    return Verdict::TooMany {
                        retry_after_ms: WINDOW_MS - since,
                    };
                }
            }
        }

        Verdict::Allow
    }

    /// Check and, if allowed, record the query.
    ///
    /// Recording happens at *schedule* time rather than send time; the
    /// difference is bounded by the scan's concurrency window, which is far
    /// below the 5 s granularity these limits care about.
    pub fn try_acquire(&mut self, endpoint: &str, now_ms: u64) -> Verdict {
        let verdict = self.check(endpoint, now_ms);
        if verdict.is_allowed() {
            let entry = self.stamps.entry(endpoint.to_string()).or_default();
            entry.expire(now_ms);
            entry.push(now_ms);
        }
        verdict
    }

    /// [`Self::try_acquire`] for one server the user asked to refresh: only
    /// the [`INTERACTIVE_MIN_INTERVAL_MS`] gap applies, not the sweep's
    /// window. The query is recorded like any other.
    pub fn try_acquire_interactive(&mut self, endpoint: &str, now_ms: u64) -> Verdict {
        if let Some(last) = self.stamps.get(endpoint).and_then(|s| s.last_ms()) {
            let since = now_ms.saturating_sub(last);
            if since < INTERACTIVE_MIN_INTERVAL_MS {
                return Verdict::TooSoon {
                    retry_after_ms: INTERACTIVE_MIN_INTERVAL_MS - since,
                };
            }
        }
        let entry = self.stamps.entry(endpoint.to_string()).or_default();
        entry.expire(now_ms);
        entry.push(now_ms);
        Verdict::Allow
    }

    /// Drop endpoints that have not been queried for a long time. Keyed by
    /// `ip:port`, so a repeatedly-queried endpoint updates its one entry
    /// rather than adding new ones.
    pub fn prune(&mut self, now_ms: u64) {
        self.stamps.retain(|_, s| match s.last_ms() {
            Some(t) => now_ms.saturating_sub(t) < STALE_AFTER_MS,
            None => false,
        });
    }
}

/// Current unix time in milliseconds.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// Back-off for endpoints that keep not answering.
///
/// A server that timed out is re-queried on every refresh otherwise, even
/// though it is almost certainly down — or already dropping our queries, in
/// which case more of them is exactly the wrong response. After consecutive
/// failures it is skipped (nothing sent) for a growing delay.
///
/// It never hides a live server for long, by construction:
///
/// - a **single** timeout changes nothing — one lost UDP packet is ordinary;
/// - any answer (sweep or single refresh) clears the streak at once;
/// - the longest delay is [`BACKOFF_MAX_MS`], and a hard refresh clears all;
/// - skipped rows are reported as not sent (`paced`), never as dead or fake.
///
/// In memory only: a restart retries everything once.
#[derive(Debug, Default)]
pub struct TimeoutBackoff {
    /// endpoint → (consecutive failures, time of the last one).
    entries: HashMap<String, (u32, u64)>,
}

/// The longest back-off: 30 minutes.
pub const BACKOFF_MAX_MS: u64 = 30 * 60 * 1000;

impl TimeoutBackoff {
    pub fn new() -> Self {
        Self::default()
    }

    /// How long to wait after `streak` consecutive failures.
    pub fn delay_ms(streak: u32) -> u64 {
        match streak {
            0 | 1 => 0,
            2 => 2 * 60 * 1000,
            3 => 10 * 60 * 1000,
            _ => BACKOFF_MAX_MS,
        }
    }

    /// Record one query result.
    pub fn record(&mut self, endpoint: &str, answered: bool, now_ms: u64) {
        if answered {
            self.entries.remove(endpoint);
        } else {
            let e = self
                .entries
                .entry(endpoint.to_string())
                .or_insert((0, now_ms));
            e.0 = e.0.saturating_add(1);
            e.1 = now_ms;
        }
    }

    /// Whether `endpoint` should be skipped right now.
    pub fn is_backing_off(&self, endpoint: &str, now_ms: u64) -> bool {
        self.entries
            .get(endpoint)
            .is_some_and(|&(streak, last)| now_ms < last.saturating_add(Self::delay_ms(streak)))
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod backoff_tests {
    use super::*;
    const EP: &str = "1.2.3.4:27015";

    #[test]
    fn one_timeout_never_hides_a_server() {
        let mut b = TimeoutBackoff::new();
        b.record(EP, false, 1_000);
        assert!(
            !b.is_backing_off(EP, 1_001),
            "a single lost packet is ordinary"
        );
    }

    #[test]
    fn consecutive_timeouts_back_off_and_the_delay_expires() {
        let mut b = TimeoutBackoff::new();
        b.record(EP, false, 0);
        b.record(EP, false, 10_000);
        assert!(b.is_backing_off(EP, 10_000 + 60_000));
        assert!(
            !b.is_backing_off(EP, 10_000 + 2 * 60_000),
            "2 minutes after the 2nd"
        );
        b.record(EP, false, 200_000);
        assert!(b.is_backing_off(EP, 200_000 + 9 * 60_000));
        for t in 0..10 {
            b.record(EP, false, 300_000 + t);
        }
        assert!(
            !b.is_backing_off(EP, 300_009 + BACKOFF_MAX_MS),
            "capped at 30 minutes"
        );
    }

    #[test]
    fn any_answer_clears_the_streak() {
        let mut b = TimeoutBackoff::new();
        b.record(EP, false, 0);
        b.record(EP, false, 1);
        b.record(EP, true, 2);
        assert!(!b.is_backing_off(EP, 3));
        b.record(EP, false, 4);
        assert!(!b.is_backing_off(EP, 5), "the streak restarts from one");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const EP: &str = "203.0.113.7:27015";

    #[test]
    fn first_query_is_allowed() {
        let g = QueryGate::new();
        assert_eq!(g.check(EP, 1_000_000), Verdict::Allow);
    }

    /// The headline rule: never re-query the same endpoint within 5 seconds.
    #[test]
    fn refuses_a_second_query_within_five_seconds() {
        let mut g = QueryGate::new();
        let t = 1_000_000;
        assert!(g.try_acquire(EP, t).is_allowed());

        for delta in [0u64, 1, 999, 4_999] {
            let v = g.check(EP, t + delta);
            assert!(
                matches!(v, Verdict::TooSoon { .. }),
                "+{delta}ms must be refused, got {v:?}"
            );
        }
        // At exactly 5s it is allowed again.
        assert!(
            g.check(EP, t + MIN_INTERVAL_MS).is_allowed(),
            "exactly {MIN_INTERVAL_MS}ms must be allowed"
        );
    }

    #[test]
    fn retry_after_counts_down() {
        let mut g = QueryGate::new();
        g.try_acquire(EP, 1_000_000);
        let v = g.check(EP, 1_000_000 + 2_000);
        assert_eq!(v.retry_after_ms(), 3_000, "must report remaining wait");
    }

    /// The second rule: at most 3 queries per rolling minute, however they are
    /// spaced.
    #[test]
    fn caps_at_three_per_rolling_minute() {
        let mut g = QueryGate::new();
        let t = 1_000_000;
        // Three queries, each past the 5s gap, inside the window.
        for i in 0..MAX_PER_WINDOW {
            let at = t + (i as u64) * MIN_INTERVAL_MS;
            assert!(
                g.try_acquire(EP, at).is_allowed(),
                "query {i} must be allowed"
            );
        }
        // A fourth, well past the 5s gap but inside the minute, must be refused.
        let fourth = t + 20_000;
        let v = g.check(EP, fourth);
        assert!(
            matches!(v, Verdict::TooMany { .. }),
            "4th query must hit the window cap, got {v:?}"
        );

        // Once the oldest of the three falls out of the window it is allowed.
        let after_window = t + WINDOW_MS;
        assert!(
            g.check(EP, after_window).is_allowed(),
            "after a full window the cap must reset"
        );
    }

    /// A tight loop of presses must not squeeze out extra queries.
    #[test]
    fn hammering_never_exceeds_the_limits() {
        let mut g = QueryGate::new();
        let mut allowed = 0usize;
        // Simulate 200 attempts, 100ms apart (20 seconds of hammering).
        for i in 0..200u64 {
            let at = 1_000_000 + i * 100;
            if g.try_acquire(EP, at).is_allowed() {
                allowed += 1;
            }
        }
        assert_eq!(
            allowed, MAX_PER_WINDOW,
            "20s of hammering must yield exactly {MAX_PER_WINDOW} queries"
        );
    }

    /// Limits are per endpoint; one server's budget must not restrict another.
    #[test]
    fn other_endpoints_are_unaffected() {
        let mut g = QueryGate::new();
        // Exhaust EP's budget.
        for i in 0..MAX_PER_WINDOW {
            assert!(g
                .try_acquire(EP, 1_000_000 + i as u64 * MIN_INTERVAL_MS)
                .is_allowed());
        }
        assert!(!g.check(EP, 1_000_000 + 5 * MIN_INTERVAL_MS).is_allowed());

        // Other endpoints are still free to query in the same instant.
        assert!(g.try_acquire("198.51.100.9:27015", 1_000_000).is_allowed());
        assert!(g.try_acquire("198.51.100.10:27015", 1_000_000).is_allowed());
    }

    /// Expired entries must not make an idle endpoint look busy.
    #[test]
    fn stale_entries_expire_rather_than_blocking_forever() {
        let mut g = QueryGate::new();
        g.try_acquire(EP, 1_000_000);
        // A day later the old timestamps are irrelevant.
        let later = 1_000_000 + 24 * 60 * 60 * 1000;
        assert!(g.check(EP, later).is_allowed());
        assert!(g.try_acquire(EP, later).is_allowed());
    }

    /// A backwards clock jump must not panic or grant a burst.
    #[test]
    fn clock_going_backwards_is_handled() {
        let mut g = QueryGate::new();
        g.try_acquire(EP, 1_000_000);
        // now < last: saturating_sub yields 0, so this is "too soon".
        let v = g.check(EP, 500_000);
        assert!(matches!(v, Verdict::TooSoon { .. }), "got {v:?}");
    }

    #[test]
    fn prune_drops_endpoints_untouched_for_an_hour() {
        let mut g = QueryGate::new();
        g.try_acquire("1.1.1.1:27015", 1_000_000);
        g.try_acquire("2.2.2.2:27015", 1_000_000 + STALE_AFTER_MS + 1);
        assert_eq!(g.tracked(), 2);
        g.prune(1_000_000 + STALE_AFTER_MS + 1);
        assert_eq!(g.tracked(), 1, "the stale endpoint must be dropped");
    }

    /// State must survive a process restart, or a second run would hammer the
    /// servers the first one just queried.
    #[test]
    fn round_trips_through_json() {
        let mut g = QueryGate::new();
        g.try_acquire(EP, 1_000_000);
        let json = serde_json::to_string(&g).unwrap();
        let back: QueryGate = serde_json::from_str(&json).unwrap();
        assert!(
            matches!(back.check(EP, 1_000_000 + 100), Verdict::TooSoon { .. }),
            "a reloaded gate must still remember the last query"
        );
    }

    #[test]
    fn empty_json_is_a_working_gate() {
        let g: QueryGate = serde_json::from_str("{}").unwrap();
        assert_eq!(g.tracked(), 0);
        assert!(g.check(EP, 1).is_allowed());
    }

    #[test]
    fn interactive_refresh_needs_only_the_short_gap() {
        let mut g = QueryGate::new();
        let t = 1_000_000;
        // A sweep query, then the dialog opening, then Refresh twice: the
        // sweep's rules would refuse all but the first.
        assert!(g.try_acquire(EP, t).is_allowed());
        assert!(g.try_acquire_interactive(EP, t + 2_000).is_allowed());
        assert!(g.try_acquire_interactive(EP, t + 4_000).is_allowed());
        assert!(g.try_acquire_interactive(EP, t + 6_000).is_allowed());
        let refused = g.try_acquire_interactive(EP, t + 7_500);
        assert_eq!(refused, Verdict::TooSoon { retry_after_ms: 500 });
        assert_eq!(refused.message(), "recently queried; retry in 1 s");
    }

    #[test]
    fn interactive_queries_still_pace_the_sweep() {
        let mut g = QueryGate::new();
        let t = 1_000_000;
        assert!(g.try_acquire_interactive(EP, t).is_allowed());
        assert!(!g.try_acquire(EP, t + 1_000).is_allowed());
    }
}
