//! Null-edge gate: validates that detected anomalies exceed shuffled controls.
//!
//! Prevents fake edge from random feature permutations or over-triggering.

use crate::detector::{IsolationForestDetector, MahalanobisDetector};
use crate::prng;
use crate::stats::RollingFeatureWindow;
use crate::types::{DetectionMethod, FeatureVector};

/// Validates signals against shuffled-feature controls and rate limits.
#[derive(Debug)]
pub struct NullEdgeGate {
    permutations: u32,
    margin: f64,
    max_signals_per_100: f64,
    recent_signals: usize,
    recent_bars: usize,
    seed: u64,
}

impl NullEdgeGate {
    #[must_use]
    pub fn new(permutations: u32, margin: f64, max_signals_per_100: f64, seed: u64) -> Self {
        Self {
            permutations,
            margin: margin.max(0.0),
            max_signals_per_100: max_signals_per_100.max(1.0),
            recent_signals: 0,
            recent_bars: 0,
            seed,
        }
    }

    /// Record a bar processed (for rate limiting).
    pub fn on_bar(&mut self) {
        self.recent_bars += 1;
        if self.recent_bars >= 100 {
            self.recent_signals = 0;
            self.recent_bars = 0;
        }
    }

    /// Record that a signal was emitted.
    pub fn on_signal(&mut self) {
        self.recent_signals += 1;
    }

    /// Whether signal rate is within acceptable bounds.
    #[must_use]
    pub fn rate_ok(&self) -> bool {
        (self.recent_signals as f64) < self.max_signals_per_100
    }

    /// Mahalanobis null-edge: real distance must exceed shuffled permutations.
    pub fn validate_mahalanobis(
        &mut self,
        window: &RollingFeatureWindow,
        x: &FeatureVector,
        real_dist: f64,
        detector: &MahalanobisDetector,
    ) -> bool {
        if self.permutations == 0 {
            return true;
        }
        if real_dist <= 0.0 {
            return false;
        }
        let mut shuffled_sum = 0.0;
        for p in 0..self.permutations {
            let perm = self.shuffle(x, p);
            shuffled_sum += detector.distance_or_zero(window, &perm);
        }
        let shuffled_mean = shuffled_sum / self.permutations as f64;
        real_dist > shuffled_mean * (1.0 + self.margin)
    }

    /// Isolation Forest null-edge: real score must exceed shuffled controls.
    pub fn validate_isolation(
        &mut self,
        window: &RollingFeatureWindow,
        x: &FeatureVector,
        real_score: f64,
        detector: &IsolationForestDetector,
    ) -> bool {
        if self.permutations == 0 {
            return true;
        }
        let mut shuffled_sum = 0.0;
        for p in 0..self.permutations {
            let perm = self.shuffle(x, p);
            shuffled_sum += detector.score(window, &perm);
        }
        let shuffled_mean = shuffled_sum / self.permutations as f64;
        real_score > shuffled_mean * (1.0 + self.margin)
    }

    /// Combined validation for the configured detection method.
    pub fn validate(
        &mut self,
        method: DetectionMethod,
        window: &RollingFeatureWindow,
        x: &FeatureVector,
        maha_dist: f64,
        iso_score: f64,
        maha_det: &MahalanobisDetector,
        iso_det: &IsolationForestDetector,
    ) -> bool {
        if !self.rate_ok() {
            return false;
        }
        match method {
            DetectionMethod::Mahalanobis => {
                self.validate_mahalanobis(window, x, maha_dist, maha_det)
            }
            DetectionMethod::IsolationForest => {
                self.validate_isolation(window, x, iso_score, iso_det)
            }
            DetectionMethod::Both => {
                self.validate_mahalanobis(window, x, maha_dist, maha_det)
                    && self.validate_isolation(window, x, iso_score, iso_det)
            }
            DetectionMethod::Either => {
                self.validate_mahalanobis(window, x, maha_dist, maha_det)
                    || self.validate_isolation(window, x, iso_score, iso_det)
            }
        }
    }

    fn shuffle(&mut self, x: &FeatureVector, perm_idx: u32) -> FeatureVector {
        let mut out = *x;
        let n = out.len();
        for i in 0..n {
            self.seed = prng::splitmix64(self.seed.wrapping_add(perm_idx as u64).wrapping_add(i as u64));
            let j = (self.seed as usize) % n;
            out.swap(i, j);
        }
        for val in out.iter_mut().take(n) {
            self.seed = prng::splitmix64(self.seed);
            if self.seed & 1 == 1 {
                *val = -*val;
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rate_limit_blocks_excess_signals() {
        let mut gate = NullEdgeGate::new(4, 0.25, 3.0, 0);
        gate.recent_signals = 3;
        assert!(!gate.rate_ok());
    }

    #[test]
    fn shuffle_produces_different_vector() {
        let mut gate = NullEdgeGate::new(4, 0.25, 10.0, 0);
        let x = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 0.5, 0.0, 0.0];
        let s = gate.shuffle(&x, 0);
        assert_ne!(s, x);
    }
}