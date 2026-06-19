//! Causal template trait.
//!
//! A `CausalTemplate` represents one known mechanical setup in order flow
//! microstructure. It evaluates the current bar against the rolling bar-feature
//! history and returns a `TemplateOutcome` describing which steps of its causal
//! chain have fired.
//!
//! First template: `Absorption → Exhaustion → Reversal`
//! (see `templates::absorption_reversal`).
//!
//! ## Design rules
//!
//! - Templates receive `&[BarFeatures]` (the rolling history, oldest first).
//!   They do NOT touch `VolumeBar` directly, do NOT touch the Mahalanobis
//!   detector, and do NOT compute features themselves. Feature math lives
//!   in `features.rs` and runs once per bar in `CausalEngine::observe`.
//! - Templates are stateless across bars except for step-1 timestamps they
//!   need to track (e.g. "when did step 1 fire?"). This is kept inside the
//!   template instance so `CausalEngine` can iterate templates uniformly.
//! - Each template decides its own `completed_step` and `direction`.
//! - Threshold parameters live on `AbsorptionReversalParams` and friends, NOT
//!   hard-coded in the template body. This makes A/B parameter sweeps cheap.

use crate::causal::{CausalDirection, Step, TemplateOutcome};
use crate::types::BarFeatures;

/// What a template needs to evaluate the current bar.
///
/// `history` is the rolling window of `BarFeatures` ending at the current bar,
/// oldest first. `cvd_std` is the rolling std of `cvd_delta` over the engine's
/// lookback window — passed in so each template doesn't re-estimate it.
#[derive(Debug, Clone)]
pub struct TemplateInput<'a> {
    pub history: &'a [BarFeatures],
    pub cvd_std: f64,
    pub cvd_mean: f64,
}

/// One causal template: ordered multi-bar setup that emits a `TemplateOutcome`.
///
/// Implementors should be cheap to evaluate (no allocation in the hot path).
pub trait CausalTemplate {
    /// Stable string id used in logs / scorecards.
    fn id(&self) -> &'static str;

    /// Number of steps in this template's causal chain (for completeness ratio).
    fn step_count(&self) -> u32;

    /// Evaluate the current bar against the rolling history.
    ///
    /// Returns `Some(outcome)` if at least step 1 fired within the template's
    /// recency window, `None` otherwise.
    fn evaluate(&mut self, input: &TemplateInput) -> Option<TemplateOutcome>;
}

/// Helper used by templates: does the bar pass the precondition gate
/// (depth on the defending side is above `min`)?
///
/// `depth_on_defending_side` is the absolute value of the depth-imbalance
/// aligned with the direction of the absorption (e.g. for a long template,
/// it's `bar.bid_vol_top10 / (bid_vol_top10 + ask_vol_top10)`).
#[inline]
#[must_use]
pub fn passes_depth_precondition(depth_on_defending_side: f64, min: f64) -> bool {
    depth_on_defending_side >= min
}

/// Helper: check if step 1 is still within the recency window from "now".
#[inline]
#[must_use]
pub fn step1_within_window(bars_since_step1: u32, max_bars: u32) -> bool {
    bars_since_step1 <= max_bars
}

/// Map a `CausalDirection` to the human-readable label used in `validate`.
#[inline]
#[must_use]
pub fn direction_label(dir: CausalDirection) -> &'static str {
    match dir {
        CausalDirection::Long => "long",
        CausalDirection::Short => "short",
        CausalDirection::Neutral => "neutral",
    }
}

/// Pick `Long` or `Short` from a positive (long) / negative (short) signed input.
/// Returns `Neutral` on zero.
#[inline]
#[must_use]
pub fn signed_direction(x: f64) -> CausalDirection {
    if x > 0.0 {
        CausalDirection::Long
    } else if x < 0.0 {
        CausalDirection::Short
    } else {
        CausalDirection::Neutral
    }
}

/// Format the per-step strengths into a debug string for the validate output.
///
/// Example: `steps=1,2,3,4 str=1.20,0.85,0.62,0.00`
#[must_use]
pub fn format_step_strengths(strengths: &[f64; 4]) -> String {
    let mut s = String::with_capacity(64);
    s.push_str("str=");
    for (i, v) in strengths.iter().enumerate() {
        if i > 0 {
            s.push(',');
        }
        s.push_str(&format!("{:.2}", v));
    }
    s
}

/// Step name used in `validate` output and per-step CSV columns.
#[inline]
#[must_use]
pub fn step_name(step: Step) -> &'static str {
    match step {
        Step::Step1Pressure => "pressure",
        Step::Step2Absorption => "absorption",
        Step::Step3Deceleration => "deceleration",
        Step::Step4Reversal => "reversal",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn depth_precondition_threshold() {
        assert!(passes_depth_precondition(0.30, 0.25));
        assert!(!passes_depth_precondition(0.10, 0.25));
        assert!(passes_depth_precondition(0.25, 0.25));
    }

    #[test]
    fn step1_window_inclusive() {
        assert!(step1_within_window(0, 6));
        assert!(step1_within_window(6, 6));
        assert!(!step1_within_window(7, 6));
    }

    #[test]
    fn signed_direction_picks_side() {
        assert_eq!(signed_direction(1.0), CausalDirection::Long);
        assert_eq!(signed_direction(-1.0), CausalDirection::Short);
        assert_eq!(signed_direction(0.0), CausalDirection::Neutral);
    }

    #[test]
    fn direction_label_strings() {
        assert_eq!(direction_label(CausalDirection::Long), "long");
        assert_eq!(direction_label(CausalDirection::Short), "short");
        assert_eq!(direction_label(CausalDirection::Neutral), "neutral");
    }

    #[test]
    fn step_names_match_template() {
        assert_eq!(step_name(Step::Step1Pressure), "pressure");
        assert_eq!(step_name(Step::Step2Absorption), "absorption");
        assert_eq!(step_name(Step::Step3Deceleration), "deceleration");
        assert_eq!(step_name(Step::Step4Reversal), "reversal");
    }

    #[test]
    fn step_strengths_format() {
        let s = format_step_strengths(&[1.2, 0.85, 0.62, 0.0]);
        assert_eq!(s, "str=1.20,0.85,0.62,0.00");
    }
}
