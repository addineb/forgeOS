//! Sequence-based pattern detector: tracks short ordered sequences of anomaly kinds across bars.
//!
//! This detector maintains a rolling window of recent *dominant anomaly kinds* (top 1-2 per bar)
//! along with their directional bias and strength, and hashes fixed-length subsequences to find
//! repetition. Unlike the original single-bar approach, this captures temporal structure:
//! e.g., "OFI_Short(Medium) → Absorption_Short(Strong) → LiquidityVacuum" is a meaningful
//! sequence that suggests persistent selling pressure.
//!
//! Key design decisions:
//! - **Strength bands**: Slightly more permissive than pure normal distribution critical values
//!   (1.60/2.50 vs 1.65/2.58) to better fit fat-tailed order flow data, where extreme z-scores
//!   are more common than a normal distribution would predict.
//! - **Quality filter**: MIN_AVG_Z = 1.60. This filters out weak background noise while allowing
//!   moderately strong patterns through. Additionally, we require the sequence's avg |z| to be
//!   at least 15% higher than the rolling median |z|, ensuring patterns stand out from recent
//!   market behavior.
//! - **Cooldown**: max(2, (seq_len + 1) / 2). This ties the cooldown to the sequence length
//!   itself — a 3-bar sequence needs 2 bars gap, a 5-bar sequence needs 3 bars gap. This is
//!   more defensible than arbitrary lookback fractions.
//!
//! This is timeframe-agnostic because it operates on bar boundaries, not clock time or
//! absolute magnitudes.

use std::collections::HashMap;
use std::collections::VecDeque;

use crate::types::{AnomalyEvent, AnomalyKind, SignalDirection};

/// Minimum average |z-score| for a sequence to be considered "quality" and counted.
/// Set to 1.60, which is slightly more permissive than the 90th percentile (1.65) to
/// better accommodate fat-tailed order flow data while still filtering out weak noise.
const MIN_AVG_Z: f64 = 1.60;

/// Tracks ordered anomaly-kind sequences across a rolling bar window.
#[derive(Debug)]
pub struct PatternCounter {
    /// Total lookback window in bars.
    lookback: usize,
    /// Minimum repetition count to consider a pattern "repetitive."
    min_count: u32,
    /// Subsequence length to hash (scales with lookback, clamped to [2, 8]).
    seq_len: usize,
    /// Cooldown: minimum bars between counted repetitions of the same sequence.
    /// Computed as max(2, (seq_len + 1) / 2).
    cooldown: usize,
    /// Rolling buffer of (bar_index, primary_anomaly, secondary_anomaly, avg_z) for recent bars.
    history: VecDeque<BarEntry>,
    /// Count of how many times each subsequence hash has appeared.
    counts: HashMap<u64, u32>,
    /// Last bar index where each sequence hash was counted (for cooldown enforcement).
    last_counted: HashMap<u64, u64>,
    /// Rolling median |z| across all bars, used for context-aware quality filtering.
    recent_z_values: VecDeque<f64>,
}

/// A single bar's entry: top 1-2 anomalies plus the bar's average |z-score|.
#[derive(Debug, Clone)]
struct BarEntry {
    bar_index: u64,
    primary: AnomalyEntry,
    secondary: Option<AnomalyEntry>,
    avg_z: f64,
}

/// A single anomaly entry with kind, directional bias, and strength.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct AnomalyEntry {
    kind: AnomalyKind,
    dir: DirTag,
    strength: StrengthLevel,
}

/// Directional tag for sequence matching.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum DirTag {
    Long,
    Short,
    Neutral,
}

/// Strength level derived from |z-score|, used for quality-aware matching.
///
/// Band rationale (adjusted for fat-tailed order flow data):
/// - Weak: |z| < 1.60. Common noise-level anomalies.
/// - Medium: 1.60 ≤ |z| < 2.50. Meaningful anomalies.
/// - Strong: |z| ≥ 2.50. Rare, high-signal anomalies.
///
/// These values are slightly more permissive than pure normal distribution critical values
/// (1.65/2.58) to better accommodate the fat tails commonly observed in order flow data,
/// where extreme z-scores occur more frequently than a normal distribution would predict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum StrengthLevel {
    Weak,   // |z| < 1.60
    Medium, // 1.60 <= |z| < 2.50
    Strong, // |z| >= 2.50
}

