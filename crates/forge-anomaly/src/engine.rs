//! Main anomaly engine: sequential volume-bar processing with multivariate
//! detection, pattern counting, and null-edge validation.

use std::collections::VecDeque;

use crate::detector::{IsolationForestDetector, MahalanobisDetector};
use crate::features::FeatureExtractor;
use crate::null_edge::NullEdgeGate;
use crate::pattern::PatternCounter;
use crate::regime::{MarketRegime, RegimeDetector};
use crate::stats::RollingFeatureWindow;
use crate::types::{
    AnomalyEvent, AnomalyKind, AnomalySignal, BarFeatures, DetectionMethod, EngineConfig,
    FeatureVector, SignalDirection, SignalType, VolumeBar,
};

/// Output of a single engine step.
#[derive(Debug, Clone)]
pub struct EngineOutput {
    pub bar_index: u64,
    pub anomalies: Vec<AnomalyEvent>,
    pub signal: Option<AnomalySignal>,
    pub mahalanobis_dist: f64,
    pub isolation_score: f64,
    pub regime: Option<MarketRegime>,
}

/// Volume-bar anomaly detection engine.
pub struct AnomalyEngine {
    cfg: EngineConfig,
    features: FeatureExtractor,
    window: RollingFeatureWindow,
    maha: MahalanobisDetector,
    iso: IsolationForestDetector,
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
            iso: IsolationForestDetector::new(cfg.isolation_trees, cfg.isolation_threshold),
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

    /// Process one volume bar sequentially.
    pub fn on_bar(&mut self, bar: &VolumeBar) -> EngineOutput {
        self.bars_processed += 1;
        self.null_edge.on_bar();

        let bar_features = match self.features.observe(bar) {
            Some(f) => f,
            None => {
                return EngineOutput {
                    bar_index: bar.bar_index,
                    anomalies: Vec::new(),
                    signal: None,
                    mahalanobis_dist: 0.0,
                    isolation_score: 0.0,
                    regime: None,
                };
            }
        };

        self.mid_returns.push_back(bar_features.mid_return_bps);
        while self.mid_returns.len() > self.cfg.regime_lookback * 2 {
            self.mid_returns.pop_front();
        }

        let vector = bar_features.to_vector();

        let maha_dist = self.maha.distance_or_zero(&self.window, &vector);
        let iso_score = self.iso.score_cached(&self.window, &vector);

        let z_fdr = self.window.z_scores_fdr(&vector, self.cfg.fdr_alpha);
        let mut anomalies = classify_feature_anomalies(&bar_features, bar, &z_fdr);

        self.window.push(vector);
        self.iso.bump_epoch();

        let mid_returns_slice: Vec<f64> = self.mid_returns.iter().copied().collect();
        let current_regime = self.regime.classify(&mid_returns_slice);

        if !self.window.ready() {
            return EngineOutput {
                bar_index: bar.bar_index,
                anomalies,
                signal: None,
                mahalanobis_dist: maha_dist,
                isolation_score: iso_score,
                regime: Some(current_regime),
            };
        }

        let detector_fired = detector_triggered(self.cfg.method, maha_dist, iso_score, &self.maha, &self.iso);

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

        let signal = if detector_fired {
            self.try_compose_signal(
                &anomalies, &bar_features, bar, maha_dist, iso_score, &vector, current_regime,
            )
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
            isolation_score: iso_score,
            regime: Some(current_regime),
        }
    }

    /// Process a batch of bars (e.g. CSV replay).
    pub fn on_bars(&mut self, bars: &[VolumeBar]) -> Vec<EngineOutput> {
        bars.iter().map(|b| self.on_bar(b)).collect()
    }

