//! Per-template (and global) rate limit.
//!
//! Replaces `NullEdgeGate` which used shuffled-feature controls. Templates
//! that pass their own falsification checks are non-random by construction,
//! so rate-limit is the only honest overfitting guard.
//!
//! Rule: across the last `window` bars, no more than `limit_per_100 * window / 100`
//! signals. Default: 8 per 100 bars.

/// Rolling signal rate limiter.
///
/// Tracks (bar_index, signal_template_id) tuples in a sliding window. On
/// each new bar, evict entries older than `window` bars, then check whether
/// emitting a signal for `template_id` would exceed the rate cap.
#[derive(Debug)]
pub struct RateLimiter {
    window: u32,
    limit_per_100: f64,
    /// Recent emit records (bar_index, template_id as &'static str).
    emits: Vec<(u64, &'static str)>,
}

impl RateLimiter {
    #[must_use]
    pub fn new(window: u32, limit_per_100: f64) -> Self {
        Self {
            window: window.max(1),
            limit_per_100,
            emits: Vec::new(),
        }
    }

    /// Trim emits to bars within `[cur_bar - window, cur_bar]`.
    pub fn prune(&mut self, cur_bar: u64) {
        let cutoff = cur_bar.saturating_sub(self.window as u64);
        self.emits.retain(|&(bi, _)| bi >= cutoff);
    }

    /// Returns true if emitting a signal for `template_id` now would stay under
    /// the rate cap. Does NOT itself emit (callers should call `record` after).
    #[must_use]
    pub fn would_emit(&self, template_id: &'static str) -> bool {
        let cap = (self.limit_per_100 * self.window as f64 / 100.0).ceil() as usize;
        let count_same = self
            .emits
            .iter()
            .filter(|(_, t)| *t == template_id)
            .count();
        count_same < cap.max(1)
    }

    /// Record that a signal fired at `bar_index` for `template_id`.
    pub fn record(&mut self, bar_index: u64, template_id: &'static str) {
        self.emits.push((bar_index, template_id));
    }

    /// Number of emits in the rolling window.
    #[must_use]
    pub fn recent_count(&self) -> usize {
        self.emits.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn under_cap_allows() {
        let mut rl = RateLimiter::new(100, 8.0);
        rl.prune(50);
        assert!(rl.would_emit("absorption_reversal"));
        assert_eq!(rl.recent_count(), 0);
    }

    #[test]
    fn at_cap_denies() {
        let mut rl = RateLimiter::new(100, 8.0); // cap = 8
        rl.prune(100);
        for i in 0..8 {
            rl.record(i, "absorption_reversal");
        }
        rl.prune(100);
        assert!(!rl.would_emit("absorption_reversal"));
        assert!(rl.would_emit("other_template"));
    }

    #[test]
    fn prune_removes_old() {
        let mut rl = RateLimiter::new(50, 8.0);
        rl.record(0, "t1");
        rl.record(10, "t1");
        rl.prune(60); // cutoff = 10
        assert_eq!(rl.recent_count(), 1); // only bar 10 remains
    }

    #[test]
    fn different_templates_independent() {
        let mut rl = RateLimiter::new(100, 8.0);
        rl.prune(100);
        for i in 0..8 {
            rl.record(i, "t1");
        }
        assert!(!rl.would_emit("t1"));
        assert!(rl.would_emit("t2"));
    }
}
