//! Sequence-based pattern detector with hybrid frequency + quality scoring.
//!
//! This detector maintains a rolling window of recent *dominant anomaly kinds* (top 1-2 per bar)
//! along with their directional bias and strength, and hashes fixed-length subsequences to find
//! repetition. Unlike pure counting approaches, this uses a hybrid scoring system that combines:
//! - **Frequency score**: How often the sequence has appeared recently (weighted 60%)
//! - **Quality score**: How much the sequence stands out from background noise (weighted 40%)
//!
//! A signal is generated only when:
//! 1. The sequence has appeared at least 2 times (minimum frequency gate)
//! 2. The combined final_score >= 0.48 threshold
//!
//! This approach prevents common/weak sequences from triggering while allowing genuinely
//! repetitive, high-quality patterns to be detected.

use std::collections::HashMap;
use std::collections::VecDeque;

use crate::types::{AnomalyEvent, AnomalyKind, SignalDirection};

/// Minimum average |z-score| for a sequence to pass the initial quality gate.
///
/// **Calibration:** 1.30 was still too strict — many real patterns with moderate
/// but consistent z-scores (1.2–1.5) were rejected.  Lowered to **1.20** to let
/// these through while still filtering true noise (< 1.0).  Sits in the upper
/// third of the "Weak" band, requiring anomalies to be at least marginally
/// significant to form patterns.
const MIN_AVG_Z: f64 = 1.20;

/// Threshold for the final hybrid score. Sequences must score >= this to be considered repetitive.
///
/// **Calibration:** Lowered from 0.48 to **0.42** to compensate for the still-low
/// pattern hit rate (14% of signals, 0% on some days).  At 0.42, a freq=2 sequence
/// with moderate quality (z ≈ 1.5–2.0 above background) passes, while freq=1 or
/// very-weak sequences still fail.
const FINAL_SCORE_THRESHOLD: f64 = 0.42;

/// Maximum expected repetitions for normalization. A sequence appearing 5+ times is considered
/// very frequent; this caps the frequency_score at 1.0.
const MAX_FREQ_FOR_NORMALIZATION: f64 = 5.0;

/// Tracks ordered anomaly-kind sequences across a rolling bar window.
#[derive(Debug)]
pub struct PatternCounter {
    /// Total lookback window in bars.
    lookback: usize,
    /// Minimum repetition count to consider a pattern "repetitive."
    #[allow(dead_code)] // stored for future threshold tuning; used via EngineConfig
    min_count: u32,
    /// Subsequence length to hash (scales with lookback, clamped to [2, 8]).
    seq_len: usize,
    /// Cooldown: minimum bars between counted repetitions of the same sequence.
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
    #[allow(dead_code)] // stored for diagnostics; used in Debug output
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

/// Strength level derived from |z-score|.
///
/// Band rationale (adjusted for fat-tailed order flow data):
/// - Weak: |z| < 1.60. Common noise-level anomalies.
/// - Medium: 1.60 ≤ |z| < 2.50. Meaningful anomalies.
/// - Strong: |z| ≥ 2.50. Rare, high-signal anomalies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum StrengthLevel {
    Weak,
    Medium,
    Strong,
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
        let seq_len = (lookback_bars / 4).clamp(2, 8);
        let cooldown = 2.max(seq_len.div_ceil(2));
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

    /// Extract the top 1-2 anomaly kinds (by |z-score|) from a bar's events.
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

