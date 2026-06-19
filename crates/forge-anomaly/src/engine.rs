//! Main anomaly engine: sequential volume-bar processing.

use std::collections::VecDeque;

use crate::detector::MahalanobisDetector;
use crate::features::FeatureExtractor;
use crate::null_edge::NullEdgeGate;
use crate::pattern::PatternCounter;
use crate::regime::{MarketRegime, RegimeDetector};
use crate::stats::RollingFeatureWindow;
use crate::types::{
    AnomalyEvent, AnomalyKind, AnomalySignal, BarFeatures, EngineConfig,
    FeatureVector, SignalDirection, SignalType, VolumeBar,
};

/// Output of a single engine step.
#[derive(Debug, Clone)]
pub struct EngineOutput {
    pub bar_index: u64,
    pub anomalies: Vec<AnomalyEvent>,
    pub signal: Option<AnomalySignal>,
    pub mahalanobis_dist: f64,
    pub regime: Option<MarketRegime>,
}

/// Volume-bar anomaly detection engine.
pub struct AnomalyEngine {
    cfg: EngineConfig,
    features: FeatureExtractor,
    window: RollingFeatureWindow,
    maha: MahalanobisDetector,
    patterns: PatternCounter,
    null_edge: NullEdgeGate,
    regime: RegimeDetector,
    mid_returns: VecDeque<f64>,
    bars_processed: u64,
    signals_emitted: u64,
}

