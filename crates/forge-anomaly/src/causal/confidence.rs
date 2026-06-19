//! Confidence formula for causal templates.
//!
//! Replaces `engine::calc_confidence()` which used anomaly-count additive logic.
//! Confidence here rises with **causal completeness** (how many of the
//! template's steps fired), NOT with how many anomaly kinds coincidentally
//! fired together.
//!
//! ## Formula
//!
//! ```text
//! completeness_ratio = steps_passed / steps_total
//! per_step_strength = average of present step strengths (capped at 1.5)
//! strength_boost    = (per_step_strength - 1.0).clamp(0.0, 0.20)
//! floor             = 0.10 * (1.0 - completeness_ratio)   // small floor
//!                     // so a partial-completion signal isn't immediately zero,
//!                     // but it does NOT exceed a half-complete signal.
//!
//! confidence = (completeness_ratio + strength_boost + floor).clamp(0.0, 1.0)
//! ```
//!
//! Examples (4-step template):
//! - 4/4 steps at strength 1.0 → 1.00 + 0.00 + 0.00 = 1.00
//! - 4/4 steps at strength 1.3 → 1.00 + 0.20 + 0.00 = 1.00 (capped)
//! - 3/4 steps at strength 1.0 → 0.75 + 0.00 + 0.025 = 0.775
//! - 2/4 steps at strength 1.2 → 0.50 + 0.04 + 0.10 = 0.64
//! - 1/4 steps at strength 1.5 → 0.25 + 0.10 + 0.225 = 0.575
//!
//! Why this is better than anomaly-count:
//! - 3 anomalies firing together no longer automatically boosts confidence.
//!   They have to map to actual causal steps.
//! - Strong steps beat weak steps; strength matters beyond mere count.
//! - A signal that completes only step 1 is clearly weaker than one that
//!   completes steps 1-3, even if step 4 hasn't fired yet.

/// Compute causal completeness as a 0..=1 confidence score.
///
/// `steps_passed` must be in `[0, steps_total]` (callers must clamp).
/// `step_strengths[i]` is the strength of step `i+1` (0 if absent).
/// Strengths are expected in z-like units (≈1.0 baseline, >1.0 strong).
#[inline]
#[must_use]
pub fn causal_completeness(steps_passed: u32, steps_total: u32, step_strengths: &[f64]) -> f64 {
    if steps_total == 0 {
        return 0.0;
    }
    let ratio = (steps_passed as f64) / (steps_total as f64);
    let avg_strength = average_present_strength(step_strengths, steps_passed);
    let strength_boost = (avg_strength - 1.0).clamp(0.0, 0.20);
    let floor = 0.10 * (1.0 - ratio);
    (ratio + strength_boost + floor).clamp(0.0, 1.0)
}

/// Average the strengths of steps that actually fired (indices 0..steps_passed).
///
/// If fewer strengths are present than `steps_passed`, missing ones default to 1.0
/// (neutral). Returns 1.0 if no steps fired.
fn average_present_strength(strengths: &[f64], steps_passed: u32) -> f64 {
    if steps_passed == 0 {
        return 1.0;
    }
    let mut sum = 0.0;
    let n = steps_passed as usize;
    for i in 0..n {
        sum += strengths.get(i).copied().unwrap_or(1.0);
    }
    (sum / n as f64).clamp(0.0, 1.5)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_completion_no_boost_hits_one() {
        let c = causal_completeness(4, 4, &[1.0, 1.0, 1.0, 1.0]);
        assert!((c - 1.0).abs() < 1e-9);
    }

    #[test]
    fn full_completion_with_boost_caps_at_one() {
        let c = causal_completeness(4, 4, &[1.5, 1.5, 1.5, 1.5]);
        assert!((c - 1.0).abs() < 1e-9);
    }

    #[test]
    fn three_quarter_completion_with_neutral_strength() {
        // 3/4 at strength 1.0 → 0.75 + 0.00 + 0.025 = 0.775
        let c = causal_completeness(3, 4, &[1.0, 1.0, 1.0, 0.0]);
        assert!((c - 0.775).abs() < 1e-9);
    }

    #[test]
    fn half_completion_with_strength_boost() {
        // 2/4 at strength 1.2 → ratio=0.5, boost=(1.2-1.0)=0.20, floor=0.10*0.5=0.05
        // → 0.5 + 0.20 + 0.05 = 0.75
        let c = causal_completeness(2, 4, &[1.2, 1.2, 0.0, 0.0]);
        assert!((c - 0.75).abs() < 1e-9);
    }

    #[test]
    fn one_quarter_with_strong_strength() {
        // 1/4 at strength 1.5 (capped at 1.5) → ratio=0.25, boost=(1.5-1.0)=0.20, floor=0.10*0.75=0.075
        // → 0.25 + 0.20 + 0.075 = 0.525
        let c = causal_completeness(1, 4, &[1.5, 0.0, 0.0, 0.0]);
        assert!((c - 0.525).abs() < 1e-9);
    }

    #[test]
    fn no_completion_returns_floor_only() {
        // 0/4 → 0.0 + 0.0 + 0.10 = 0.10
        let c = causal_completeness(0, 4, &[0.0; 4]);
        assert!((c - 0.10).abs() < 1e-9);
    }

    #[test]
    fn zero_total_returns_zero() {
        let c = causal_completeness(0, 0, &[]);
        assert_eq!(c, 0.0);
    }

    #[test]
    fn weak_steps_dont_drag_confidence_below_floor() {
        // 4/4 with strength 0.5 (weak) → 1.0 + 0.0 + 0.0 = 1.0 (ratio saturates)
        let c = causal_completeness(4, 4, &[0.5, 0.5, 0.5, 0.5]);
        assert!((c - 1.0).abs() < 1e-9);
    }
}