    fn try_compose_signal(
        &mut self,
        anomalies: &[AnomalyEvent],
        features: &BarFeatures,
        bar: &VolumeBar,
        maha_dist: f64,
        iso_score: f64,
        vector: &FeatureVector,
        regime: MarketRegime,
    ) -> Option<AnomalySignal> {
        if anomalies.is_empty() {
            return None;
        }

        let passed_null = self.null_edge.validate(
            self.cfg.method, &self.window, vector, maha_dist, iso_score, &self.maha, &self.iso,
        );
        if !passed_null {
            return None;
        }

        let direction = vote_direction(anomalies)?;
        let (signal_type, description) = classify_signal_type(features, &direction, anomalies, regime);
        let confidence = composite_confidence(anomalies, maha_dist, iso_score, &self.cfg);
        if confidence < self.cfg.min_confidence {
            return None;
        }

        let expected_move = expected_move_bps(maha_dist, iso_score, &self.cfg);
        let min_move = self.cfg.fee_bps + self.cfg.edge_margin_bps;
        if expected_move < min_move {
            return None;
        }

        let hold_bars = hold_bars(maha_dist, iso_score, &self.cfg);
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
            isolation_score: iso_score,
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

fn detector_triggered(
    method: DetectionMethod,
    maha_dist: f64,
    iso_score: f64,
    maha: &MahalanobisDetector,
    iso: &IsolationForestDetector,
) -> bool {
    let maha_hit = maha_dist >= maha.threshold();
    let iso_hit = iso_score >= iso.threshold();
    match method {
        DetectionMethod::Mahalanobis => maha_hit,
        DetectionMethod::IsolationForest => iso_hit,
        DetectionMethod::Both => maha_hit && iso_hit,
        DetectionMethod::Either => maha_hit || iso_hit,
    }
}

fn classify_feature_anomalies(
    f: &BarFeatures,
    bar: &VolumeBar,
    z_fdr: &[(f64, bool); crate::FEATURE_DIM],
) -> Vec<AnomalyEvent> {
    let zt = 2.0;
    let mut out = Vec::new();

    push_anomaly(&mut out, bar, AnomalyKind::Ofi, z_fdr[0], f.ofi_normalized, zt, true);
    push_anomaly(&mut out, bar, AnomalyKind::Cvd, z_fdr[1], f.cvd_delta, zt, true);
    push_anomaly(&mut out, bar, AnomalyKind::DepthImbalance, z_fdr[2], f.depth_imbalance, zt, true);
    push_anomaly(&mut out, bar, AnomalyKind::Absorption, z_fdr[3], f.absorption, zt, true);

    push_anomaly(&mut out, bar, AnomalyKind::LiquidityVacuum, z_fdr[4], f.liquidity_vacuum, zt, false);
    // Correct direction for vacuum: net bid vacuum = short pressure, net ask vacuum = long pressure
    let signed_vac = f.ask_vacuum - f.bid_vacuum;
    if let Some(event) = out.last_mut() {
        if event.kind == AnomalyKind::LiquidityVacuum {
            event.direction = if signed_vac > 0.0 {
                SignalDirection::Long
            } else if signed_vac < 0.0 {
                SignalDirection::Short
            } else {
                SignalDirection::Neutral
            };
        }
    }

    push_anomaly(&mut out, bar, AnomalyKind::VolDeltaDivergence, z_fdr[5], f.vol_delta_divergence, zt, false);
    // Skip index 6 (cvd_acceleration_normalized) — no separate anomaly kind for it
    push_anomaly(&mut out, bar, AnomalyKind::AggressorImbalance, z_fdr[7], f.aggressor_ratio, zt, true);
    push_anomaly(&mut out, bar, AnomalyKind::LargePrint, z_fdr[8], f.large_print_imbalance, zt, true);
    push_anomaly(&mut out, bar, AnomalyKind::TradeIntensity, z_fdr[9], f.trade_intensity, zt, false);

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

fn classify_signal_type(
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

    let regime_str = format!("{}", regime);

    if has_divergence || (has_absorption && !flow_aligns) {
        let desc = format!(
            "reversal[{}]: divergence={:.2} absorption={:.2} mid_ret={:.1}bps agg={:.3} large={:.3}",
            regime_str, f.vol_delta_divergence, f.absorption, f.mid_return_bps,
            f.aggressor_ratio, f.large_print_imbalance
        );
        (SignalType::Reversal, desc)
    } else if has_large_print || has_aggressor || has_trade_intensity {
        let desc = format!(
            "spot[{}]: ofi={:.3} cvd={:.2} agg={:.3} large={:.3} intensity={:.2}",
            regime_str, f.ofi_normalized, f.cvd_delta,
            f.aggressor_ratio, f.large_print_imbalance, f.trade_intensity
        );
        (SignalType::MomentumContinuation, desc)
    } else {
        let desc = format!(
            "momentum[{}]: ofi={:.3} cvd={:.2} imb={:.3} vacuum={:.3} agg={:.3} large={:.3} intensity={:.2}",
            regime_str, f.ofi_normalized, f.cvd_delta, f.depth_imbalance, f.liquidity_vacuum,
            f.aggressor_ratio, f.large_print_imbalance, f.trade_intensity
        );
        (SignalType::MomentumContinuation, desc)
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

fn composite_confidence(events: &[AnomalyEvent], maha: f64, iso: f64, cfg: &EngineConfig) -> f64 {
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
    let iso_boost = (iso - cfg.isolation_threshold).clamp(0.0, 0.2);
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

    (base + maha_boost + iso_boost + pattern_boost + agreement).min(1.0)
}

fn expected_move_bps(maha: f64, iso: f64, cfg: &EngineConfig) -> f64 {
    let raw = maha * cfg.expected_move_maha_coeff + iso * cfg.expected_move_iso_coeff;
    raw.max(cfg.fee_bps + cfg.edge_margin_bps)
}

fn hold_bars(maha: f64, iso: f64, cfg: &EngineConfig) -> u32 {
    let scale = (maha / cfg.mahalanobis_threshold)
        .max(iso / cfg.isolation_threshold)
        .max(1.0);
    let hold = (cfg.base_hold_bars as f64 * scale).round() as u32;
    hold.clamp(cfg.base_hold_bars, cfg.max_hold_bars)
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
