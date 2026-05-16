//! Per-rendezvous bandwidth metering + new-session rate limiting.
//!
//! Per `spec/relay-protocol.md` "Bandwidth metering and rate limiting" and
//! the relay component spec's "Auth and accounting":
//!
//! - **Per-rendezvous byte counters** — cumulative consumer→storage and
//!   storage→consumer byte volume, kept in memory for operational
//!   telemetry. This is *not* OHDC state; it never sees plaintext (the
//!   relay only ever counts `DATA`-frame ciphertext lengths).
//! - **Per-rendezvous new-session rate limit** — a sliding-window counter
//!   of consumer attaches. When a rendezvous exceeds the configured
//!   allowance, the relay rejects the *attach* with HTTP `429` before the
//!   WebSocket / QUIC stream is ever upgraded; the storage tunnel and any
//!   already-open sessions are untouched.
//!
//! The whole table is in-memory: metering is best-effort telemetry, and a
//! relay restart resetting the counters / windows is acceptable (it errs
//! toward *more* permissive, never toward locking a user out). Durable
//! billing-grade accounting, if an operator needs it, is layered on top by
//! scraping `snapshot()` — out of scope for v1.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Default sliding window for the new-session rate limit.
pub const DEFAULT_RATE_WINDOW: Duration = Duration::from_secs(60);

/// Default number of consumer attaches allowed per rendezvous per window.
/// Generous enough that a normal consumer (CORD reconnecting per chat turn,
/// a clinician opening a session) never trips it; tight enough that a
/// runaway / abusive client is throttled. Operators can override via
/// `[metering]` in `relay.toml`.
pub const DEFAULT_RATE_MAX_SESSIONS: u32 = 30;

/// Tunable metering / rate-limit policy. Snapshotted from `relay.toml`'s
/// `[metering]` block at boot.
#[derive(Debug, Clone)]
pub struct MeteringPolicy {
    /// Sliding-window length for the new-session counter.
    pub rate_window: Duration,
    /// Maximum consumer attaches per rendezvous within `rate_window`.
    /// `0` disables the limit entirely (pure metering, no throttling).
    pub rate_max_sessions: u32,
}

impl Default for MeteringPolicy {
    fn default() -> Self {
        Self {
            rate_window: DEFAULT_RATE_WINDOW,
            rate_max_sessions: DEFAULT_RATE_MAX_SESSIONS,
        }
    }
}

/// Per-rendezvous counters. Cheap to copy out via [`MeteringTable::snapshot`].
#[derive(Debug, Clone, Default)]
pub struct RendezvousMetrics {
    /// Cumulative consumer→storage `DATA`-frame payload bytes.
    pub bytes_up: u64,
    /// Cumulative storage→consumer `DATA`-frame payload bytes.
    pub bytes_down: u64,
    /// Total consumer attaches accepted over this rendezvous' lifetime.
    pub sessions_total: u64,
    /// Consumer attaches rejected by the rate limiter.
    pub sessions_rejected: u64,
}

impl RendezvousMetrics {
    /// Total bytes forwarded in both directions.
    pub fn bytes_total(&self) -> u64 {
        self.bytes_up.saturating_add(self.bytes_down)
    }
}

/// Internal per-rendezvous row: counters + the sliding-window attach
/// timestamps used by the rate limiter.
#[derive(Debug, Default)]
struct Row {
    metrics: RendezvousMetrics,
    /// Attach instants within the current window. Pruned on each check.
    recent_attaches: Vec<Instant>,
}

/// Outcome of a rate-limit check for a consumer attach.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AttachDecision {
    /// The attach is within the allowance; proceed.
    Allow,
    /// The rendezvous exceeded its new-session allowance. The attach
    /// handler must reject with HTTP `429`.
    RateLimited,
}

impl AttachDecision {
    pub fn is_allowed(self) -> bool {
        matches!(self, AttachDecision::Allow)
    }
}

/// In-memory metering table, keyed by `rendezvous_id`. Cloneable handle is
/// not provided — wrap in `Arc` (as `RelayState` does).
#[derive(Debug)]
pub struct MeteringTable {
    policy: MeteringPolicy,
    rows: Mutex<HashMap<String, Row>>,
}

impl MeteringTable {
    pub fn new(policy: MeteringPolicy) -> Self {
        Self {
            policy,
            rows: Mutex::new(HashMap::new()),
        }
    }

    /// Convenience: a table with the default policy.
    pub fn with_defaults() -> Self {
        Self::new(MeteringPolicy::default())
    }

    pub fn policy(&self) -> &MeteringPolicy {
        &self.policy
    }

    /// Check whether a new consumer attach for `rendezvous_id` is allowed
    /// under the rate limit, and — when allowed — record it. Returns
    /// [`AttachDecision::RateLimited`] when the rendezvous has already used
    /// its allowance in the current window.
    ///
    /// `now` is injected so tests can drive the sliding window
    /// deterministically; production passes `Instant::now()`.
    pub fn check_and_record_attach(
        &self,
        rendezvous_id: &str,
        now: Instant,
    ) -> AttachDecision {
        let mut rows = self.rows.lock().unwrap();
        let row = rows.entry(rendezvous_id.to_string()).or_default();

        // Prune attaches that have fallen out of the window.
        let window = self.policy.rate_window;
        row.recent_attaches
            .retain(|t| now.duration_since(*t) < window);

        // `0` disables throttling entirely.
        if self.policy.rate_max_sessions != 0
            && row.recent_attaches.len() as u32 >= self.policy.rate_max_sessions
        {
            row.metrics.sessions_rejected =
                row.metrics.sessions_rejected.saturating_add(1);
            return AttachDecision::RateLimited;
        }

        row.recent_attaches.push(now);
        row.metrics.sessions_total = row.metrics.sessions_total.saturating_add(1);
        AttachDecision::Allow
    }

