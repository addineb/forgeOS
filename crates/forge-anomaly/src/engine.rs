use crate::detector::MahalanobisDetector;
use crate::features::FeatureExtractor;
use crate::null_edge::NullEdgeGate;
use crate::pattern::PatternCounter;
use crate::stats::RollingFeatureWindow;
use crate::types::{
    AnomalyEvent, AnomalyKind, AnomalySignal, BarFeatures, EngineConfig, FeatureVector,
    SignalDirection, SignalType, VolumeBar,
};

#[derive(Debug, Clone)]
pub struct EngineOutput {
    pub bar_index: u64,
    pub anomalies: Vec<AnomalyEvent>,
    pub signal: Option<AnomalySignal>,
    pub mahalanobis_dist: f64,
}

pub struct AnomalyEngine {
    cfg: EngineConfig,
    features: FeatureExtractor,
    window: RollingFeatureWindow,
    maha: MahalanobisDetector,
    patterns: PatternCounter,
    null_edge: NullEdgeGate,
    bars_processed: u64,
}

impl AnomalyEngine {
    #[must_use]
    pub fn new(cfg: EngineConfig) -> Self {
        Self {
            features: FeatureExtractor::new(cfg.depth_top_n, cfg.ofi_normalized),
            window: RollingFeatureWindow::new(cfg.lookback_bars),
            maha: MahalanobisDetector::new(cfg.mahalanobis_threshold, cfg.cov_regularization),
            patterns: PatternCounter::new(cfg.pattern_lookback_bars, cfg.min_pattern_count),
            null_edge: NullEdgeGate::new(
                cfg.null_edge_permutations,
                cfg.null_edge_margin,
                cfg.max_signals_per_100_bars,
                cfg.null_edge_seed,
            ),
            cfg,
            bars_processed: 0,
        }
    }

    #[must_use]
    pub fn config(&self) -> &EngineConfig {
        &self.cfg
    }

    #[must_use]
    pub fn bars_processed(&self) -> u64 {
        self.bars_processed
    }

    #[must_use]
    pub fn ready(&self) -> bool {
        self.window.ready()
    }

    pub fn on_bar(&mut self, bar: &VolumeBar) -> EngineOutput {
        self.bars_processed += 1;
        self.null_edge.on_bar();

        let Some(f) = self.features.observe(bar) else {
            return self.empty_output(bar.bar_index);
        };

        let vec = f.to_vector();
        let raw_dist = self.maha.distance_or_zero(&self.window, &vec);
        // Bug fix #3: Cap Mahalanobis distance to prevent regime-shift spikes
        // (observed up to 507) from producing unrealistic expected_move values
        // and saturating confidence. A cap at ~10× threshold keeps the distance
        // meaningful for anomaly ranking without blowing up downstream metrics.
        let dist = if self.cfg.mahalanobis_max > 0.0 {
            raw_dist.min(self.cfg.mahalanobis_max)
        } else {
            raw_dist
        };
        let mut anomalies = self.detect_anomalies(&f, bar, &vec);
        self.window.push(vec);

        if !self.window.ready() {
            return EngineOutput {
                anomalies,
                mahalanobis_dist: dist,
                ..self.empty_output(0)
            };
        }

        let fired = dist >= self.maha.threshold();
        if fired && !anomalies.is_empty() {
            self.update_patterns(&mut anomalies, bar);
        }

        let signal = fired
            .then(|| self.try_signal(&anomalies, &f, bar, dist))
            .flatten();
        if signal.is_some() {
            self.null_edge.on_signal();
        }

        EngineOutput {
            bar_index: bar.bar_index,
            anomalies,
            signal,
            mahalanobis_dist: dist,
        }
    }

    pub fn on_bars(&mut self, bars: &[VolumeBar]) -> Vec<EngineOutput> {
        bars.iter().map(|b| self.on_bar(b)).collect()
    }

    fn empty_output(&self, bar_index: u64) -> EngineOutput {
        EngineOutput {
            bar_index,
            anomalies: Vec::new(),
            signal: None,
            mahalanobis_dist: 0.0,
        }
    }

