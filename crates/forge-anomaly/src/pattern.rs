//! Sequence-based pattern detector: tracks short ordered sequences of anomaly kinds across bars.
//!
//! Instead of hashing co-occurring anomalies within a single bar, this detector
//! maintains a rolling window of recent *dominant anomaly kinds* (one per bar)
//! and hashes fixed-length subsequences to find repetition.
//!
//! Example: if the last 5 bars produced anomaly sequences like
//! `[Ofi, Absorption, LiquidityVacuum]` and that same 3-element sequence
//! appears again, the detector flags it as repetitive.
//!
//! This is timeframe-agnostic because it operates on bar boundaries, not
//! clock time or absolute magnitudes.

use std::collections::HashMap;
use std::collections::VecDeque;

use crate::types::{AnomalyEvent, AnomalyKind};

/// Maximum length of subsequence to hash (longer sequences are truncated).
const MAX_SEQ_LEN: usize = 5;

/// Tracks ordered anomaly-kind sequences across a rolling bar window.
#[derive(Debug)]
pub struct PatternCounter {
    /// Total lookback window in bars.
    lookback: usize,
    /// Minimum repetition count to consider a pattern "repetitive."
    min_count: u32,
    /// Subsequence length to hash (clamped to [2, MAX_SEQ_LEN]).
    seq_len: usize,
    /// Rolling buffer of (bar_index, dominant_anomaly_kind) for recent bars.
    history: VecDeque<(u64, AnomalyKind)>,
    /// Count of how many times each subsequence hash has appeared.
    counts: HashMap<u64, u32>,
}

impl PatternCounter {
    #[must_use]
    pub fn new(lookback_bars: usize, min_count: u32) -> Self {
        let seq_len = (lookback_bars / 4).clamp(2, MAX_SEQ_LEN);
        Self {
            lookback: lookback_bars.max(1),
            min_count: min_count.max(2),
            seq_len,
            history: VecDeque::new(),
            counts: HashMap::new(),
        }
    }

    /// Extract the dominant (highest |z|) anomaly kind from a bar's events.
    /// Returns `None` if no events are present.
    #[must_use]
    fn dominant_kind(events: &[AnomalyEvent]) -> Option<AnomalyKind> {
        events
            .iter()
            .filter(|e| e.kind != AnomalyKind::PatternRepeat)
            .max_by(|a, b| a.z_score.abs().partial_cmp(&b.z_score.abs()).unwrap_or(std::cmp::Ordering::Equal))
            .map(|e| e.kind)
    }

    /// Record the dominant anomaly kind for the current bar.
    /// Returns the repetition count for the current bar's subsequence.
    pub fn record(&mut self, bar_index: u64, events: &[AnomalyEvent]) -> u32 {
        let Some(kind) = Self::dominant_kind(events) else {
            return 0;
        };

        self.history.push_back((bar_index, kind));
        while self.history.len() > self.lookback {
            self.history.pop_front();
        }

        let key = self.current_seq_hash();
        if key == 0 {
            return 0;
        }

        *self.counts.entry(key).or_insert(0) += 1;
        self.counts.get(&key).copied().unwrap_or(1)
    }

    #[must_use]
    pub fn is_repetitive(&self, key: u64) -> bool {
        key != 0 && self.counts.get(&key).copied().unwrap_or(0) >= self.min_count
    }

    /// Hash the most recent `seq_len` anomaly kinds into a compact u64 key.
    /// Returns 0 if fewer than `seq_len` entries are available.
    #[must_use]
    pub fn current_seq_hash(&self) -> u64 {
        if self.history.len() < self.seq_len {
            return 0;
        }
        let recent: Vec<AnomalyKind> = self
            .history
            .iter()
            .rev()
            .take(self.seq_len)
            .rev()
            .map(|(_, k)| *k)
            .collect();

        // Reject sequences with consecutive duplicates (noise filter).
        for i in 1..recent.len() {
            if recent[i] == recent[i - 1] {
                return 0;
            }
        }

        let mut key: u64 = 0;
        for &kind in &recent {
            let bits = kind_to_bits(kind);
            key = key.wrapping_mul(131).wrapping_add(bits);
        }
        key
    }
}

/// Map AnomalyKind to a unique bit pattern for hashing.
fn kind_to_bits(kind: AnomalyKind) -> u64 {
    match kind {
        AnomalyKind::Ofi => 1,
        AnomalyKind::Cvd => 2,
        AnomalyKind::DepthImbalance => 3,
        AnomalyKind::Absorption => 4,
        AnomalyKind::LiquidityVacuum => 5,
        AnomalyKind::VolDeltaDivergence => 6,
        AnomalyKind::AggressorImbalance => 7,
        AnomalyKind::LargePrint => 8,
        AnomalyKind::TradeIntensity => 9,
        AnomalyKind::PatternRepeat => 10,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SignalDirection;

    fn event(kind: AnomalyKind, z: f64) -> AnomalyEvent {
        AnomalyEvent {
            bar_index: 0, ts: 0, kind, direction: SignalDirection::Neutral,
            z_score: z, raw_value: 0.0, confidence: 0.5,
        }
    }

    #[test]
    fn detects_repeating_sequence() {
        let mut counter = PatternCounter::new(20, 2);
        let seq_len = counter.seq_len;
        // Build a repeating sequence of length seq_len
        let kinds = [AnomalyKind::Ofi, AnomalyKind::Absorption, AnomalyKind::LiquidityVacuum];
        let seq: Vec<AnomalyKind> = (0..seq_len).map(|i| kinds[i % kinds.len()]).collect();
        
        for round in 0..2 {
            for (i, &kind) in seq.iter().enumerate() {
                let bar = (seq_len * round + i) as u64;
                let events = vec![event(kind, 3.0)];
                counter.record(bar, &events);
            }
        }
        let key = counter.current_seq_hash();
        assert!(key != 0);
        assert!(counter.is_repetitive(key));
    }

    #[test]
    fn rejects_consecutive_duplicates() {
        let mut counter = PatternCounter::new(10, 2);
        // Same kind repeated → hash should be 0.
        for i in 0..5 {
            let events = vec![event(AnomalyKind::Ofi, 3.0)];
            counter.record(i, &events);
        }
        assert_eq!(counter.current_seq_hash(), 0);
    }

    #[test]
    fn returns_zero_when_insufficient_history() {
        let counter = PatternCounter::new(20, 2);
        assert_eq!(counter.current_seq_hash(), 0);
    }
}
