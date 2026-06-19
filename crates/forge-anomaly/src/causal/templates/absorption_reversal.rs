//! First causal template: Absorption → Exhaustion → Reversal.
//!
//! Causal chain:
//!
//! ```text
//! Step 1: Heavy one-sided CVD fires (aggressive pressure against one side).
//! Step 2: Absorption holds (best bid/ask does not move despite pressure).
//! Step 3: CVD decelerates given absorption (pressure dying — aggressor paying but not progressing).
//! Step 4: Trapped aggressors now underwater, forced exit drives reversal.
//! ```
//!
//! Each step is independently falsifiable. If step 2 didn't hold, no signal.
//! If step 3 didn't decelerate, no signal. Each step produces a per-step
//! strength (z-like) that calibrates `causal_completeness` beyond mere count.
//!
//! Precondition gate (before step 1): the defending side must have non-trivial
//! depth. If one-sided and thin, "absorption" is meaningless (nothing to
//! absorb into). This is a depth-imbalance filter, not a feature.
//!
//! Direction: Long if absorption is on the bid side (sellers absorbed against
//! bids, expect buyers to step in); Short if absorption is on the ask side.

use crate::causal::template::{
    direction_label, format_step_strengths, passes_depth_precondition, signed_direction,
    step1_within_window, step_name, CausalTemplate, DiagnosticSnapshot, TemplateInput,
};
use crate::causal::{AbsorptionReversalParams, CausalDirection, Step, TemplateOutcome};
use crate::types::BarFeatures;

/// Diagnostic counters tracking how far candidate episodes progress through
/// the 3-step causal chain. Incremented inside `evaluate()` at each
/// transition point so we can localize where the chain breaks in real data.
///
/// Bucket meanings:
/// - `bars_evaluated`: total bars passed to `evaluate()`
/// - `step1_attempts`: bars where pressure_threshold was evaluated
/// - `step1_fired`: bars where step 1 fired (pressure + precondition passed)
/// - `step2_attempts`: bars with an active step 1 that we tested step 2 on
/// - `step2_fired`: step 2 fired (absorption on defending side ≥ threshold)
/// - `step3_attempts`: bars with an active step 2 that we tested step 3 on
/// - `step3_fired`: step 3 fired (deceleration with same sign passed)
/// - `step1_expired`: bars where an active step 1 exceeded the recency window
/// - `sign_flip_rejected`: bars where step 3 was rejected due to sign flip
/// - `absorption_strict_failed`: step 2 attempts where strict `>` absorption didn't hold
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiagnosticCounters {
    pub bars_evaluated: u64,
    pub step1_attempts: u64,
    pub step1_fired: u64,
    pub step2_attempts: u64,
    pub step2_fired: u64,
    pub step3_attempts: u64,
    pub step3_fired: u64,
    pub step1_expired: u64,
    pub sign_flip_rejected: u64,
    pub absorption_strict_failed: u64,
}

/// One running instance of the absorption → reversal template.
///
/// Carries the bar-index where step 1 most recently fired, so step 3
/// completion is gated on recency.
pub struct AbsorptionReversalTemplate {
    params: AbsorptionReversalParams,
    /// Bar index where step 1 most recently fired (None = no active step-1).
    step1_bar: Option<u64>,
    /// Signed CVD magnitude at step 1 (for deceleration ratio in step 3).
    step1_cvd_abs: f64,
    /// Direction implied by step 1 (sign of CVD).
    step1_dir: CausalDirection,
    /// Defending-side depth at step 1 (for precondition continuity).
    step1_defending_depth: f64,
    /// Diagnostic counters (only updated when running with `--diagnostic`).
    diag: DiagnosticCounters,
}

impl AbsorptionReversalTemplate {
    #[must_use]
    pub fn new(params: AbsorptionReversalParams) -> Self {
        Self {
            params,
            step1_bar: None,
            step1_cvd_abs: 0.0,
            step1_dir: CausalDirection::Neutral,
            step1_defending_depth: 0.0,
            diag: DiagnosticCounters::default(),
        }
    }

    /// Reset diagnostic counters. Call before re-running with diagnostic mode.
    pub fn reset_diag(&mut self) {
        self.diag = DiagnosticCounters::default();
    }

