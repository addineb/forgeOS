use crate::detector::MahalanobisDetector;
use crate::features::FeatureExtractor;
use crate::null_edge::NullEdgeGate;
use crate::pattern::PatternCounter;
use crate::stats::RollingFeatureWindow;
use crate::types::{
    AnomalyEvent, AnomalyKind, AnomalySignal, BarFeatures, EngineConfig,
    FeatureVector, SignalDirection, SignalType, VolumeBar,
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
    pub fn config(&self) -> &EngineConfig { &self.cfg }

    #[must_use]
    pub fn bars_processed(&self) -> u64 { self.bars_processed }

    #[must_use]
    pub fn ready(&self) -> bool { self.window.ready() }

    pub fn on_bar(&mut self, bar: &VolumeBar) -> EngineOutput {
        self.bars_processed += 1;
        self.null_edge.on_bar();

        let Some(f) = self.features.observe(bar) else {
            return self.empty_output(bar.bar_index);
        };

        let vec = f.to_vector();
        let dist = self.maha.distance_or_zero(&self.window, &vec);
        let mut anomalies = self.detect_anomalies(&f, bar, &vec);
        self.window.push(vec);

        if !self.window.ready() {
            return EngineOutput { anomalies, mahalanobis_dist: dist, ..self.empty_output(0) };
        }

        let fired = dist >= self.maha.threshold();
        if fired && !anomalies.is_empty() {
            self.update_patterns(&mut anomalies, bar);
        }

        let signal = fired.then(|| self.try_signal(&anomalies, &f, bar, dist)).flatten();
        if signal.is_some() { self.null_edge.on_signal(); }

        EngineOutput { bar_index: bar.bar_index, anomalies, signal, mahalanobis_dist: dist }
    }

    pub fn on_bars(&mut self, bars: &[VolumeBar]) -> Vec<EngineOutput> {
        bars.iter().map(|b| self.on_bar(b)).collect()
    }

    fn empty_output(&self, bar_index: u64) -> EngineOutput {
        EngineOutput { bar_index, anomalies: Vec::new(), signal: None, mahalanobis_dist: 0.0 }
    }

    fn detect_anomalies(
        &self, f: &BarFeatures, bar: &VolumeBar, vec: &FeatureVector,
    ) -> Vec<AnomalyEvent> {
        let z_fdr = self.window.z_scores_fdr(vec, self.cfg.fdr_alpha);
        let t = 2.0_f64;
        let mut out = Vec::new();
        let pairs: &[(AnomalyKind, usize, f64)] = &[
            (AnomalyKind::Ofi, 0, f.ofi_normalized),
            (AnomalyKind::Cvd, 1, f.cvd_delta),
            (AnomalyKind::DepthImbalance, 2, f.depth_imbalance),
            (AnomalyKind::Absorption, 3, f.absorption),
            (AnomalyKind::LiquidityVacuum, 4, f.liquidity_vacuum),
            (AnomalyKind::VolDeltaDivergence, 5, f.vol_delta_divergence),
            (AnomalyKind::AggressorImbalance, 7, f.aggressor_ratio),
            (AnomalyKind::LargePrint, 8, f.large_print_imbalance),
            (AnomalyKind::TradeIntensity, 9, f.trade_intensity),
        ];
        for &(kind, idx, raw) in pairs {
            let (z, sig) = z_fdr[idx];
            if z.abs() < t || !sig { continue; }
            let dir = if kind == AnomalyKind::VolDeltaDivergence {
                vol_delta_dir(raw, bar.cvd_delta, z)
            } else {
                z_sign(z)
            };
            out.push(AnomalyEvent {
                bar_index: bar.bar_index, ts: bar.ts, kind, direction: dir,
                z_score: z, raw_value: raw, confidence: ((z.abs() - t) / t).clamp(0.0, 1.0),
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
            anomalies.push(AnomalyEvent {
                bar_index: bar.bar_index, ts: bar.ts,
                kind: AnomalyKind::PatternRepeat, direction: dir,
                z_score: count as f64, raw_value: count as f64,
                confidence: (count as f64 / self.cfg.min_pattern_count as f64).min(1.0),
            });
        }
    }

    fn try_signal(
        &mut self, anomalies: &[AnomalyEvent], f: &BarFeatures, bar: &VolumeBar, dist: f64,
    ) -> Option<AnomalySignal> {
        let vec = f.to_vector();
        if !self.null_edge.validate(&self.window, &vec, dist, &self.maha) { return None; }

        let direction = weighted_dir(anomalies)?;

        let (confidence, pattern_count) = calc_confidence(anomalies, dist, &self.cfg);
        if confidence < self.cfg.min_confidence { return None; }

        let expected_move = dist * self.cfg.expected_move_maha_coeff;
        if expected_move < self.cfg.fee_bps + self.cfg.edge_margin_bps { return None; }

        let flow_aligns = f.ofi_normalized.signum() == f.cvd_delta.signum()
            && f.ofi_normalized.abs() > 0.001 && f.cvd_delta.abs() > 0.001;
        let (signal_type, label) = classify_signal(anomalies, flow_aligns);

        let hold_bars = (((self.cfg.base_hold_bars as f64)
            * (dist / self.cfg.mahalanobis_threshold).max(1.0)).round() as u32)
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
    let (lw, sw) = events.iter()
        .filter(|e| e.kind != AnomalyKind::PatternRepeat)
        .fold((0.0, 0.0), |(l, s), e| match e.direction {
            SignalDirection::Long => (l + e.z_score.abs(), s),
            SignalDirection::Short => (l, s + e.z_score.abs()),
            SignalDirection::Neutral => (l, s),
        });
    if lw > sw && lw > 0.0 { Some(SignalDirection::Long) }
    else if sw > lw && sw > 0.0 { Some(SignalDirection::Short) }
    else { None }
}

fn z_sign(z: f64) -> SignalDirection {
    if z > 0.0 { SignalDirection::Long } else { SignalDirection::Short }
}

fn vol_delta_dir(raw: f64, cvd_delta: f64, z: f64) -> SignalDirection {
    if raw > 0.0 && cvd_delta > 0.0 { SignalDirection::Long }
    else if raw > 0.0 && cvd_delta < 0.0 { SignalDirection::Short }
    else { z_sign(z) }
}

fn vac_dir(signed: f64) -> SignalDirection {
    if signed > 0.0 { SignalDirection::Long }
    else if signed < 0.0 { SignalDirection::Short }
    else { SignalDirection::Neutral }
}

fn calc_confidence(events: &[AnomalyEvent], maha: f64, cfg: &EngineConfig) -> (f64, u32) {
    let (mut total, mut weight) = (0.0, 0.0);
    let mut kinds = 0u32;
    let mut pattern_conf = 0.0;
    let mut pattern_count = 1_u32;
    for e in events {
        if e.kind == AnomalyKind::PatternRepeat {
            pattern_conf = e.confidence * 0.15;
            pattern_count = e.raw_value as u32;
            continue;
        }
        kinds += 1;
        let w = e.z_score.abs().max(1.0);
        total += e.confidence * w;
        weight += w;
    }
    let base = if weight > 0.0 { total / weight } else { 0.0 };
    let maha_boost = (maha / cfg.mahalanobis_threshold - 1.0).clamp(0.0, 0.2);
    let agreement = match kinds { 0 | 1 => 0.0, 2 => 0.08, _ => 0.15 };
    ((base + maha_boost + pattern_conf + agreement).min(1.0), pattern_count)
}

fn classify_signal(events: &[AnomalyEvent], flow_aligns: bool) -> (SignalType, &'static str) {
    let mut has_div = false;
    let mut has_abs = false;
    let mut has_spot = false;
    for e in events {
        match e.kind {
            AnomalyKind::VolDeltaDivergence => has_div = true,
            AnomalyKind::Absorption => has_abs = true,
            AnomalyKind::LargePrint | AnomalyKind::AggressorImbalance | AnomalyKind::TradeIntensity => has_spot = true,
            _ => {}
        }
    }
    if has_div || (has_abs && !flow_aligns) { (SignalType::Reversal, "reversal") }
    else if has_spot { (SignalType::MomentumContinuation, "spot") }
    else { (SignalType::MomentumContinuation, "momentum") }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synth(idx: u64, bid_q: f64, cvd: f64) -> VolumeBar {
        VolumeBar {
            ts: idx * 1_000_000_000, bar_index: idx,
            cum_vol: (idx + 1) as f64 * 10.0, bar_vol: 10.0,
            mid_price: 50_000.0, best_bid: 49_999.0, best_ask: 50_001.0,
            bid_vol_top5: bid_q, ask_vol_top5: 100.0,
            bid_vol_top10: bid_q * 1.5, ask_vol_top10: 150.0,
            cvd_delta: cvd, ..Default::default()
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