impl AnomalyEngine {
    #[must_use]
    pub fn new(cfg: EngineConfig) -> Self {
        let lb = cfg.lookback_bars;
        Self {
            features: FeatureExtractor::new(cfg.depth_top_n, cfg.ofi_normalized),
            window: RollingFeatureWindow::new(lb),
            maha: MahalanobisDetector::new(cfg.mahalanobis_threshold, cfg.cov_regularization),
            patterns: PatternCounter::new(cfg.pattern_lookback_bars, cfg.min_pattern_count),
            null_edge: NullEdgeGate::new(
                cfg.null_edge_permutations,
                cfg.null_edge_margin,
                cfg.max_signals_per_100_bars,
                cfg.null_edge_seed,
            ),
            regime: RegimeDetector::new(
                cfg.regime_lookback,
                cfg.regime_vol_threshold,
                cfg.regime_autocorr_threshold,
            ),
            mid_returns: VecDeque::new(),
            cfg,
            bars_processed: 0,
            signals_emitted: 0,
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
    pub fn signals_emitted(&self) -> u64 {
        self.signals_emitted
    }

    #[must_use]
    pub fn ready(&self) -> bool {
        self.window.ready()
    }

    /// Process one volume bar.
    pub fn on_bar(&mut self, bar: &VolumeBar) -> EngineOutput {
        self.bars_processed += 1;
        self.null_edge.on_bar();

        // Phase 1: feature extraction
        let f = match self.features.observe(bar) {
            Some(f) => f,
            None => {
                return EngineOutput {
                    bar_index: bar.bar_index,
                    anomalies: Vec::new(),
                    signal: None,
                    mahalanobis_dist: 0.0,
                    regime: None,
                };
            }
        };

        // Phase 2: regime detection
        self.mid_returns.push_back(f.mid_return_bps);
        while self.mid_returns.len() > self.cfg.regime_lookback * 2 {
            self.mid_returns.pop_front();
        }
        let mid_slice: Vec<f64> = self.mid_returns.iter().copied().collect();
        let regime = self.regime.classify(&mid_slice);

        // Phase 3: multivariate anomaly detection
        let vector = f.to_vector();
        let maha_dist = self.maha.distance_or_zero(&self.window, &vector);

        // Phase 4: per-feature anomaly classification (FDR-corrected z-scores)
        let z_fdr = self.window.z_scores_fdr(&vector, self.cfg.fdr_alpha);
        let mut anomalies = classify_anomalies(&f, bar, &z_fdr);

        // Update rolling window AFTER detecting against prior distribution
        self.window.push(vector);

        // Warmup guard — don't emit signals until window is full
        if !self.window.ready() {
            return EngineOutput {
                bar_index: bar.bar_index,
                anomalies,
                signal: None,
                mahalanobis_dist: maha_dist,
                regime: Some(regime),
            };
        }

        // Phase 5: pattern tracking (reinforcement)
        let detector_fired = maha_dist >= self.maha.threshold();
        if detector_fired && !anomalies.is_empty() {
            let sig_key = PatternCounter::signature(&anomalies);
            let count = self.patterns.record(sig_key);
            if self.patterns.is_repetitive(sig_key) {
                let dir = majority_direction(&anomalies);
                anomalies.push(AnomalyEvent {
                    bar_index: bar.bar_index,
                    ts: bar.ts,
                    kind: AnomalyKind::PatternRepeat,
                    direction: dir,
                    z_score: count as f64,
                    raw_value: count as f64,
                    confidence: (count as f64 / self.cfg.min_pattern_count as f64).min(1.0),
                });
            }
        }

        // Phase 6: signal composition
        let signal = if detector_fired {
            self.compose_signal(&anomalies, &f, bar, maha_dist, &vector, regime)
        } else {
            None
        };

        if signal.is_some() {
            self.signals_emitted += 1;
            self.null_edge.on_signal();
        }

        EngineOutput {
            bar_index: bar.bar_index,
            anomalies,
            signal,
            mahalanobis_dist: maha_dist,
            regime: Some(regime),
        }
    }

    /// Process a batch of bars.
    pub fn on_bars(&mut self, bars: &[VolumeBar]) -> Vec<EngineOutput> {
        bars.iter().map(|b| self.on_bar(b)).collect()
    }

    fn compose_signal(
        &mut self,
        anomalies: &[AnomalyEvent],
        features: &BarFeatures,
        bar: &VolumeBar,
        maha_dist: f64,
        vector: &FeatureVector,
        regime: MarketRegime,
    ) -> Option<AnomalySignal> {
        // Null-edge gate
        if !self.null_edge.validate(&self.window, vector, maha_dist, &self.maha) {
            return None;
        }

        // Direction voting
        let direction = vote_direction(anomalies)?;

        // Signal classification
        let (signal_type, description) = classify_signal(features, &direction, anomalies, regime);

        // Confidence check
        let confidence = composite_confidence(anomalies, maha_dist, &self.cfg);
        if confidence < self.cfg.min_confidence {
            return None;
        }

        // Fee-aware expected move
        let expected_move = maha_dist * self.cfg.expected_move_maha_coeff;
        let min_move = self.cfg.fee_bps + self.cfg.edge_margin_bps;
        if expected_move < min_move {
            return None;
        }

        // Hold duration
        let hold = ((self.cfg.base_hold_bars as f64) * (maha_dist / self.cfg.mahalanobis_threshold).max(1.0))
            .round() as u32;
        let hold_bars = hold.clamp(self.cfg.base_hold_bars, self.cfg.max_hold_bars);

        let pattern_count = anomalies
            .iter()
            .find(|e| e.kind == AnomalyKind::PatternRepeat)
            .map(|e| e.raw_value as u32)
            .unwrap_or(1);

        Some(AnomalySignal {
            bar_index: bar.bar_index,
            ts: bar.ts,
            signal_type,
            direction,
            confidence,
            description,
            mahalanobis_dist: maha_dist,
            pattern_count,
            expected_move_bps: expected_move,
            hold_bars,
            passed_null_edge: true,
            regime: Some(regime),
            events: anomalies.to_vec(),
        })
    }
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

fn classify_anomalies(
    f: &BarFeatures,
    bar: &VolumeBar,
    z_fdr: &[(f64, bool); crate::FEATURE_DIM],
) -> Vec<AnomalyEvent> {
    const Z_THRESHOLD: f64 = 2.0;
    let mut out = Vec::new();

    macro_rules! push {
        ($kind:ident, $idx:expr, $raw:expr, $momentum:expr) => {
            push_anomaly(&mut out, bar, AnomalyKind::$kind, z_fdr[$idx], $raw, Z_THRESHOLD, $momentum);
        };
    }

    push!(Ofi, 0, f.ofi_normalized, true);
    push!(Cvd, 1, f.cvd_delta, true);
    push!(DepthImbalance, 2, f.depth_imbalance, true);
    push!(Absorption, 3, f.absorption, true);

    push!(LiquidityVacuum, 4, f.liquidity_vacuum, false);
    let signed_vac = f.ask_vacuum - f.bid_vacuum;
    if let Some(event) = out.last_mut() {
        if event.kind == AnomalyKind::LiquidityVacuum {
            event.direction = match signed_vac {
                v if v > 0.0 => SignalDirection::Long,
                v if v < 0.0 => SignalDirection::Short,
                _ => SignalDirection::Neutral,
            };
        }
    }

    push!(VolDeltaDivergence, 5, f.vol_delta_divergence, false);
    // Skip index 6 (CVD acceleration) — no corresponding AnomalyKind
    push!(AggressorImbalance, 7, f.aggressor_ratio, true);
    push!(LargePrint, 8, f.large_print_imbalance, true);
    push!(TradeIntensity, 9, f.trade_intensity, false);

    out
}

fn push_anomaly(
    out: &mut Vec<AnomalyEvent>,
    bar: &VolumeBar,
    kind: AnomalyKind,
    (z, significant): (f64, bool),
    raw: f64,
    threshold: f64,
    momentum_direction: bool,
) {
    if z.abs() < threshold || !significant {
        return;
    }
    let direction = if kind == AnomalyKind::VolDeltaDivergence {
        if raw > 0.0 && bar.cvd_delta > 0.0 {
            SignalDirection::Long
        } else if raw > 0.0 && bar.cvd_delta < 0.0 {
            SignalDirection::Short
        } else {
            z_sign(z)
        }
    } else if momentum_direction {
        z_sign(z)
    } else {
        z_sign(z)
    };

    let confidence = ((z.abs() - threshold) / threshold).clamp(0.0, 1.0);
    out.push(AnomalyEvent {
        bar_index: bar.bar_index,
        ts: bar.ts,
        kind,
        direction,
        z_score: z,
        raw_value: raw,
        confidence,
    });
}

fn z_sign(z: f64) -> SignalDirection {
    if z > 0.0 {
        SignalDirection::Long
    } else {
        SignalDirection::Short
    }
}

fn classify_signal(
    f: &BarFeatures,
    direction: &SignalDirection,
    events: &[AnomalyEvent],
    regime: MarketRegime,
) -> (SignalType, String) {
    let has_divergence = events.iter().any(|e| e.kind == AnomalyKind::VolDeltaDivergence);
    let has_absorption = events.iter().any(|e| e.kind == AnomalyKind::Absorption);
    let has_large_print = events.iter().any(|e| e.kind == AnomalyKind::LargePrint);
    let has_aggressor = events.iter().any(|e| e.kind == AnomalyKind::AggressorImbalance);
    let has_trade_intensity = events.iter().any(|e| e.kind == AnomalyKind::TradeIntensity);
    let flow_aligns = match direction {
        SignalDirection::Long => f.cvd_delta > 0.0 || f.ofi_normalized > 0.0,
        SignalDirection::Short => f.cvd_delta < 0.0 || f.ofi_normalized < 0.0,
        SignalDirection::Neutral => false,
    };

    let r = format!("{}", regime);

    if has_divergence || (has_absorption && !flow_aligns) {
        (
            SignalType::Reversal,
            format!("reversal[{}]: div={:.2} abs={:.2} ret={:.1}bps agg={:.3} lp={:.3}",
                r, f.vol_delta_divergence, f.absorption, f.mid_return_bps,
                f.aggressor_ratio, f.large_print_imbalance),
        )
    } else if has_large_print || has_aggressor || has_trade_intensity {
        (
            SignalType::MomentumContinuation,
            format!("spot[{}]: ofi={:.3} cvd={:.2} agg={:.3} lp={:.3} int={:.2}",
                r, f.ofi_normalized, f.cvd_delta,
                f.aggressor_ratio, f.large_print_imbalance, f.trade_intensity),
        )
    } else {
        (
            SignalType::MomentumContinuation,
            format!("momentum[{}]: ofi={:.3} cvd={:.2} imb={:.3} vac={:.3} agg={:.3} lp={:.3} int={:.2}",
                r, f.ofi_normalized, f.cvd_delta, f.depth_imbalance, f.liquidity_vacuum,
                f.aggressor_ratio, f.large_print_imbalance, f.trade_intensity),
        )
    }
}

fn vote_direction(events: &[AnomalyEvent]) -> Option<SignalDirection> {
    let (mut long_w, mut short_w) = (0.0, 0.0);
    for e in events {
        if e.kind == AnomalyKind::PatternRepeat {
            continue;
        }
        let w = e.z_score.abs();
        match e.direction {
            SignalDirection::Long => long_w += w,
            SignalDirection::Short => short_w += w,
            SignalDirection::Neutral => {}
        }
    }
    if long_w > short_w && long_w > 0.0 {
        Some(SignalDirection::Long)
    } else if short_w > long_w && short_w > 0.0 {
        Some(SignalDirection::Short)
    } else {
        None
    }
}

fn majority_direction(events: &[AnomalyEvent]) -> SignalDirection {
    vote_direction(events).unwrap_or(SignalDirection::Neutral)
}

fn composite_confidence(events: &[AnomalyEvent], maha: f64, cfg: &EngineConfig) -> f64 {
    let (mut total, mut weight, mut kinds) = (0.0, 0.0, 0u32);
    for e in events {
        if e.kind == AnomalyKind::PatternRepeat {
            continue;
        }
        kinds += 1;
        let w = e.z_score.abs().max(1.0);
        total += e.confidence * w;
        weight += w;
    }
    let base = if weight > 0.0 { total / weight } else { 0.0 };
    let maha_boost = (maha / cfg.mahalanobis_threshold - 1.0).clamp(0.0, 0.2);
    let pattern_boost = events
        .iter()
        .find(|e| e.kind == AnomalyKind::PatternRepeat)
        .map(|e| e.confidence * 0.15)
        .unwrap_or(0.0);
    let agreement = match kinds {
        0 | 1 => 0.0,
        2 => 0.08,
        _ => 0.15,
    };
    (base + maha_boost + pattern_boost + agreement).min(1.0)
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
            let out = engine.on_bar(&synth(i, 100.0, 0.0));
            assert!(out.signal.is_none());
        }
    }
}