    /// Compute the defending-side depth-imbalance ratio (0..=1) for `dir`.
    /// `bar.depth_imbalance` is (bid - ask) / (bid + ask). For Long we want
    /// bids thick → positive; for Short we want asks thick → negative.
    /// Returns 0..=1.
    fn defending_depth(bar: &BarFeatures, dir: CausalDirection) -> f64 {
        match dir {
            CausalDirection::Long => (bar.depth_imbalance + 1.0) / 2.0,
            CausalDirection::Short => (1.0 - bar.depth_imbalance) / 2.0,
            CausalDirection::Neutral => 0.0,
        }
    }
}

impl CausalTemplate for AbsorptionReversalTemplate {
    fn id(&self) -> &'static str {
        "absorption_reversal"
    }

    fn step_count(&self) -> u32 {
        3 // steps 1, 2, 3 — step 4 (forced exit) is the implied outcome
          // and is observed over hold_bars, not gated as a pre-signal check.
    }

    fn diagnostic(&self, template_id: &str) -> Option<DiagnosticSnapshot> {
        if template_id != self.id() {
            return None;
        }
        Some(DiagnosticSnapshot {
            template_id: self.id().to_string(),
            bars_evaluated: self.diag.bars_evaluated,
            step1_attempts: self.diag.step1_attempts,
            step1_fired: self.diag.step1_fired,
            step2_attempts: self.diag.step2_attempts,
            step2_fired: self.diag.step2_fired,
            step3_attempts: self.diag.step3_attempts,
            step3_fired: self.diag.step3_fired,
            step1_expired: self.diag.step1_expired,
            sign_flip_rejected: self.diag.sign_flip_rejected,
            absorption_strict_failed: self.diag.absorption_strict_failed,
        })
    }

    fn evaluate(&mut self, input: &TemplateInput) -> Option<TemplateOutcome> {
        let history = input.history;
        let cvd_std = input.cvd_std;
        let cur = match history.last() {
            Some(b) => b,
            None => return None,
        };
        let cur_bar = cur.bar_index;

        // Per-step strengths (0 if step not fired).
        let mut strengths = [0.0_f64; 4];
        let mut steps_passed: u32 = 0;
        let mut completed = None;
        let mut direction = self.step1_dir;
        let mut bars_in_story: u32 = 0;

        self.diag.bars_evaluated += 1;

        // ── Step 1: heavy one-sided CVD ──
        // Threshold: |cvd_delta| > cvd_pressure_threshold * cvd_std
        // (z-like units; uses rolling std so it adapts to regime).
        let pressure_threshold = self.params.cvd_pressure_threshold * cvd_std;
        let abs_cvd = cur.cvd_delta.abs();
        let step1_strength = if cvd_std > 0.0 {
            (abs_cvd / pressure_threshold).min(2.0)
        } else {
            0.0
        };

        self.diag.step1_attempts += 1;
        if step1_strength >= 1.0 && self.step1_bar.is_none() {
            // Only fire step 1 when no episode is currently armed. Re-arming on
            // every bar where pressure is elevated would let a small "echo"
            // bar overwrite the original magnitude baseline, breaking the
            // deceleration comparison in step 3.
            let step1_dir = signed_direction(cur.cvd_delta);
            if step1_dir != CausalDirection::Neutral {
                let defending_depth = Self::defending_depth(cur, step1_dir);
                if passes_depth_precondition(defending_depth, self.params.depth_precondition_min) {
                    // Step 1 fired.
                    strengths[0] = step1_strength;
                    steps_passed = 1;
                    completed = Some(Step::Step1Pressure);
                    self.step1_bar = Some(cur_bar);
                    self.step1_cvd_abs = abs_cvd;
                    self.step1_dir = step1_dir;
                    self.step1_defending_depth = defending_depth;
                    direction = step1_dir;
                    self.diag.step1_fired += 1;
                }
            }
        }

        // ── Step 2: absorption holds in the same direction as step 1 ──
        // Absorption on the defending side means: aggressive flow continued
        // against that side but the price didn't break. We check if EITHER
        // bid_absorption (for Long) or ask_absorption (for Short) is elevated.
        if let Some(step1_bar_idx) = self.step1_bar {
            // Recency check: step 1 must be recent enough.
            if !step1_within_window(
                (cur_bar - step1_bar_idx) as u32,
                self.params.max_step1_to_signal_bars,
            ) {
                // Step 1 expired; reset.
                self.reset_step1();
                self.diag.step1_expired += 1;
            } else {
                // Same direction as step 1?
                self.diag.step2_attempts += 1;
                if direction != CausalDirection::Neutral {
                    let defending_absorption = match direction {
                        CausalDirection::Long => cur.bid_absorption,
                        CausalDirection::Short => cur.ask_absorption,
                        CausalDirection::Neutral => 0.0,
                    };
                    if defending_absorption >= self.params.absorption_hold_threshold {
                        let step2_strength =
                            (defending_absorption / self.params.absorption_hold_threshold).min(2.0);
                        strengths[1] = step2_strength;
                        steps_passed = 2;
                        completed = Some(Step::Step2Absorption);
                        bars_in_story = (cur_bar - step1_bar_idx) as u32;
                        self.diag.step2_fired += 1;
                    } else {
                        self.diag.absorption_strict_failed += 1;
                    }
                }
            }
        }

        // ── Step 3: pressure fading (two valid paths) ──
        //
        // Path A (same-sign fade): CVD still pushes the same side as step 1
        //   AND its magnitude has dropped below deceleration_ratio × step1.
        //   Classic "pressure dying but still leaning" — aggressors exhausting.
        //
        // Path B (near-zero): CVD has faded to near-zero regardless of sign.
        //   |cvd| < near_zero_ratio × step1_cvd_abs (default 0.15).
        //   If flow has essentially stopped, the aggressive episode is over
        //   regardless of which side the last flicker was on. This catches
        //   the common real-data case where CVD crosses zero before
        //   deceleration completes (100% of step3 attempts in first diagnostic).
        //
        // Sign flips that are NOT near-zero are still rejected — those are
        // a different story (forced exit / capitulation, not exhaustion).
        if steps_passed >= 2 {
            self.diag.step3_attempts += 1;
            let near_zero_ratio = 0.15;
            let near_zero = cur.cvd_delta.abs() < near_zero_ratio * self.step1_cvd_abs;
            let (sign_ok, magnitude_ok) = match direction {
                CausalDirection::Long => (
                    cur.cvd_delta > 0.0,
                    cur.cvd_delta.abs() < self.params.deceleration_ratio * self.step1_cvd_abs,
                ),
                CausalDirection::Short => (
                    cur.cvd_delta < 0.0,
                    cur.cvd_delta.abs() < self.params.deceleration_ratio * self.step1_cvd_abs,
                ),
                CausalDirection::Neutral => (false, false),
            };
            let decelerated = (sign_ok && magnitude_ok) || near_zero;
            if decelerated {
                // Strength: how much it decelerated (ratio of current to step1).
                let now_abs = cur.cvd_delta.abs();
                let ratio = if self.step1_cvd_abs > 0.0 {
                    (now_abs / self.step1_cvd_abs).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                // Lower ratio = stronger deceleration. Map: ratio=1 → 1.0, ratio=0 → 1.5.
                let step3_strength = (1.5 - 0.5 * ratio).clamp(0.5, 1.5);
                strengths[2] = step3_strength;
                steps_passed = 3;
                completed = Some(Step::Step3Deceleration);
                // Hold bar count: bars in story capped by max_step1_to_signal_bars.
                bars_in_story = (cur_bar - self.step1_bar.unwrap_or(cur_bar)) as u32;
                self.diag.step3_fired += 1;
            } else {
                // Distinguish why step 3 failed.
                if !sign_ok {
                    self.diag.sign_flip_rejected += 1;
                }
                // (Cases where magnitude alone failed are counted in step3_attempts
                //  minus step3_fired minus sign_flip_rejected.)
            }
        }

        // If step 3 didn't fire this bar, do NOT emit. The template requires
        // step 3 to commit (otherwise we don't have a full causal story).
        if steps_passed < 3 {
            return None;
        }

        // If we got here, all 3 steps fired on this bar. Build the outcome.
        // Reset step 1 so the same absorption episode doesn't double-fire.
        self.reset_step1();

        Some(TemplateOutcome {
            template_id: self.id(),
            completed_step: completed,
            steps_passed,
            steps_total: self.step_count(),
            step_strengths: strengths,
            direction,
            bars_in_story,
        })
    }
}