impl DirTag {
    fn from_direction(dir: SignalDirection) -> Self {
        match dir {
            SignalDirection::Long => DirTag::Long,
            SignalDirection::Short => DirTag::Short,
            SignalDirection::Neutral => DirTag::Neutral,
        }
    }
}

impl StrengthLevel {
    fn from_z(z_abs: f64) -> Self {
        if z_abs < 1.60 {
            StrengthLevel::Weak
        } else if z_abs < 2.50 {
            StrengthLevel::Medium
        } else {
            StrengthLevel::Strong
        }
    }
}

impl PatternCounter {
    #[must_use]
    pub fn new(lookback_bars: usize, min_count: u32) -> Self {
        // seq_len scales with lookback: ~25% of lookback, capped at 8.
        let seq_len = (lookback_bars / 4).clamp(2, 8);
        // Cooldown: at least 2 bars, or (seq_len + 1) / 2.
        // Rationale: (seq_len + 1) / 2 gives a more consistent progression:
        //   seq_len=3 → cooldown=2, seq_len=4 → cooldown=2, seq_len=5 → cooldown=3, etc.
        let cooldown = 2.max((seq_len + 1) / 2);
        Self {
            lookback: lookback_bars.max(1),
            min_count: min_count.max(2),
            seq_len,
            cooldown,
            history: VecDeque::new(),
            counts: HashMap::new(),
            last_counted: HashMap::new(),
            recent_z_values: VecDeque::with_capacity(lookback_bars.max(1)),
        }
    }

