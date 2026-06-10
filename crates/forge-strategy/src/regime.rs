//! Regime classifier: label the tape as Trending / Sideways / Neutral from the
//! Kaufman EFFICIENCY RATIO (net move / total path over a rolling window of
//! mid-price samples). ER in [0, 1]: ~1 = clean directional move, ~0 = choppy
//! back-and-forth.
//!
//! This is an ATTRIBUTION lens, not a trade gate: bots trade identically; the
//! sweep uses the label to split each variant's P&L by regime, so we can see
//! which variant earns its money in chop vs in trend.

use std::collections::VecDeque;

/// Market regime label.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Regime {
    /// Clean directional move (efficiency ratio >= `hi`).
    Trending,
    /// Choppy / mean-reverting (efficiency ratio <= `lo`).
    Sideways,
    /// In between the two thresholds (or not enough history yet).
    Neutral,
}

impl Regime {
    /// Stable index for per-regime arrays: Trending=0, Sideways=1, Neutral=2.
    #[must_use]
    pub fn idx(self) -> usize {
        match self {
            Regime::Trending => 0,
            Regime::Sideways => 1,
            Regime::Neutral => 2,
        }
    }
}

/// Classifier knobs.
#[derive(Clone, Copy, Debug)]
pub struct RegimeConfig {
    /// Rolling window length in mid-price samples.
    pub window: usize,
    /// Efficiency ratio at/above which the tape is Trending.
    pub hi: f64,
    /// Efficiency ratio at/below which the tape is Sideways.
    pub lo: f64,
}

impl Default for RegimeConfig {
    fn default() -> Self {
        Self { window: 32, hi: 0.5, lo: 0.2 }
    }
}

impl RegimeConfig {
    /// Classify an efficiency-ratio value into a regime.
    #[must_use]
    pub fn classify(&self, er: f64) -> Regime {
        if er >= self.hi {
            Regime::Trending
        } else if er <= self.lo {
            Regime::Sideways
        } else {
            Regime::Neutral
        }
    }
}

/// Rolling Kaufman efficiency ratio over mid-price samples (fixed-point raw).
pub struct EfficiencyRatio {
    window: usize,
    mids: VecDeque<i64>,
    abs_path: i64, // sum of |delta| over the consecutive mids held
}

impl EfficiencyRatio {
    /// New estimator over the last `window` deltas (window >= 2).
    #[must_use]
    pub fn new(window: usize) -> Self {
        Self { window: window.max(2), mids: VecDeque::new(), abs_path: 0 }
    }

    /// Fold one mid-price sample (raw fixed-point).
    pub fn observe(&mut self, mid: i64) {
        if let Some(&last) = self.mids.back() {
            self.abs_path += (mid - last).abs();
        }
        self.mids.push_back(mid);
        // keep window+1 mids so we have exactly `window` consecutive deltas
        while self.mids.len() > self.window + 1 {
            let a = self.mids.pop_front().unwrap_or(0);
            if let Some(&b) = self.mids.front() {
                self.abs_path -= (b - a).abs();
            }
        }
    }

    /// True once a full window of deltas has accumulated.
    #[must_use]
    pub fn ready(&self) -> bool {
        self.mids.len() > self.window
    }

    /// Efficiency ratio in [0, 1]: |net move| / total path. 0 if no path.
    #[must_use]
    pub fn value(&self) -> f64 {
        if self.abs_path <= 0 || self.mids.len() < 2 {
            return 0.0;
        }
        let (front, back) = (self.mids.front().copied().unwrap_or(0), self.mids.back().copied().unwrap_or(0));
        let net = (back - front).abs();
        net as f64 / self.abs_path as f64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_trend_is_trending() {
        let rc = RegimeConfig::default();
        let mut er = EfficiencyRatio::new(16);
        let mut mid = 100_000_000i64;
        for _ in 0..40 {
            mid += 1_000_000; // straight line up: net == path => ER == 1
            er.observe(mid);
        }
        assert!(er.ready());
        assert!((er.value() - 1.0).abs() < 1e-9, "ER={}", er.value());
        assert_eq!(rc.classify(er.value()), Regime::Trending);
    }

    #[test]
    fn pure_chop_is_sideways() {
        let rc = RegimeConfig::default();
        let mut er = EfficiencyRatio::new(16);
        let base = 100_000_000i64;
        for i in 0..40 {
            // oscillate +/- one tick: lots of path, ~zero net => ER ~ 0
            er.observe(base + if i % 2 == 0 { 1_000_000 } else { 0 });
        }
        assert!(er.ready());
        assert!(er.value() <= rc.lo, "ER={}", er.value());
        assert_eq!(rc.classify(er.value()), Regime::Sideways);
    }

    #[test]
    fn window_rolls_off() {
        let mut er = EfficiencyRatio::new(4);
        for i in 0..100 {
            er.observe(100_000_000 + i * 1_000_000);
        }
        // only the last 4 deltas count; still a clean trend => ER == 1
        assert!((er.value() - 1.0).abs() < 1e-9);
    }
}