    /// Record `n` consumer→storage `DATA` payload bytes.
    pub fn record_up(&self, rendezvous_id: &str, n: u64) {
        if n == 0 {
            return;
        }
        let mut rows = self.rows.lock().unwrap();
        let row = rows.entry(rendezvous_id.to_string()).or_default();
        row.metrics.bytes_up = row.metrics.bytes_up.saturating_add(n);
    }

    /// Record `n` storage→consumer `DATA` payload bytes.
    pub fn record_down(&self, rendezvous_id: &str, n: u64) {
        if n == 0 {
            return;
        }
        let mut rows = self.rows.lock().unwrap();
        let row = rows.entry(rendezvous_id.to_string()).or_default();
        row.metrics.bytes_down = row.metrics.bytes_down.saturating_add(n);
    }

    /// Copy out the counters for one rendezvous (`None` if never seen).
    pub fn snapshot(&self, rendezvous_id: &str) -> Option<RendezvousMetrics> {
        let rows = self.rows.lock().unwrap();
        rows.get(rendezvous_id).map(|r| r.metrics.clone())
    }

    /// Copy out every rendezvous' counters — for an operator telemetry
    /// endpoint / log line.
    pub fn snapshot_all(&self) -> Vec<(String, RendezvousMetrics)> {
        let rows = self.rows.lock().unwrap();
        rows.iter()
            .map(|(k, v)| (k.clone(), v.metrics.clone()))
            .collect()
    }

    /// Drop a rendezvous' row entirely — called on deregistration so the
    /// table doesn't accumulate dead users.
    pub fn forget(&self, rendezvous_id: &str) {
        self.rows.lock().unwrap().remove(rendezvous_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_counters_accumulate_per_direction() {
        let m = MeteringTable::with_defaults();
        m.record_up("rzv", 100);
        m.record_up("rzv", 50);
        m.record_down("rzv", 200);
        let s = m.snapshot("rzv").unwrap();
        assert_eq!(s.bytes_up, 150);
        assert_eq!(s.bytes_down, 200);
        assert_eq!(s.bytes_total(), 350);
    }

    #[test]
    fn unknown_rendezvous_has_no_snapshot() {
        let m = MeteringTable::with_defaults();
        assert!(m.snapshot("never-seen").is_none());
    }

    #[test]
    fn rate_limit_allows_up_to_max_then_rejects() {
        let policy = MeteringPolicy {
            rate_window: Duration::from_secs(60),
            rate_max_sessions: 3,
        };
        let m = MeteringTable::new(policy);
        let now = Instant::now();
        // First three attaches in the same instant: allowed.
        for _ in 0..3 {
            assert_eq!(m.check_and_record_attach("rzv", now), AttachDecision::Allow);
        }
        // Fourth in-window attach: rejected.
        assert_eq!(
            m.check_and_record_attach("rzv", now),
            AttachDecision::RateLimited
        );
        let s = m.snapshot("rzv").unwrap();
        assert_eq!(s.sessions_total, 3);
        assert_eq!(s.sessions_rejected, 1);
    }

    #[test]
    fn rate_limit_window_slides() {
        let policy = MeteringPolicy {
            rate_window: Duration::from_secs(60),
            rate_max_sessions: 2,
        };
        let m = MeteringTable::new(policy);
        let t0 = Instant::now();
        assert_eq!(m.check_and_record_attach("rzv", t0), AttachDecision::Allow);
        assert_eq!(m.check_and_record_attach("rzv", t0), AttachDecision::Allow);
        assert_eq!(
            m.check_and_record_attach("rzv", t0),
            AttachDecision::RateLimited
        );
        // 61s later the first two attaches have aged out of the window.
        let t1 = t0 + Duration::from_secs(61);
        assert_eq!(m.check_and_record_attach("rzv", t1), AttachDecision::Allow);
    }

    #[test]
    fn rate_limit_is_per_rendezvous() {
        let policy = MeteringPolicy {
            rate_window: Duration::from_secs(60),
            rate_max_sessions: 1,
        };
        let m = MeteringTable::new(policy);
        let now = Instant::now();
        assert_eq!(m.check_and_record_attach("a", now), AttachDecision::Allow);
        assert_eq!(
            m.check_and_record_attach("a", now),
            AttachDecision::RateLimited
        );
        // A different rendezvous has its own allowance.
        assert_eq!(m.check_and_record_attach("b", now), AttachDecision::Allow);
    }

    #[test]
    fn zero_max_disables_throttling() {
        let policy = MeteringPolicy {
            rate_window: Duration::from_secs(60),
            rate_max_sessions: 0,
        };
        let m = MeteringTable::new(policy);
        let now = Instant::now();
        for _ in 0..1000 {
            assert_eq!(m.check_and_record_attach("rzv", now), AttachDecision::Allow);
        }
    }

    #[test]
    fn forget_drops_the_row() {
        let m = MeteringTable::with_defaults();
        m.record_up("rzv", 10);
        assert!(m.snapshot("rzv").is_some());
        m.forget("rzv");
        assert!(m.snapshot("rzv").is_none());
    }
}