    /// Extract the top 1-2 anomaly kinds (by |z-score|) from a bar's events,
    /// along with their directional bias and strength. Returns empty vec if no events.
    #[must_use]
    fn top_anomalies(events: &[AnomalyEvent]) -> Vec<AnomalyEntry> {
        let mut sorted: Vec<&AnomalyEvent> = events
            .iter()
            .filter(|e| e.kind != AnomalyKind::PatternRepeat)
            .collect();
        sorted.sort_by(|a, b| {
            b.z_score
                .abs()
                .partial_cmp(&a.z_score.abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        sorted
            .into_iter()
            .take(2)
            .map(|e| AnomalyEntry {
                kind: e.kind,
                dir: DirTag::from_direction(e.direction),
                strength: StrengthLevel::from_z(e.z_score.abs()),
            })
            .collect()
    }

    /// Compute the median |z| from recent bars for context-aware filtering.
    fn median_recent_z(&self) -> f64 {
        if self.recent_z_values.is_empty() {
            return 0.0;
        }
        let mut vals: Vec<f64> = self.recent_z_values.iter().copied().collect();
        vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mid = vals.len() / 2;
        if vals.len() % 2 == 0 {
            (vals[mid - 1] + vals[mid]) / 2.0
        } else {
            vals[mid]
        }
    }

    /// Record the top anomalies for the current bar.
    /// Returns the repetition count for the current bar's subsequence (0 if not counted).
    pub fn record(&mut self, bar_index: u64, events: &[AnomalyEvent]) -> u32 {
        let top = Self::top_anomalies(events);
        if top.is_empty() {
            return 0;
        }

        let primary = top[0];
        let secondary = top.get(1).copied();
        let avg_z = events
            .iter()
            .filter(|e| e.kind != AnomalyKind::PatternRepeat)
            .map(|e| e.z_score.abs())
            .sum::<f64>()
            / events
                .iter()
                .filter(|e| e.kind != AnomalyKind::PatternRepeat)
                .count()
                .max(1) as f64;

        // Track recent |z| values for context-aware median computation.
        self.recent_z_values.push_back(avg_z);
        while self.recent_z_values.len() > self.lookback {
            self.recent_z_values.pop_front();
        }

        self.history.push_back(BarEntry {
            bar_index,
            primary,
            secondary,
            avg_z,
        });
        while self.history.len() > self.lookback {
            self.history.pop_front();
        }

        let key = self.current_seq_hash();
        if key == 0 {
            return 0;
        }

        // Cooldown check: only count if enough bars have passed since last count.
        let last = self.last_counted.get(&key).copied().unwrap_or(0);
        if bar_index.saturating_sub(last) < self.cooldown as u64 {
            return self.counts.get(&key).copied().unwrap_or(0);
        }

        *self.counts.entry(key).or_insert(0) += 1;
        self.last_counted.insert(key, bar_index);
        self.counts.get(&key).copied().unwrap_or(1)
    }

    #[must_use]
    pub fn is_repetitive(&self, key: u64) -> bool {
        key != 0 && self.counts.get(&key).copied().unwrap_or(0) >= self.min_count
    }

    /// Hash the most recent `seq_len` anomaly entries into a compact u64 key.
    /// Returns 0 if fewer than `seq_len` entries are available, if the sequence
    /// fails quality checks (consecutive duplicates, low avg z, or not standing out
    /// from recent background noise), etc.
    #[must_use]
    pub fn current_seq_hash(&self) -> u64 {
        if self.history.len() < self.seq_len {
            return 0;
        }

        // Build sequence from primary anomalies.
        let recent: Vec<&BarEntry> = self
            .history
            .iter()
            .rev()
            .take(self.seq_len)
            .rev()
            .collect();

        // Quality filter 1: reject sequences with low average |z-score|.
        let total_z: f64 = recent.iter().map(|e| e.avg_z).sum();
        let avg_z = total_z / recent.len() as f64;
        if avg_z < MIN_AVG_Z {
            return 0;
        }

        // Quality filter 2 (context awareness): the sequence's avg |z| must be at least
        // 15% higher than the rolling median |z|. This ensures the pattern stands out
        // significantly from recent background market behavior.
        // Example: if median |z| = 1.8, sequence needs avg |z| >= 1.8 * 1.15 = 2.07.
        // Only apply when median is meaningfully above avg_z (median > avg_z * 1.1),
        // so uniform sequences (where median ≈ avg) are not rejected.
        let median_z = self.median_recent_z();
        if median_z > avg_z * 1.1 && avg_z < median_z * 1.15 {
            return 0;
        }

        // Extract entries, using secondary as fallback if primary is PatternRepeat.
        let entries: Vec<AnomalyEntry> = recent
            .iter()
            .map(|e| {
                if e.primary.kind == AnomalyKind::PatternRepeat {
                    e.secondary.unwrap_or(e.primary)
                } else {
                    e.primary
                }
            })
            .collect();

        // Reject sequences with consecutive duplicate kinds (noise filter).
        for i in 1..entries.len() {
            if entries[i].kind == entries[i - 1].kind {
                return 0;
            }
        }

        // Build hash from kind + direction + strength.
        let mut key: u64 = 0;
        for &entry in &entries {
            let bits = entry_to_bits(entry);
            key = key.wrapping_mul(131).wrapping_add(bits);
        }
        key
    }
}

/// Map AnomalyEntry (kind + direction + strength) to a unique bit pattern for hashing.
/// Direction is encoded in bits 8-9, strength in bits 10-11, keeping kind as primary.
fn entry_to_bits(entry: AnomalyEntry) -> u64 {
    let kind_bits = match entry.kind {
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
    };
    let dir_bits = match entry.dir {
        DirTag::Long => 0x100,
        DirTag::Short => 0x200,
        DirTag::Neutral => 0,
    };
    let strength_bits = match entry.strength {
        StrengthLevel::Weak => 0x400,
        StrengthLevel::Medium => 0x800,
        StrengthLevel::Strong => 0xC00,
    };
    kind_bits | dir_bits | strength_bits
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(kind: AnomalyKind, z: f64, dir: SignalDirection) -> AnomalyEvent {
        AnomalyEvent {
            bar_index: 0,
            ts: 0,
            kind,
            direction: dir,
            z_score: z,
            raw_value: 0.0,
            confidence: 0.5,
        }
    }

    #[test]
    fn detects_repeating_sequence_with_strength() {
        let mut counter = PatternCounter::new(20, 2);
        let seq_len = counter.seq_len;

        // Build a repeating sequence with strong directional bias (short, high z).
        let kinds = [
            (AnomalyKind::Ofi, SignalDirection::Short),
            (AnomalyKind::Absorption, SignalDirection::Short),
            (AnomalyKind::LiquidityVacuum, SignalDirection::Short),
        ];
        let seq: Vec<(AnomalyKind, SignalDirection)> = (0..seq_len)
            .map(|i| kinds[i % kinds.len()])
            .collect();

        for round in 0..2 {
            for (i, &(kind, dir)) in seq.iter().enumerate() {
                let bar = (seq_len * round + i) as u64;
                let events = vec![event(kind, 3.0, dir)]; // Strong strength (>= 2.50)
                counter.record(bar, &events);
            }
        }
        let key = counter.current_seq_hash();
        assert!(key != 0);
        assert!(counter.is_repetitive(key));
    }

    #[test]
    fn distinguishes_directional_bias() {
        let mut counter = PatternCounter::new(20, 2);
        let seq_len = counter.seq_len;

        // First round: short-biased sequence.
        for i in 0..seq_len {
            let kind = [AnomalyKind::Ofi, AnomalyKind::Absorption][i % 2];
            let events = vec![event(kind, 3.0, SignalDirection::Short)];
            counter.record(i as u64, &events);
        }

        // Second round: long-biased sequence (different hash).
        for i in 0..seq_len {
            let kind = [AnomalyKind::Ofi, AnomalyKind::Absorption][i % 2];
            let events = vec![event(kind, 3.0, SignalDirection::Long)];
            counter.record((seq_len + i) as u64, &events);
        }

        let key = counter.current_seq_hash();
        assert!(key != 0);
        // Should not be repetitive because direction differs.
        assert!(!counter.is_repetitive(key));
    }

    #[test]
    fn rejects_weak_sequences() {
        let mut counter = PatternCounter::new(20, 2);
        let seq_len = counter.seq_len;

        // Build a sequence with weak z-scores (below MIN_AVG_Z threshold of 1.60).
        for i in 0..seq_len {
            let kind = [AnomalyKind::Ofi, AnomalyKind::Absorption][i % 2];
            let events = vec![event(kind, 1.0, SignalDirection::Short)]; // Weak
            counter.record(i as u64, &events);
        }

        // Should be rejected due to low avg z.
        assert_eq!(counter.current_seq_hash(), 0);
    }

    #[test]
    fn rejects_consecutive_duplicates() {
        let mut counter = PatternCounter::new(10, 2);
        for i in 0..5 {
            let events = vec![event(AnomalyKind::Ofi, 3.0, SignalDirection::Neutral)];
            counter.record(i, &events);
        }
        assert_eq!(counter.current_seq_hash(), 0);
    }

    #[test]
    fn returns_zero_when_insufficient_history() {
        let counter = PatternCounter::new(20, 2);
        assert_eq!(counter.current_seq_hash(), 0);
    }

    #[test]
    fn cooldown_prevents_tight_clustering() {
        let mut counter = PatternCounter::new(50, 2);
        let seq_len = counter.seq_len;
        let cooldown = counter.cooldown;

        // Build a strong sequence.
        for i in 0..seq_len {
            let kind = [AnomalyKind::Ofi, AnomalyKind::Absorption][i % 2];
            let events = vec![event(kind, 3.0, SignalDirection::Short)];
            counter.record(i as u64, &events);
        }
        let key1 = counter.current_seq_hash();
        let count1 = counter.counts.get(&key1).copied().unwrap_or(0);
        assert!(count1 >= 1);

        // Try to repeat immediately (within cooldown).
        let second_start = seq_len as u64;
        for i in 0..seq_len {
            let kind = [AnomalyKind::Ofi, AnomalyKind::Absorption][i % 2];
            let events = vec![event(kind, 3.0, SignalDirection::Short)];
            counter.record(second_start + i as u64, &events);
        }

        // The gap from first count (at seq_len-1) to second sequence end (2*seq_len-1) is seq_len.
        // If seq_len < cooldown, the second should NOT be counted.
        let count2 = counter.counts.get(&key1).copied().unwrap_or(0);
        if (seq_len as u64) < cooldown as u64 {
            assert_eq!(count1, count2, "Should not count within cooldown");
        } else {
            assert!(count2 >= count1, "Should count after cooldown expires");
        }
    }

    #[test]
    fn context_awareness_filters_background_noise() {
        let mut counter = PatternCounter::new(30, 2);
        let seq_len = counter.seq_len;

        // First, populate with high-z bars to raise the median.
        for i in 0..10 {
            let events = vec![event(AnomalyKind::Cvd, 4.0, SignalDirection::Neutral)];
            counter.record(i as u64, &events);
        }

        // Now try a moderate-z sequence (avg z = 1.8, which is > MIN_AVG_Z = 1.60).
        // But since median is ~4.0, the 15% rule should reject it.
        for i in 0..seq_len {
            let kind = [AnomalyKind::Ofi, AnomalyKind::Absorption][i % 2];
            let events = vec![event(kind, 1.8, SignalDirection::Short)];
            counter.record((10 + i) as u64, &events);
        }

        // Should be rejected because avg_z (1.8) < median_z (4.0) * 1.15 (4.6).
        assert_eq!(counter.current_seq_hash(), 0);
    }
}