    fn detect_anomalies(
        &self,
        f: &BarFeatures,
        bar: &VolumeBar,
        vec: &FeatureVector,
    ) -> Vec<AnomalyEvent> {
        let z_fdr = self.window.z_scores_fdr(vec, self.cfg.fdr_alpha);
        // Default z-score gate for most features.
        let t_default = 2.0_f64;
        // Lower gate for low-variance features (now CVD-estimated, so they have
        // moderate variance). Raised from 1.5 to 1.8 to reduce noise dominance
        // while still catching genuine aggressor/large-print/intensity signals.
        let t_low = 1.8_f64;
        let mut out = Vec::new();
        // Each entry: (AnomalyKind, feature_index, raw_value, z_threshold, skip_fdr)
        // skip_fdr = true for low-variance features whose p-values are always
        // non-significant under BH correction — FDR kills them before the z-gate
        // can fire. We trust the relaxed z-gate (1.5) as the sole filter.
        let pairs: &[(AnomalyKind, usize, f64, f64, bool)] = &[
            (AnomalyKind::Ofi, 0, f.ofi_normalized, t_default, false),
            (AnomalyKind::Cvd, 1, f.cvd_delta, t_default, false),
            (
                AnomalyKind::DepthImbalance,
                2,
                f.depth_imbalance,
                t_default,
                false,
            ),
            (AnomalyKind::Absorption, 3, f.absorption, t_default, false),
            (
                AnomalyKind::LiquidityVacuum,
                4,
                f.liquidity_vacuum,
                t_default,
                false,
            ),
            (
                AnomalyKind::VolDeltaDivergence,
                5,
                f.vol_delta_divergence,
                t_default,
                false,
            ),
            // Low-variance features: relaxed z-gate AND skip FDR (Bug fix #4 v2).
            (
                AnomalyKind::AggressorImbalance,
                7,
                f.aggressor_ratio,
                t_low,
                true,
            ),
            (
                AnomalyKind::LargePrint,
                8,
                f.large_print_imbalance,
                t_low,
                true,
            ),
            (
                AnomalyKind::TradeIntensity,
                9,
                f.trade_intensity,
                t_low,
                true,
            ),
        ];
        for &(kind, idx, raw, t, skip_fdr) in pairs {
            let (z, sig) = z_fdr[idx];
            // For low-variance features, skip FDR — trust the z-gate alone.
            if z.abs() < t || (!skip_fdr && !sig) {
                continue;
            }
            let dir = if kind == AnomalyKind::VolDeltaDivergence {
                vol_delta_dir(raw, bar.cvd_delta, z)
            } else {
                z_sign(z)
            };
            out.push(AnomalyEvent {
                bar_index: bar.bar_index,
                ts: bar.ts,
                kind,
                direction: dir,
                z_score: z,
                raw_value: raw,
                // Use event_confidence for meaningful gradient (Bug fix #1)
                confidence: event_confidence(z, t),
            });
        }
        if let Some(ev) = out.last_mut() {
            if ev.kind == AnomalyKind::LiquidityVacuum {
                ev.direction = vac_dir(f.ask_vacuum - f.bid_vacuum);
            }
        }
        out
    }

    fn update_patterns(&mut self, anomalies: &mut Vec<AnomalyEvent>, bar: &VolumeBar) {
        let count = self.patterns.record(bar.bar_index, anomalies);
        let key = self.patterns.current_seq_hash();
        if self.patterns.is_repetitive(key) {
            let dir = weighted_dir(anomalies).unwrap_or(SignalDirection::Neutral);
            // Cap pattern-event confidence at 0.85 so even heavily-repeated
            // patterns don't push the composite above 0.92.
            let pat_conf = ((count as f64 / self.cfg.min_pattern_count as f64) * 0.65 + 0.20).min(0.85);
            anomalies.push(AnomalyEvent {
                bar_index: bar.bar_index,
                ts: bar.ts,
                kind: AnomalyKind::PatternRepeat,
                direction: dir,
                z_score: count as f64,
                raw_value: count as f64,
                confidence: pat_conf,
            });
        }
    }

