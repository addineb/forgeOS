//! Anomaly engine: processes volume bars, detects anomalies, emits signals.

use crate::detector::MahalanobisDetector;
use crate::features::FeatureExtractor;
use crate::null_edge::NullEdgeGate;
use crate::pattern::PatternCounter;
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
}

/// Volume-bar anomaly detection engine.
///
/// Owns the feature extractor, rolling window, Mahalanobis detector,
/// pattern counter, and null-edge gate. Orchestrates them in a linear
/// pipeline on each bar.
pub struct AnomalyEngine {
    cfg: EngineConfig,
    features: FeatureExtractor,
    window: RollingFeatureWindow,
    maha: MahalanobisDetector,
    patterns: PatternCounter,
    null_edge: NullEdgeGate,
    bars_processed: u64,
    signals_emitted: u64,
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

        let f = match self.features.observe(bar) {
            Some(f) => f,
            None => return self.empty_output(bar.bar_index),
        };

        let vector = f.to_vector();
        let maha_dist = self.maha.distance_or_zero(&self.window, &vector);

        let z_fdr = self.window.z_scores_fdr(&vector, self.cfg.fdr_alpha);
        let mut anomalies = classify_anomalies(&f, bar, &z_fdr);

        self.window.push(vector);

        if !self.window.ready() {
            return EngineOutput { anomalies, mahalanobis_dist: maha_dist, ..self.empty_output(0) };
        }

        let fired = maha_dist >= self.maha.threshold();
        if fired && !anomalies.is_empty() {
            track_patterns(&mut self.patterns, &mut anomalies, bar, self.cfg.min_pattern_count);
        }

        let signal = if fired {
            self.compose_signal(&anomalies, &f, bar, maha_dist, &vector)
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
        }
    }

    /// Process a batch of bars.
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

    fn compose_signal(
        &mut self,
        anomalies: &[AnomalyEvent],
        f: &BarFeatures,
        bar: &VolumeBar,
        maha_dist: f64,
        vector: &FeatureVector,
    ) -> Option<AnomalySignal> {
        if !self.null_edge.validate(&self.window, vector, maha_dist, &self.maha) {
            return None;
        }

        let direction = vote_direction(anomalies)?;
        let signal_type = classify_signal_type(f, anomalies);
        let confidence = composite_confidence(anomalies, maha_dist, &self.cfg);
        if confidence < self.cfg.min_confidence {
            return None;
        }

        let expected_move = maha_dist * self.cfg.expected_move_maha_coeff;
        if expected_move < self.cfg.fee_bps + self.cfg.edge_margin_bps {
            return None;
        }

        let hold_bars = ((self.cfg.base_hold_bars as f64)
            * (maha_dist / self.cfg.mahalanobis_threshold).max(1.0))
        .round() as u32;
        let hold_bars = hold_bars.clamp(self.cfg.base_hold_bars, self.cfg.max_hold_bars);

        let pattern_count = anomalies
            .iter()
            .find(|e| e.kind == AnomalyKind::PatternRepeat)
            .map(|e| e.raw_value as u32)
            .unwrap_or(1);

        let desc = anomaly_description(f, anomalies, signal_type);

        Some(AnomalySignal {
            bar_index: bar.bar_index,
            ts: bar.ts,
            signal_type,
            direction,
            confidence,
            description: desc,
            mahalanobis_dist: maha_dist,
            pattern_count,
            expected_move_bps: expected_move,
            hold_bars,
            passed_null_edge: true,
            events: anomalies.to_vec(),
        })
    }
}

// ─── Free functions: execution logic, no state ─────────────────────────────────

