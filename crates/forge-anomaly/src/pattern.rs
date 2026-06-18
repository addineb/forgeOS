//! Repetitive pattern counter: tracks how often similar anomaly signatures recur.

use std::collections::{HashMap, VecDeque};

use crate::types::{AnomalyEvent, AnomalyKind, SignalDirection};

/// Tracks co-occurrence signatures within a rolling bar window.
#[derive(Debug)]
pub struct PatternCounter {
    lookback: usize,
    min_count: u32,
    history: VecDeque<u64>,
    counts: HashMap<u64, u32>,
}

impl PatternCounter {
    #[must_use]
    pub fn new(lookback_bars: usize, min_count: u32) -> Self {
        Self {
            lookback: lookback_bars.max(1),
            min_count: min_count.max(2),
            history: VecDeque::new(),
            counts: HashMap::new(),
        }
    }

    /// Hash firing anomaly kinds + direction + z-magnitude into a compact key.
    #[must_use]
    pub fn signature(events: &[AnomalyEvent]) -> u64 {
        let mut key: u64 = 0;
        for e in events {
            let kind_bits = match e.kind {
                AnomalyKind::Ofi => 1u64,
                AnomalyKind::Cvd => 2,
                AnomalyKind::DepthImbalance => 4,
                AnomalyKind::Absorption => 8,
                AnomalyKind::LiquidityVacuum => 16,
                AnomalyKind::VolDeltaDivergence => 32,
                AnomalyKind::PatternRepeat => 64,
            };
            let bucket = (e.z_score.abs() / 2.0) as u64;
            let magnitude_bucket = bucket.min(3);
            let dir_bits = match e.direction {
                SignalDirection::Long => 128,
                SignalDirection::Short => 256,
                SignalDirection::Neutral => 0,
            };
            let combined = kind_bits | (magnitude_bucket << 8) | dir_bits;
            key = key.wrapping_mul(131).wrapping_add(combined);
        }
        key
    }

    /// Record a pattern; returns count after insertion.
    pub fn record(&mut self, key: u64) -> u32 {
        self.history.push_back(key);
        *self.counts.entry(key).or_insert(0) += 1;
        while self.history.len() > self.lookback {
            if let Some(old) = self.history.pop_front() {
                if let Some(c) = self.counts.get_mut(&old) {
                    *c = c.saturating_sub(1);
                    if *c == 0 {
                        self.counts.remove(&old);
                    }
                }
            }
        }
        self.counts.get(&key).copied().unwrap_or(1)
    }

    #[must_use]
    pub fn is_repetitive(&self, key: u64) -> bool {
        self.counts.get(&key).copied().unwrap_or(0) >= self.min_count
    }
}