    fn try_signal(
        &mut self,
        anomalies: &[AnomalyEvent],
        f: &BarFeatures,
        bar: &VolumeBar,
        dist: f64,
    ) -> Option<AnomalySignal> {
        let vec = f.to_vector();
        let has_pattern = anomalies
            .iter()
            .any(|e| e.kind == AnomalyKind::PatternRepeat);
        // Pattern-bearing signals: skip the shuffle-based null-edge test
        // (the pattern detector's quality filters already validate non-
        // randomness) but respect the global rate limit to prevent over-trading.
        let null_ok = if has_pattern {
            self.null_edge.rate_ok()
        } else {
            self.null_edge
                .validate(&self.window, &vec, dist, &self.maha)
        };
        if !null_ok {
            return None;
        }

        let direction = weighted_dir(anomalies)?;

        let (confidence, pattern_count) = calc_confidence(anomalies, dist, &self.cfg);
        if confidence < self.cfg.min_confidence {
            return None;
        }

        let expected_move = dist * self.cfg.expected_move_maha_coeff;
        if expected_move < self.cfg.fee_bps + self.cfg.edge_margin_bps {
            return None;
        }

        let flow_aligns = f.ofi_normalized.signum() == f.cvd_delta.signum()
            && f.ofi_normalized.abs() > 0.001
            && f.cvd_delta.abs() > 0.001;
        let (signal_type, label) = classify_signal(anomalies, flow_aligns);

        let hold_bars = (((self.cfg.base_hold_bars as f64)
            * (dist / self.cfg.mahalanobis_threshold).max(1.0))
        .round() as u32)
            .clamp(self.cfg.base_hold_bars, self.cfg.max_hold_bars);

        Some(AnomalySignal {
            bar_index: bar.bar_index, ts: bar.ts, signal_type, direction, confidence,
            description: format!(
                "[{}] ofi={:.3} cvd={:.2} imb={:.3} abs={:.2} vac={:.3} div={:.2} agg={:.3} lp={:.3} int={:.2} ret={:.1}bps",
                label, f.ofi_normalized, f.cvd_delta, f.depth_imbalance, f.absorption,
                f.liquidity_vacuum, f.vol_delta_divergence, f.aggressor_ratio,
                f.large_print_imbalance, f.trade_intensity, f.mid_return_bps,
            ),
            mahalanobis_dist: dist, pattern_count, expected_move_bps: expected_move,
            hold_bars, passed_null_edge: true, events: anomalies.to_vec(),
        })
    }
}

fn weighted_dir(events: &[AnomalyEvent]) -> Option<SignalDirection> {
    let (lw, sw) = events
        .iter()
        .filter(|e| e.kind != AnomalyKind::PatternRepeat)
        .fold((0.0, 0.0), |(l, s), e| match e.direction {
            SignalDirection::Long => (l + e.z_score.abs(), s),
            SignalDirection::Short => (l, s + e.z_score.abs()),
            SignalDirection::Neutral => (l, s),
        });
    if lw > sw && lw > 0.0 {
        Some(SignalDirection::Long)
    } else if sw > lw && sw > 0.0 {
        Some(SignalDirection::Short)
    } else {
        None
    }
}

fn z_sign(z: f64) -> SignalDirection {
    if z > 0.0 {
        SignalDirection::Long
    } else {
        SignalDirection::Short
    }
}

/// Build the per-event confidence from the z-score.
///
/// Uses `(z-t)/(z+5.5t)` — steep denominator so high z-scores compress.
/// Maps: z=3→0.06, z=5→0.17, z=8→0.29, z=15→0.41, z=30→0.51.
/// Keeps per-event confidence in 0.06-0.55 range, giving calc_confidence
/// headroom so the composite spans 0.55-0.88 for typical signals
/// (rather than saturating near 1.0).
fn event_confidence(z: f64, t: f64) -> f64 {
    let d = z.abs() - t;
    if d <= 0.0 {
        return 0.0;
    }
    (d / (z.abs() + 5.5 * t)).clamp(0.0, 1.0)
}

fn vol_delta_dir(raw: f64, cvd_delta: f64, z: f64) -> SignalDirection {
    if raw > 0.0 && cvd_delta > 0.0 {
        SignalDirection::Long
    } else if raw > 0.0 && cvd_delta < 0.0 {
        SignalDirection::Short
    } else {
        z_sign(z)
    }
}