fn track_patterns(
    patterns: &mut PatternCounter,
    anomalies: &mut Vec<AnomalyEvent>,
    bar: &VolumeBar,
    min_pattern_count: u32,
) {
    let key = PatternCounter::signature(anomalies);
    let count = patterns.record(key);
    if patterns.is_repetitive(key) {
        let dir = vote_direction(anomalies).unwrap_or(SignalDirection::Neutral);
        anomalies.push(AnomalyEvent {
            bar_index: bar.bar_index,
            ts: bar.ts,
            kind: AnomalyKind::PatternRepeat,
            direction: dir,
            z_score: count as f64,
            raw_value: count as f64,
            confidence: (count as f64 / min_pattern_count as f64).min(1.0),
        });
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

fn classify_signal_type(f: &BarFeatures, events: &[AnomalyEvent]) -> SignalType {
    let has_divergence = events.iter().any(|e| e.kind == AnomalyKind::VolDeltaDivergence);
    let has_absorption = events.iter().any(|e| e.kind == AnomalyKind::Absorption);
    let flow_aligns = f.ofi_normalized.signum() == f.cvd_delta.signum()
        && f.ofi_normalized.abs() > 0.001
        && f.cvd_delta.abs() > 0.001;
    if has_divergence || (has_absorption && !flow_aligns) {
        SignalType::Reversal
    } else {
        SignalType::MomentumContinuation
    }
}

fn composite_confidence(events: &[AnomalyEvent], maha: f64, cfg: &EngineConfig) -> f64 {
    let (mut total, mut weight) = (0.0, 0.0);
    let mut kinds = 0u32;
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

fn anomaly_description(f: &BarFeatures, events: &[AnomalyEvent], sig_type: SignalType) -> String {
    let label = match sig_type {
        SignalType::Reversal => "reversal",
        SignalType::MomentumContinuation => {
            let has_spot = events.iter().any(|e| matches!(e.kind,
                AnomalyKind::LargePrint | AnomalyKind::AggressorImbalance | AnomalyKind::TradeIntensity
            ));
            if has_spot { "spot" } else { "momentum" }
        }
    };
    format!(
        "[{}] ofi={:.3} cvd={:.2} imb={:.3} abs={:.2} vac={:.3} div={:.2} agg={:.3} lp={:.3} int={:.2} ret={:.1}bps",
        label, f.ofi_normalized, f.cvd_delta, f.depth_imbalance, f.absorption,
        f.liquidity_vacuum, f.vol_delta_divergence, f.aggressor_ratio,
        f.large_print_imbalance, f.trade_intensity, f.mid_return_bps,
    )
}

// ─── Per-feature anomaly classification ────────────────────────────────────────

fn classify_anomalies(
    f: &BarFeatures,
    bar: &VolumeBar,
    z_fdr: &[(f64, bool); crate::FEATURE_DIM],
) -> Vec<AnomalyEvent> {
    let z_thresh = 2.0;
    let mut out = Vec::new();

    push_anomaly(&mut out, bar, AnomalyKind::Ofi, z_fdr[0], f.ofi_normalized, z_thresh, true);
    push_anomaly(&mut out, bar, AnomalyKind::Cvd, z_fdr[1], f.cvd_delta, z_thresh, true);
    push_anomaly(&mut out, bar, AnomalyKind::DepthImbalance, z_fdr[2], f.depth_imbalance, z_thresh, true);
    push_anomaly(&mut out, bar, AnomalyKind::Absorption, z_fdr[3], f.absorption, z_thresh, true);

    push_anomaly(&mut out, bar, AnomalyKind::LiquidityVacuum, z_fdr[4], f.liquidity_vacuum, z_thresh, false);
    if let Some(ev) = out.last_mut() {
        if ev.kind == AnomalyKind::LiquidityVacuum {
            let signed = f.ask_vacuum - f.bid_vacuum;
            ev.direction = if signed > 0.0 {
                SignalDirection::Long
            } else if signed < 0.0 {
                SignalDirection::Short
            } else {
                SignalDirection::Neutral
            };
        }
    }

    push_anomaly(&mut out, bar, AnomalyKind::VolDeltaDivergence, z_fdr[5], f.vol_delta_divergence, z_thresh, false);
    push_anomaly(&mut out, bar, AnomalyKind::AggressorImbalance, z_fdr[7], f.aggressor_ratio, z_thresh, true);
    push_anomaly(&mut out, bar, AnomalyKind::LargePrint, z_fdr[8], f.large_print_imbalance, z_thresh, true);
    push_anomaly(&mut out, bar, AnomalyKind::TradeIntensity, z_fdr[9], f.trade_intensity, z_thresh, false);

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
            z_dir(z)
        }
    } else if momentum_direction {
        z_dir(z)
    } else {
        z_dir(z)
    };
    out.push(AnomalyEvent {
        bar_index: bar.bar_index,
        ts: bar.ts,
        kind,
        direction,
        z_score: z,
        raw_value: raw,
        confidence: ((z.abs() - threshold) / threshold).clamp(0.0, 1.0),
    });
}

fn z_dir(z: f64) -> SignalDirection {
    if z > 0.0 {
        SignalDirection::Long
    } else {
        SignalDirection::Short
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
            let out = engine.on_bar(&synth(i, 100.0, 0.0));
            assert!(out.signal.is_none());
        }
    }
}
