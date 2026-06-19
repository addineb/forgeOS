//! Causal engine: structural order-flow templates that answer
//! "why should this situation force someone to trade?" instead of
//! "is this bar statistically unusual?".
//!
//! Reuses `FeatureExtractor` (feature math) and a thin rolling buffer for
//! scale estimation. Does NOT touch Mahalanobis, NullEdge, or PatternCounter.
//!
//! First template: Absorption → Exhaustion → Reversal.
//! Test ONE philosophy at a time.

//! Causal engine: structural order-flow templates that answer
//! "why should this situation force someone to trade?" instead of
//! "is this bar statistically unusual?".
//!
//! Reuses `FeatureExtractor` (feature math) and a thin rolling buffer for
//! scale estimation. Does NOT touch Mahalanobis, NullEdge, or PatternCounter.
//!
//! First template: Absorption → Exhaustion → Reversal.
//! Test ONE philosophy at a time.

pub mod confidence;
pub mod engine;
pub mod rate_limit;
pub mod template;
pub mod templates;

pub use engine::{CausalEngine, CausalEngineOutput};

use std::collections::VecDeque;

/// Which engine implementation runs when `validate` or the library is called.
///
/// Default is `Legacy` so every existing caller behaves bit-identically.
/// Flip to `Causal` after the first A/B validation cycle produces numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineMode {
    /// Mahalanobis + z-score + FDR + PatternRepeat (existing behavior).
    Legacy,
    /// Causal template matching (new).
    Causal,
}

impl Default for EngineMode {
    fn default() -> Self {
        EngineMode::Legacy
    }
}

/// One step inside a causal template's chain.
///
/// Example for Absorption → Exhaustion → Reversal:
/// `Step1Pressure` (heavy CVD), `Step2Absorption` (price holds),
/// `Step3Deceleration` (CVD decays given absorption), `Step4Reversal` (forced exit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Step {
    Step1Pressure,
    Step2Absorption,
    Step3Deceleration,
    Step4Reversal,
}

/// What a template returned for the most recent bar.
#[derive(Debug, Clone)]
pub struct TemplateOutcome {
    pub template_id: &'static str,
    /// Which step is the highest completed step on this bar.
    /// `None` if precondition gate (depth) failed.
    pub completed_step: Option<Step>,
    /// Number of steps completed (0..=4).
    pub steps_passed: u32,
    /// Total steps in this template (for causal_completeness ratio).
    pub steps_total: u32,
    /// Per-step z-like strength, indexed by `Step` enum (0..=3).
    /// Used to calibrate confidence beyond bare completeness.
    pub step_strengths: [f64; 4],
    /// Direction the template is betting on: Long if absorption is on bid side (bought into bid support, expect up),
    /// Short if absorption is on ask side (sold into ask support, expect down).
    pub direction: CausalDirection,
    /// Bars between step 1 firing and step 3 completing. Used to set hold period.
    pub bars_in_story: u32,
}

/// Directional bet from a template match.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CausalDirection {
    /// Absorption on bid side — buyer is being absorbed, expect price up.
    Long,
    /// Absorption on ask side — seller is being absorbed, expect price down.
    Short,
    /// Template did not commit to a direction yet.
    Neutral,
}

/// Signal emitted by the CausalEngine when a template completes enough steps
/// to act on.
///
/// `completed_step` is the highest step reached. `confidence` is the
/// causal completeness (steps_passed / steps_total) adjusted by per-step
/// strength. `hold_bars` is the suggested hold from the template's
/// `bars_in_story` field.
#[derive(Debug, Clone)]
pub struct CausalSignal {
    pub bar_index: u64,
    pub ts: u64,
    pub template_id: &'static str,
    pub direction: CausalDirection,
    pub confidence: f64,
    pub description: String,
    pub bars_in_story: u32,
    pub hold_bars: u32,
    pub expected_move_bps: f64,
    pub completed_step: Step,
    pub steps_passed: u32,
    pub steps_total: u32,
    pub step_strengths: [f64; 4],
}

/// Per-template parameters the user can tune from `EngineConfig`.
///
/// All thresholds are z-like units (multiples of recent rolling std for that feature).
/// Defaults are educated guesses — the first A/B validation will tell us which need adjustment.
#[derive(Debug, Clone)]
pub struct AbsorptionReversalParams {
    /// Step 1: |cvd_delta| > this * rolling_cvd_std fires the pressure step.
    pub cvd_pressure_threshold: f64,
    /// Step 2: |absorption| > this fires the absorption step.
    pub absorption_hold_threshold: f64,
    /// Step 3: |cvd_now| < this * |cvd_step1| fires the deceleration step.
    pub deceleration_ratio: f64,
    /// Precondition gate: defending-side depth_imbalance must be above this.
    /// If one-sided and thin, absorption is meaningless (nothing to absorb into).
    pub depth_precondition_min: f64,
    /// Template must complete step 3 within this many bars of step 1.
    pub max_step1_to_signal_bars: u32,
    /// Hold period after signal.
    pub hold_bars: u32,
    /// Expected move coefficient (hold period * this = expected_move_bps).
    pub expected_move_coeff: f64,
}