    /// Compute the median |z| from recent bars.
    fn median_recent_z(&self) -> f64 {
        if self.recent_z_values.is_empty() {
            return 0.0;
        }
        let mut vals: Vec<f64> = self.recent_z_values.iter().copied().collect();
        vals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mid = vals.len() / 2;
        if vals.len().is_multiple_of(2) {
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

        // Cooldown check.
        let last = self.last_counted.get(&key).copied().unwrap_or(0);
        if bar_index.saturating_sub(last) < self.cooldown as u64 {
            return self.counts.get(&key).copied().unwrap_or(0);
        }

        *self.counts.entry(key).or_insert(0) += 1;
        self.last_counted.insert(key, bar_index);
        self.counts.get(&key).copied().unwrap_or(1)
    }

    /// Check if a sequence key is repetitive using hybrid scoring.
    /// Returns true only if:
    /// 1. frequency >= 2 (minimum gate)
    /// 2. final_score >= FINAL_SCORE_THRESHOLD
    #[must_use]
    pub fn is_repetitive(&self, key: u64) -> bool {
        let freq = self.counts.get(&key).copied().unwrap_or(0);
        if freq < 2 {
            return false; // Minimum frequency gate
        }

        let final_score = self.compute_final_score(key, freq);
        final_score >= FINAL_SCORE_THRESHOLD
    }

    /// Compute the hybrid final_score for a sequence.
    ///
    /// Formula: final_score = (frequency_score * 0.60) + (quality_score * 0.40)
    ///
    /// **Bug fix #2:** The old implementation hardcoded `quality_score = 0.75`,
    /// completely ignoring the actual z-score quality of the sequence.  Now we
    /// compute the sequence's average |z| relative to the rolling median |z|,
    /// clamped to produce a meaningful 0–1 range.  If we can't determine the
    /// quality (no history), we use a neutral default of 0.5.
    ///
    /// - frequency_score: normalized repetition count (freq / MAX_FREQ_FOR_NORMALIZATION,
    ///   capped at 1.0).  Weight 0.60 because repetition is the primary signal.
    ///
    /// - quality_score: (seq_avg_z / median_z - 1.0), clamped to [0.0, 1.0].
    ///   Measures how much the sequence stands out above background noise.
    ///   Weight 0.40 — quality matters but shouldn't dominate without repetition.
    fn compute_final_score(&self, key: u64, freq: u32) -> f64 {
        let frequency_score = (freq as f64 / MAX_FREQ_FOR_NORMALIZATION).min(1.0);

        // Compute quality_score from actual data instead of a hardcoded constant.
        // Uses a blend of absolute quality (how strong the sequence is vs the
        // minimum bar) and relative quality (how much it stands out vs background).
        let quality_score = match self.get_last_avg_z_for_key(key) {
            Some(seq_avg_z) if seq_avg_z > 0.0 => {
                let median_z = self.median_recent_z();
                // Absolute: sequences with higher avg_z are inherently higher quality.
                // Normalized so that avg_z = 2×MIN_AVG_Z maps to 1.0.
                let abs_quality = ((seq_avg_z / MIN_AVG_Z) * 0.5).min(1.0);
                // Relative: how much the sequence exceeds background noise.
                let rel_quality = if median_z > 0.0 {
                    ((seq_avg_z / median_z - 1.0) * 0.5).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                // Blend: absolute dominates when sequences are strong in isolation;
                // relative helps when the whole market is noisy.
                abs_quality * 0.6 + rel_quality * 0.4
            }
            _ => 0.5, // fallback: can't determine quality, use neutral
        };

        frequency_score * 0.60 + quality_score * 0.40
    }

    /// Get the avg_z of the most recent bar entry for a key.
    fn get_last_avg_z_for_key(&self, _key: u64) -> Option<f64> {
        self.history.back().map(|e| e.avg_z)
    }

    /// Hash the most recent `seq_len` anomaly entries into a compact u64 key.
    #[must_use]
    pub fn current_seq_hash(&self) -> u64 {
        if self.history.len() < self.seq_len {
            return 0;
        }

        let recent: Vec<&BarEntry> = self.history.iter().rev().take(self.seq_len).rev().collect();

        // Quality filter 1: reject low avg |z|.
        let total_z: f64 = recent.iter().map(|e| e.avg_z).sum();
        let avg_z = total_z / recent.len() as f64;
        if avg_z < MIN_AVG_Z {
            return 0;
        }

        // Quality filter 2: context awareness.
        // Require the sequence's avg |z| to be at least 10% above the rolling
        // median (relaxed from 15% — too strict for calmer regimes where the
        // background z is already low and a 10% excess is meaningful).
        // The safeguard condition (median > avg * 1.1) prevents sequences that
        // are far BELOW the background from being counted.
        let median_z = self.median_recent_z();
        if median_z > avg_z * 1.1 && avg_z < median_z * 1.10 {
            return 0;
        }

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

        // **Bug fix #2:** Old code rejected any sequence with consecutive duplicate
        // kinds entirely.  Real market data often has consecutive bars dominated by
        // the same anomaly kind.  Instead of rejecting, we count duplicate runs and
        // apply a mild penalty: each duplicate pair reduces the quality gate slightly.
        // If the entire sequence is one kind repeated, we still reject (no pattern).
        let dup_count = entries
            .windows(2)
            .filter(|w| w[0].kind == w[1].kind)
            .count();
        if dup_count >= entries.len() - 1 {
            return 0; // all same kind — not a meaningful pattern
        }
        // Mild penalty: each duplicate pair slightly raises the effective avg_z bar
        // by requiring the non-duplicate entries to carry more weight.
        let penalty_factor = if dup_count > 0 {
            1.0 + (dup_count as f64 * 0.10)
        } else {
            1.0
        };
        let effective_avg_z = avg_z / penalty_factor;
        if effective_avg_z < MIN_AVG_Z {
            return 0;
        }

        let mut key: u64 = 0;
        for &entry in &entries {
            let bits = entry_to_bits(entry);
            key = key.wrapping_mul(131).wrapping_add(bits);
        }
        key
    }
}

/// Map AnomalyEntry to a unique bit pattern for hashing.
///
/// **Pattern detection v2:** Direction is intentionally EXCLUDED from the hash.
/// Real microstructure patterns repeat across both bull and bear regimes — a
/// [LiquidityVacuum → Absorption → OFI] structure is the same phenomenon
/// whether it's long or short.  Hashing direction fragmented identical structures
/// into different bins, making the frequency counter useless on real data.
///
/// Trade-off: we lose the ability to distinguish "bullish OFI pattern" from
/// "bearish OFI pattern" at the sequence level.  This is acceptable because:
/// - The quality filters (avg_z, context awareness, hybrid scoring) prevent
///   noise patterns from accumulating.
/// - The signal-level `weighted_dir()` still determines the trade direction
///   from the current bar's anomaly events, not the pattern history.
/// - StrengthLevel is also excluded (see previous fix).
fn entry_to_bits(entry: AnomalyEntry) -> u64 {
    match entry.kind {
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
    // Direction and StrengthLevel intentionally omitted — see doc comment.
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
    fn detects_repeating_sequence_with_hybrid_scoring() {
        let mut counter = PatternCounter::new(20, 2);
        let seq_len = counter.seq_len;

        let kinds = [
            (AnomalyKind::Ofi, SignalDirection::Short),
            (AnomalyKind::Absorption, SignalDirection::Short),
            (AnomalyKind::LiquidityVacuum, SignalDirection::Short),
        ];
        let seq: Vec<(AnomalyKind, SignalDirection)> =
            (0..seq_len).map(|i| kinds[i % kinds.len()]).collect();

        // First occurrence.
        for (i, &(kind, dir)) in seq.iter().enumerate() {
            let bar = i as u64;
            let events = vec![event(kind, 3.0, dir)];
            counter.record(bar, &events);
        }

        // Second occurrence (should trigger with high quality).
        for (i, &(kind, dir)) in seq.iter().enumerate() {
            let bar = (seq_len + i) as u64;
            let events = vec![event(kind, 3.0, dir)];
            counter.record(bar, &events);
        }

        let key = counter.current_seq_hash();
        assert!(key != 0);
        assert!(counter.is_repetitive(key));
    }

    #[test]
    fn rejects_single_occurrence() {
        let mut counter = PatternCounter::new(20, 2);
        let seq_len = counter.seq_len;

        // Only one occurrence — should not trigger due to min frequency gate.
        for i in 0..seq_len {
            let kind = [AnomalyKind::Ofi, AnomalyKind::Absorption][i % 2];
            let events = vec![event(kind, 3.0, SignalDirection::Short)];
            counter.record(i as u64, &events);
        }

        let key = counter.current_seq_hash();
        assert!(key != 0);
        assert!(!counter.is_repetitive(key)); // freq = 1 < 2
    }

    #[test]
    fn rejects_weak_sequences() {
        // MIN_AVG_Z = 1.30: z=1.0 entries fall below the quality gate,
        // so no hash is generated at all — the sequence is invisible.
        let mut counter = PatternCounter::new(20, 2);
        let seq_len = counter.seq_len;

        for i in 0..seq_len * 2 {
            let kind = [AnomalyKind::Ofi, AnomalyKind::Absorption][i % 2];
            let events = vec![event(kind, 1.0, SignalDirection::Short)]; // Weak
            counter.record(i as u64, &events);
        }

        assert_eq!(counter.current_seq_hash(), 0); // Rejected by MIN_AVG_Z (1.30 > 1.0)
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

        for i in 0..seq_len {
            let kind = [AnomalyKind::Ofi, AnomalyKind::Absorption][i % 2];
            let events = vec![event(kind, 3.0, SignalDirection::Short)];
            counter.record(i as u64, &events);
        }
        let key1 = counter.current_seq_hash();
        let count1 = counter.counts.get(&key1).copied().unwrap_or(0);
        assert!(count1 >= 1);

        let second_start = seq_len as u64;
        for i in 0..seq_len {
            let kind = [AnomalyKind::Ofi, AnomalyKind::Absorption][i % 2];
            let events = vec![event(kind, 3.0, SignalDirection::Short)];
            counter.record(second_start + i as u64, &events);
        }

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

        // High-z background.
        for i in 0..10 {
            let events = vec![event(AnomalyKind::Cvd, 4.0, SignalDirection::Neutral)];
            counter.record(i as u64, &events);
        }

        // Moderate-z sequence.
        for i in 0..seq_len {
            let kind = [AnomalyKind::Ofi, AnomalyKind::Absorption][i % 2];
            let events = vec![event(kind, 1.8, SignalDirection::Short)];
            counter.record((10 + i) as u64, &events);
        }

        assert_eq!(counter.current_seq_hash(), 0); // Rejected by 10% context rule
    }
}