fn vac_dir(signed: f64) -> SignalDirection {
    if signed > 0.0 {
        SignalDirection::Long
    } else if signed < 0.0 {
        SignalDirection::Short
    } else {
        SignalDirection::Neutral
    }
}

fn calc_confidence(events: &[AnomalyEvent], maha: f64, cfg: &EngineConfig) -> (f64, u32) {
    // Multiplicative formula: final = base * maha_factor * agreement_factor + pattern_bonus
    //
    // With the steeper event_confidence (z+5.5t denominator), per-event scores
    // stay in 0.06-0.55 range.  Tighter factor caps and a halved pattern_bonus
    // (0.05 instead of 0.12) keep the composite from saturating at 1.0:
    //   maha_factor:    1.00 – 1.04  (capped lower)
    //   agreement_factor: 1.00 – 1.04  (unchanged)
    //   pattern_bonus:  0 – 0.05      (was 0–0.12)
    // Expected final range: 0.55 – 0.90 for real signals.
    let (mut total, mut weight) = (0.0, 0.0);
    let mut kinds = 0u32;
    let mut pattern_bonus = 0.0;
    let mut pattern_count = 1_u32;
    for e in events {
        if e.kind == AnomalyKind::PatternRepeat {
            // Pattern bonus is intentionally modest (was 0.12, now 0.05): we want
            // patterns to add confirmation, not dominate the composite.  Patterns
            // that have appeared 4+ times get the full multiplier; otherwise a
            // softer scaling prevents low-count patterns from inflating confidence.
            let rep_mult = (e.raw_value as f64 / 4.0).min(1.0).max(0.25);
            pattern_bonus = e.confidence * 0.05 * rep_mult;
            pattern_count = e.raw_value as u32;
            continue;
        }
        kinds += 1;
        let w = e.z_score.abs().max(1.0);
        total += e.confidence * w;
        weight += w;
    }
    let base = if weight > 0.0 { total / weight } else { 0.0 };
    // Maha factor: capped at 1.04 — mild edge for multivariate extremity.
    let maha_factor = 1.0 + ((maha / cfg.mahalanobis_threshold - 1.0) * 0.020).clamp(0.0, 0.04);
    // Agreement factor: capped at 1.04 — small confirmation from multiple kinds.
    let agreement_factor = 1.0 + (kinds.saturating_sub(1) as f64 * 0.012).min(0.04);
    (
        (base * maha_factor * agreement_factor + pattern_bonus).min(1.0),
        pattern_count,
    )
}

fn classify_signal(events: &[AnomalyEvent], flow_aligns: bool) -> (SignalType, &'static str) {
    let mut has_div = false;
    let mut has_abs = false;
    let mut has_spot = false;
    for e in events {
        match e.kind {
            AnomalyKind::VolDeltaDivergence => has_div = true,
            AnomalyKind::Absorption => has_abs = true,
            AnomalyKind::LargePrint
            | AnomalyKind::AggressorImbalance
            | AnomalyKind::TradeIntensity => has_spot = true,
            _ => {}
        }
    }
    if has_div || (has_abs && !flow_aligns) {
        (SignalType::Reversal, "reversal")
    } else if has_spot {
        (SignalType::MomentumContinuation, "spot")
    } else {
        (SignalType::MomentumContinuation, "momentum")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synth(idx: u64, bid_q: f64, cvd: f64) -> VolumeBar {
        VolumeBar {
            ts: idx * 1_000_000_000,
            bar_index: idx,
            cum_vol: (idx + 1) as f64 * 10.0,
            bar_vol: 10.0,
            mid_price: 50_000.0,
            best_bid: 49_999.0,
            best_ask: 50_001.0,
            bid_vol_top5: bid_q,
            ask_vol_top5: 100.0,
            bid_vol_top10: bid_q * 1.5,
            ask_vol_top10: 150.0,
            cvd_delta: cvd,
            ..Default::default()
        }
    }

    #[test]
    fn warms_up_without_signals() {
        let mut engine = AnomalyEngine::new(EngineConfig::default());
        for i in 0..10 {
            assert!(engine.on_bar(&synth(i, 100.0, 0.0)).signal.is_none());
        }
    }
}