impl Default for AbsorptionReversalParams {
    fn default() -> Self {
        Self {
            cvd_pressure_threshold: 0.6,
            absorption_hold_threshold: 0.3,
            deceleration_ratio: 0.5,
            depth_precondition_min: 0.25,
            max_step1_to_signal_bars: 6,
            hold_bars: 8,
            expected_move_coeff: 3.0,
        }
    }
}

/// One template's settings bundled with the CausalEngine config.
#[derive(Debug, Clone)]
pub struct CausalTemplatesConfig {
    pub absorption_reversal: AbsorptionReversalParams,
    /// Per-100-bars signal rate cap, enforced across all templates combined.
    pub signal_rate_limit: f64,
    /// Rolling window (in bars) used for cvd_mean/cvd_std estimation.
    pub lookback_bars: usize,
}

impl Default for CausalTemplatesConfig {
    fn default() -> Self {
        Self {
            absorption_reversal: AbsorptionReversalParams::default(),
            signal_rate_limit: 8.0,
            lookback_bars: 50,
        }
    }
}

/// Tiny rolling buffer for a single scalar feature (currently CVD delta).
/// Separate from `RollingFeatureWindow` (which is multivariate, used by legacy)
/// to keep the causal engine self-contained.
#[derive(Debug)]
pub struct CausalRollingBuf {
    buf: VecDeque<f64>,
    cap: usize,
}

impl CausalRollingBuf {
    #[must_use]
    pub fn new(cap: usize) -> Self {
        Self {
            buf: VecDeque::with_capacity(cap.max(1)),
            cap: cap.max(1),
        }
    }

    pub fn push(&mut self, x: f64) {
        if self.buf.len() >= self.cap {
            self.buf.pop_front();
        }
        self.buf.push_back(x);
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.buf.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.buf.is_empty()
    }

    /// Sample mean (returns 0 if buffer is empty).
    #[must_use]
    pub fn mean(&self) -> f64 {
        if self.buf.is_empty() {
            return 0.0;
        }
        let n = self.buf.len() as f64;
        self.buf.iter().sum::<f64>() / n
    }

    /// Sample std (returns 0 if buffer has <2 elements).
    /// Computed as sqrt(sum((x - mean)^2) / (n - 1)) — biased-corrected.
    #[must_use]
    pub fn std(&self) -> f64 {
        if self.buf.len() < 2 {
            return 0.0;
        }
        let m = self.mean();
        let n = self.buf.len() as f64;
        let var = self.buf.iter().map(|x| (x - m).powi(2)).sum::<f64>() / (n - 1.0);
        var.sqrt()
    }

    /// Last value pushed.
    #[must_use]
    pub fn last(&self) -> f64 {
        self.buf.back().copied().unwrap_or(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rolling_buf_mean_std() {
        let mut b = CausalRollingBuf::new(5);
        for x in [1.0, 2.0, 3.0, 4.0, 5.0] {
            b.push(x);
        }
        assert!((b.mean() - 3.0).abs() < 1e-9);
        // Sample std of [1,2,3,4,5] = sqrt(2.5) ≈ 1.5811
        assert!((b.std() - 2.5_f64.sqrt()).abs() < 1e-9);
    }

    #[test]
    fn rolling_buf_respects_cap() {
        let mut b = CausalRollingBuf::new(3);
        for x in [1.0, 2.0, 3.0, 4.0, 5.0] {
            b.push(x);
        }
        assert_eq!(b.len(), 3);
        assert_eq!(b.last(), 5.0);
        // mean of [3,4,5] = 4.0
        assert!((b.mean() - 4.0).abs() < 1e-9);
    }

    #[test]
    fn rolling_buf_empty() {
        let b = CausalRollingBuf::new(5);
        assert_eq!(b.len(), 0);
        assert_eq!(b.mean(), 0.0);
        assert_eq!(b.std(), 0.0);
        assert_eq!(b.last(), 0.0);
    }

    #[test]
    fn engine_mode_default_is_legacy() {
        assert_eq!(EngineMode::default(), EngineMode::Legacy);
    }
}