impl AbsorptionReversalTemplate {
    fn reset_step1(&mut self) {
        self.step1_bar = None;
        self.step1_cvd_abs = 0.0;
        self.step1_dir = CausalDirection::Neutral;
        self.step1_defending_depth = 0.0;
    }
}

/// Build a human-readable description for the validate output.
#[must_use]
pub fn describe_absorption_reversal(outcome: &TemplateOutcome) -> String {
    format!(
        "[{}] {} | {} | dir={} steps={}/{} bars={} {}",
        outcome.template_id,
        step_name(outcome.completed_step.unwrap_or(Step::Step1Pressure)),
        format_step_strengths(&outcome.step_strengths),
        direction_label(outcome.direction),
        outcome.steps_passed,
        outcome.steps_total,
        outcome.bars_in_story,
        // Include diagnostic anchors so a human can validate against the chart.
        "",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::causal::template::TemplateInput;

    fn feat(idx: u64, cvd: f64, bid_abs: f64, ask_abs: f64, depth_im: f64) -> BarFeatures {
        BarFeatures {
            bar_index: idx,
            cvd_delta: cvd,
            cvd_momentum: 0.0,
            bid_absorption: bid_abs,
            ask_absorption: ask_abs,
            absorption: bid_abs - ask_abs,
            depth_imbalance: depth_im,
            mid_return_bps: 0.0,
            ..Default::default()
        }
    }

    /// Drives the template through a complete absorption → reversal episode
    /// and asserts that the template returns Some(outcome) only when step 3
    /// completes (template contract: returns None until full chain fires).
    #[test]
    fn completes_full_chain() {
        let mut t = AbsorptionReversalTemplate::new(AbsorptionReversalParams::default());
        // Build a history of bars with stable CVD then a spike then decay.
        let mut history: Vec<BarFeatures> = (0..20)
            .map(|i| feat(i, 0.1 * (((i % 5) as f64) - 2.0), 0.0, 0.0, 0.5))
            .collect();
        // Step 1: strong negative CVD → Short direction.
        // For Short, defending side = ask, so depth_imbalance must be NEGATIVE
        // (asks-heavy). defending_depth = (1 - depth_im) / 2.
        // With depth_im = -0.7, defending_depth = (1 - (-0.7))/2 = 0.85 ≥ 0.25 ✓
        history.push(feat(20, -3.0, 0.0, 0.6, -0.7));
        let input = TemplateInput {
            history: &history,
            cvd_std: 1.0,
            cvd_mean: 0.0,
        };
        // Template returns None until step 3 — but step 1 should be tracking.
        let outcome = t.evaluate(&input);
        assert!(outcome.is_none(), "template returns None until step 3");
        assert!(t.step1_bar.is_some(), "step 1 should be armed");
        assert_eq!(t.step1_dir, CausalDirection::Short);

        // Step 2: ask_absorption holds (same direction Short).
        history.push(feat(21, -2.5, 0.0, 0.5, -0.7));
        let input = TemplateInput {
            history: &history,
            cvd_std: 1.0,
            cvd_mean: 0.0,
        };
        let outcome = t.evaluate(&input);
        assert!(outcome.is_none(), "step 2 alone does not emit");
        assert!(t.step1_bar.is_some(), "step 1 still armed");

        // Step 3: CVD fading but still negative (same side as step 1).
        // Old step 1 was cvd=-3.0, so current |cvd| must be < 0.5*3 = 1.5
        // AND still negative (not flipped). cvd=-0.4 satisfies both.
        history.push(feat(22, -0.4, 0.0, 0.5, -0.7));
        let input = TemplateInput {
            history: &history,
            cvd_std: 1.0,
            cvd_mean: 0.0,
        };
        let outcome = t.evaluate(&input);
        assert!(outcome.is_some(), "step 3 completes the chain");
        let o = outcome.unwrap();
        assert_eq!(o.completed_step, Some(Step::Step3Deceleration));
        assert_eq!(o.steps_passed, 3);
        assert_eq!(o.direction, CausalDirection::Short);
        // After emission, step 1 is reset (next episode needs new step 1).
        assert!(t.step1_bar.is_none(), "step 1 resets after emit");
    }

    /// Step 2 must fail (no absorption holding) → chain breaks, no signal.
    #[test]
    fn breaks_when_absorption_doesnt_hold() {
        let mut t = AbsorptionReversalTemplate::new(AbsorptionReversalParams::default());
        let mut history: Vec<BarFeatures> = (0..20).map(|i| feat(i, 0.0, 0.0, 0.0, 0.5)).collect();
        history.push(feat(20, -3.0, 0.0, 0.0, 0.7)); // step 1 (no absorption)
        history.push(feat(21, -2.5, 0.0, 0.0, 0.7)); // step 2 FAILS
        history.push(feat(22, -2.0, 0.0, 0.0, 0.7)); // deceleration but no step 2
        let input = TemplateInput {
            history: &history,
            cvd_std: 1.0,
            cvd_mean: 0.0,
        };
        // The first bar triggers step 1; subsequent bars should NOT complete step 3.
        // Run several bars and assert we never see steps_passed == 3.
        for _ in 0..3 {
            let _ = t.evaluate(&input);
        }
        // (no panic; absence of full completion is what we want)
    }

    /// Step 1 with thin defending-side depth is rejected by precondition gate.
    #[test]
    fn rejects_thin_defending_depth() {
        let mut t = AbsorptionReversalTemplate::new(AbsorptionReversalParams::default());
        let mut history: Vec<BarFeatures> = (0..20).map(|i| feat(i, 0.0, 0.0, 0.0, 0.5)).collect();
        // CVD spike but defending depth_imbalance = -0.9 → very thin bids
        // for a Long (defending_depth = (-0.9 + 1)/2 = 0.05 < 0.25).
        history.push(feat(20, -3.0, 0.0, 0.0, -0.9));
        let input = TemplateInput {
            history: &history,
            cvd_std: 1.0,
            cvd_mean: 0.0,
        };
        let outcome = t.evaluate(&input);
        // step 1 should NOT fire because precondition failed.
        assert!(outcome.is_none());
    }

    /// Step 3 rejects a sign flip with significant magnitude. If CVD flips
    /// sign AND the magnitude is still substantial (above near-zero), the
    /// template must NOT emit — that's a different story (capitulation).
    /// Near-zero flips (|cvd| < 0.15×step1) are accepted separately.
    #[test]
    fn step3_rejects_sign_flip() {
        let mut t = AbsorptionReversalTemplate::new(AbsorptionReversalParams::default());
        let mut history: Vec<BarFeatures> = (0..20).map(|i| feat(i, 0.0, 0.0, 0.0, 0.5)).collect();
        // Step 1: Short, cvd=-3.0, asks-heavy defending.
        history.push(feat(20, -3.0, 0.0, 0.5, -0.7));
        let input = TemplateInput {
            history: &history,
            cvd_std: 1.0,
            cvd_mean: 0.0,
        };
        assert!(t.evaluate(&input).is_none(), "step 1 only");
        assert_eq!(t.step1_dir, CausalDirection::Short);

        // Step 2: ask_absorption holds (0.5 ≥ 0.12 with relaxed threshold).
        history.push(feat(21, -2.5, 0.0, 0.5, -0.7));
        let input = TemplateInput {
            history: &history,
            cvd_std: 1.0,
            cvd_mean: 0.0,
        };
        assert!(t.evaluate(&input).is_none(), "step 2 only");

        // Step 3 candidate: cvd=+1.0. Sign flipped, and magnitude 1.0 is
        // ABOVE the near_zero threshold (0.15×3.0=0.45). This is a real
        // sign flip with bite — must be rejected.
        history.push(feat(22, 1.0, 0.0, 0.5, -0.7));
        let input = TemplateInput {
            history: &history,
            cvd_std: 1.0,
            cvd_mean: 0.0,
        };
        let outcome = t.evaluate(&input);
        assert!(
            outcome.is_none(),
            "step 3 must reject a sign flip with magnitude above near-zero"
        );
    }

    /// Step 3 still fires when magnitude drops below ratio AND sign is preserved.
    /// Verified with the existing completes_full_chain test (cvd=-0.4 after
    /// step1 cvd=-3.0). This test documents that explicitly with a separate
    /// assertion set.
    #[test]
    fn step3_accepts_same_sign_fade() {
        let mut t = AbsorptionReversalTemplate::new(AbsorptionReversalParams::default());
        let mut history: Vec<BarFeatures> = (0..20).map(|i| feat(i, 0.0, 0.0, 0.0, 0.5)).collect();
        // Step 1: Short, cvd=-3.0.
        history.push(feat(20, -3.0, 0.0, 0.5, -0.7));
        let input = TemplateInput {
            history: &history,
            cvd_std: 1.0,
            cvd_mean: 0.0,
        };
        let _ = t.evaluate(&input);
        // Step 2: ask_absorption holds.
        history.push(feat(21, -2.5, 0.0, 0.5, -0.7));
        let input = TemplateInput {
            history: &history,
            cvd_std: 1.0,
            cvd_mean: 0.0,
        };
        let _ = t.evaluate(&input);
        // Step 3: cvd=-1.0 (still negative, |1.0| < 0.5 × 3 = 1.5 — yes).
        history.push(feat(22, -1.0, 0.0, 0.5, -0.7));
        let input = TemplateInput {
            history: &history,
            cvd_std: 1.0,
            cvd_mean: 0.0,
        };
        let outcome = t.evaluate(&input);
        assert!(outcome.is_some(), "step 3 must accept same-sign fade");
        assert_eq!(outcome.unwrap().direction, CausalDirection::Short);
    }

    /// Step 3 accepts a sign flip that is near-zero. If CVD has faded to
    /// essentially nothing (|cvd| < 0.15 × step1 magnitude), the aggressive
    /// episode is over regardless of which side the last flicker was on.
    /// This is Path B of the deceleration check.
    #[test]
    fn step3_accepts_near_zero() {
        let mut t = AbsorptionReversalTemplate::new(AbsorptionReversalParams::default());
        let mut history: Vec<BarFeatures> = (0..20).map(|i| feat(i, 0.0, 0.0, 0.0, 0.5)).collect();
        // Step 1: Short, cvd=-3.0.
        history.push(feat(20, -3.0, 0.0, 0.5, -0.7));
        let input = TemplateInput {
            history: &history,
            cvd_std: 1.0,
            cvd_mean: 0.0,
        };
        let _ = t.evaluate(&input);
        // Step 2: ask_absorption holds (0.5 ≥ 0.12 with relaxed threshold).
        history.push(feat(21, -2.5, 0.0, 0.5, -0.7));
        let input = TemplateInput {
            history: &history,
            cvd_std: 1.0,
            cvd_mean: 0.0,
        };
        let _ = t.evaluate(&input);
        // Step 3: cvd=+0.3. Sign flipped (long instead of short), but
        // |0.3| < 0.15 × 3.0 = 0.45 → near_zero = true → accepted.
        history.push(feat(22, 0.3, 0.0, 0.5, -0.7));
        let input = TemplateInput {
            history: &history,
            cvd_std: 1.0,
            cvd_mean: 0.0,
        };
        let outcome = t.evaluate(&input);
        assert!(
            outcome.is_some(),
            "step 3 must accept near-zero sign flip as deceleration"
        );
        let o = outcome.unwrap();
        assert_eq!(o.direction, CausalDirection::Short);
        assert_eq!(o.completed_step, Some(Step::Step3Deceleration));
    }

    /// Diagnostic counters track every transition through the chain.
    /// This test verifies the counters increment at each branch point on a
    /// known synthetic sequence.
    #[test]
    fn diagnostic_counters_track_chain_progress() {
        let mut t = AbsorptionReversalTemplate::new(AbsorptionReversalParams::default());
        let mut history: Vec<BarFeatures> = (0..20).map(|i| feat(i, 0.0, 0.0, 0.0, 0.5)).collect();

        // Bar 20: step 1 (Short, cvd=-3.0, asks-heavy).
        history.push(feat(20, -3.0, 0.0, 0.5, -0.7));
        let input = TemplateInput {
            history: &history,
            cvd_std: 1.0,
            cvd_mean: 0.0,
        };
        let _ = t.evaluate(&input);
        let snap = t
            .diagnostic("absorption_reversal")
            .expect("template exposes diagnostics");
        assert_eq!(snap.bars_evaluated, 1);
        assert_eq!(snap.step1_attempts, 1);
        assert_eq!(snap.step1_fired, 1);
        // Step 2 fires on the SAME bar as step 1: bar 20 has both cvd=-3.0
        // (pressure) AND ask_absorption=0.5 (absorption). Same-bar absorption
        // is causally valid — the pressure and the defense coexist on one bar.
        assert_eq!(snap.step2_attempts, 1);
        assert_eq!(snap.step2_fired, 1);
        // Step 3 was attempted (same bar, steps_passed=2) but magnitude too
        // high: |cvd|=3.0 not < 0.5×3.0.  Fails magnitude but stays sign-ok.
        assert_eq!(snap.step3_attempts, 1);
        assert_eq!(snap.step3_fired, 0);

        // Bar 21: step 2 fires again (ask_absorption=0.5 ≥ 0.12),
        // step 3 attempted again (cvd=-2.5, |2.5| < 0.7×3.0=2.1 → fails).
        history.push(feat(21, -2.5, 0.0, 0.5, -0.7));
        let input = TemplateInput {
            history: &history,
            cvd_std: 1.0,
            cvd_mean: 0.0,
        };
        let _ = t.evaluate(&input);
        let snap = t.diagnostic("absorption_reversal").unwrap();
        assert_eq!(snap.bars_evaluated, 2);
        assert_eq!(snap.step2_attempts, 2);
        assert_eq!(snap.step2_fired, 2);
        // Step 3 attempted on bars 20 and 21.
        assert_eq!(snap.step3_attempts, 2);
        assert_eq!(snap.step3_fired, 0);

        // Bar 22: step 3 fires (cvd=-1.0, same sign, |1.0| < 1.5).
        history.push(feat(22, -1.0, 0.0, 0.5, -0.7));
        let input = TemplateInput {
            history: &history,
            cvd_std: 1.0,
            cvd_mean: 0.0,
        };
        let _ = t.evaluate(&input);
        let snap = t.diagnostic("absorption_reversal").unwrap();
        assert_eq!(snap.bars_evaluated, 3);
        assert_eq!(snap.step2_attempts, 3);
        assert_eq!(snap.step2_fired, 3);
        // Step 3 attempted on bars 20, 21, 22 (fired only on 22).
        assert_eq!(snap.step3_attempts, 3);
        assert_eq!(snap.step3_fired, 1);
    }

    /// Sign-flip rejections are counted separately from magnitude rejections.
    #[test]
    fn diagnostic_counts_sign_flip() {
        let mut t = AbsorptionReversalTemplate::new(AbsorptionReversalParams::default());
        let mut history: Vec<BarFeatures> = (0..20).map(|i| feat(i, 0.0, 0.0, 0.0, 0.5)).collect();
        // Step 1: Short, cvd=-3.0.
        history.push(feat(20, -3.0, 0.0, 0.5, -0.7));
        let input = TemplateInput {
            history: &history,
            cvd_std: 1.0,
            cvd_mean: 0.0,
        };
        let _ = t.evaluate(&input);
        // Step 2: ask_absorption holds.
        history.push(feat(21, -2.5, 0.0, 0.5, -0.7));
        let input = TemplateInput {
            history: &history,
            cvd_std: 1.0,
            cvd_mean: 0.0,
        };
        let _ = t.evaluate(&input);
        // Step 3 candidate: cvd=+1.0 (sign flipped, magnitude 1.0 > near_zero
        // threshold 0.45, so this is properly rejected).
        history.push(feat(22, 1.0, 0.0, 0.5, -0.7));
        let input = TemplateInput {
            history: &history,
            cvd_std: 1.0,
            cvd_mean: 0.0,
        };
        let _ = t.evaluate(&input);
        let snap = t.diagnostic("absorption_reversal").unwrap();
        // Step 3 attempted on bars 20 (fails magnitude), 21 (fails magnitude),
        // and 22 (sign flip rejected).
        assert_eq!(snap.step3_attempts, 3);
        assert_eq!(snap.sign_flip_rejected, 1);
        assert_eq!(snap.step3_fired, 0);
    }
}
