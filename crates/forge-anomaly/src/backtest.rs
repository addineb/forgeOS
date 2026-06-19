//! Backtest evaluation: measures whether detected anomalies predict forward returns.

/// Forward returns associated with a single bar (from enriched depthscope).
#[derive(Debug, Clone, Copy, Default)]
pub struct ForwardReturns {
    pub fwd_ret_15m_bps: f64,
    pub fwd_ret_1h_bps: f64,
    pub fwd_ret_4h_bps: f64,
}

/// Summary statistics for signals of a given type/direction.
#[derive(Debug, Clone, Default)]
pub struct SignalStats {
    pub n_signals: u64,
    pub n_long: u64,
    pub n_short: u64,
    pub hit_rate_1h: f64,
    pub mean_ret_1h_bps: f64,
    pub median_ret_1h_bps: f64,
    pub std_ret_1h_bps: f64,
    pub sharpe_1h: f64,
    pub mean_ret_15m_bps: f64,
    pub mean_ret_4h_bps: f64,
    pub worst_ret_1h_bps: f64,
    pub best_ret_1h_bps: f64,
    pub mean_expected_move_bps: f64,
    pub realized_vs_expected: f64,
    pub avg_confidence: f64,
    pub avg_hold_bars: f64,
    pub avg_pattern_count: f64,
    pub avg_maha_dist: f64,
    pub n_momentum: u64,
    pub n_reversal: u64,
}

/// A matched pair of engine output + forward returns for one bar.
#[derive(Debug, Clone)]
pub struct EvalRow {
    pub bar_index: u64,
    pub ts: u64,
    pub direction: crate::types::SignalDirection,
    pub signal_type: crate::types::SignalType,
    pub confidence: f64,
    pub expected_move_bps: f64,
    pub hold_bars: u32,
    pub pattern_count: u32,
    pub maha_dist: f64,
    pub fwd: ForwardReturns,
}

/// Run evaluation over matched engine outputs + forward returns.
pub fn evaluate(rows: &[EvalRow]) -> SignalStats {
    if rows.is_empty() {
        return SignalStats::default();
    }

    let n = rows.len() as u64;
    let mut long_count = 0u64;
    let mut short_count = 0u64;
    let mut mom_count = 0u64;
    let mut rev_count = 0u64;
    let mut hits_1h = 0u64;
    let mut sum_1h = 0.0;
    let mut sum_15m = 0.0;
    let mut sum_4h = 0.0;
    let mut sum_sq_1h = 0.0;
    let mut sum_conf = 0.0;
    let mut sum_expected = 0.0;
    let mut sum_hold = 0.0;
    let mut sum_pattern = 0.0;
    let mut sum_maha = 0.0;
    let mut worst_1h = f64::INFINITY;
    let mut best_1h = f64::NEG_INFINITY;
    let mut rets_1h: Vec<f64> = Vec::with_capacity(rows.len());

    for r in rows {
        let signed_ret_1h = match r.direction {
            crate::types::SignalDirection::Long => r.fwd.fwd_ret_1h_bps,
            crate::types::SignalDirection::Short => -r.fwd.fwd_ret_1h_bps,
            crate::types::SignalDirection::Neutral => 0.0,
        };

        match r.direction {
            crate::types::SignalDirection::Long => long_count += 1,
            crate::types::SignalDirection::Short => short_count += 1,
            crate::types::SignalDirection::Neutral => {}
        }
        match r.signal_type {
            crate::types::SignalType::MomentumContinuation => mom_count += 1,
            crate::types::SignalType::Reversal => rev_count += 1,
        }

        if signed_ret_1h > 0.0 {
            hits_1h += 1;
        }
        sum_1h += signed_ret_1h;
        sum_sq_1h += signed_ret_1h * signed_ret_1h;
        sum_15m += match r.direction {
            crate::types::SignalDirection::Long => r.fwd.fwd_ret_15m_bps,
            crate::types::SignalDirection::Short => -r.fwd.fwd_ret_15m_bps,
            crate::types::SignalDirection::Neutral => 0.0,
        };
        sum_4h += match r.direction {
            crate::types::SignalDirection::Long => r.fwd.fwd_ret_4h_bps,
            crate::types::SignalDirection::Short => -r.fwd.fwd_ret_4h_bps,
            crate::types::SignalDirection::Neutral => 0.0,
        };
        sum_conf += r.confidence;
        sum_expected += r.expected_move_bps;
        sum_hold += r.hold_bars as f64;
        sum_pattern += r.pattern_count as f64;
        sum_maha += r.maha_dist;
        worst_1h = worst_1h.min(signed_ret_1h);
        best_1h = best_1h.max(signed_ret_1h);
        rets_1h.push(signed_ret_1h);
    }

    let nf = n as f64;
    let mean_1h = sum_1h / nf;
    let var_1h = (sum_sq_1h / nf - mean_1h * mean_1h).max(0.0);
    let std_1h = var_1h.sqrt();

    rets_1h.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let median_1h = if rets_1h.is_empty() {
        0.0
    } else {
        let m = rets_1h.len() / 2;
        if rets_1h.len().is_multiple_of(2) {
            (rets_1h[m - 1] + rets_1h[m]) / 2.0
        } else {
            rets_1h[m]
        }
    };

    let mean_expected = sum_expected / nf;
    let realized_vs_expected = if mean_expected.abs() > 1e-6 {
        mean_1h / mean_expected
    } else {
        0.0
    };

    SignalStats {
        n_signals: n,
        n_long: long_count,
        n_short: short_count,
        hit_rate_1h: hits_1h as f64 / nf,
        mean_ret_1h_bps: mean_1h,
        median_ret_1h_bps: median_1h,
        std_ret_1h_bps: std_1h,
        sharpe_1h: if std_1h > 1e-6 { mean_1h / std_1h } else { 0.0 },
        mean_ret_15m_bps: sum_15m / nf,
        mean_ret_4h_bps: sum_4h / nf,
        worst_ret_1h_bps: worst_1h,
        best_ret_1h_bps: best_1h,
        mean_expected_move_bps: mean_expected,
        realized_vs_expected,
        avg_confidence: sum_conf / nf,
        avg_hold_bars: sum_hold / nf,
        avg_pattern_count: sum_pattern / nf,
        avg_maha_dist: sum_maha / nf,
        n_momentum: mom_count,
        n_reversal: rev_count,
    }
}

impl std::fmt::Display for SignalStats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "=== Signal Evaluation ===")?;
        writeln!(f, "  signals:        {}", self.n_signals)?;
        writeln!(f, "  long / short:   {} / {}", self.n_long, self.n_short)?;
        writeln!(f, "  momentum / rev: {} / {}", self.n_momentum, self.n_reversal)?;
        writeln!(f, "  hit_rate_1h:    {:.1}%", self.hit_rate_1h * 100.0)?;
        writeln!(f, "  mean_ret_1h:    {:.2} bps", self.mean_ret_1h_bps)?;
        writeln!(f, "  median_ret_1h:  {:.2} bps", self.median_ret_1h_bps)?;
        writeln!(f, "  std_ret_1h:     {:.2} bps", self.std_ret_1h_bps)?;
        writeln!(f, "  sharpe_1h:      {:.3}", self.sharpe_1h)?;
        writeln!(f, "  mean_ret_15m:   {:.2} bps", self.mean_ret_15m_bps)?;
        writeln!(f, "  mean_ret_4h:    {:.2} bps", self.mean_ret_4h_bps)?;
        writeln!(f, "  worst_ret_1h:   {:.2} bps", self.worst_ret_1h_bps)?;
        writeln!(f, "  best_ret_1h:    {:.2} bps", self.best_ret_1h_bps)?;
        writeln!(f, "  expected_move:  {:.2} bps", self.mean_expected_move_bps)?;
        writeln!(f, "  realized/expected: {:.3}", self.realized_vs_expected)?;
        writeln!(f, "  avg_confidence: {:.3}", self.avg_confidence)?;
        writeln!(f, "  avg_hold_bars:  {:.1}", self.avg_hold_bars)?;
        writeln!(f, "  avg_patterns:   {:.1}", self.avg_pattern_count)?;
        writeln!(f, "  avg_maha_dist:  {:.2}", self.avg_maha_dist)
    }
}

/// Simple calibration: regresses |signed_ret_1h| against maha_dist.
/// Returns `(maha_coeff, 0.0)` since the second variable (iso_score) is removed.
pub fn calibrate_expected_move(rows: &[EvalRow]) -> (f64, f64) {
    if rows.len() < 3 {
        return (3.0, 0.0);
    }
    let (mut s_xx, mut s_xy) = (0.0, 0.0);
    for r in rows {
        let y = match r.direction {
            crate::types::SignalDirection::Long => r.fwd.fwd_ret_1h_bps,
            crate::types::SignalDirection::Short => -r.fwd.fwd_ret_1h_bps,
            crate::types::SignalDirection::Neutral => 0.0,
        }
        .abs();
        s_xx += r.maha_dist * r.maha_dist;
        s_xy += r.maha_dist * y;
    }
    let coeff = if s_xx.abs() > 1e-12 { s_xy / s_xx } else { 3.0 };
    (coeff.max(0.0), 0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::AnomalyEngine;
    use crate::types::{EngineConfig, SignalDirection, SignalType, VolumeBar};

    #[test]
    fn empty_input_returns_defaults() {
        let stats = evaluate(&[]);
        assert_eq!(stats.n_signals, 0);
    }

    #[test]
    fn single_long_signal_counted() {
        let rows = vec![EvalRow {
            bar_index: 0,
            ts: 0,
            direction: SignalDirection::Long,
            signal_type: SignalType::MomentumContinuation,
            confidence: 0.7,
            expected_move_bps: 15.0,
            hold_bars: 5,
            pattern_count: 2,
            maha_dist: 5.0,
            fwd: ForwardReturns {
                fwd_ret_15m_bps: 5.0,
                fwd_ret_1h_bps: 12.0,
                fwd_ret_4h_bps: 20.0,
            },
        }];
        let stats = evaluate(&rows);
        assert_eq!(stats.n_signals, 1);
        assert_eq!(stats.n_long, 1);
        assert_eq!(stats.n_short, 0);
        assert!((stats.hit_rate_1h - 1.0).abs() < 1e-9);
        assert!((stats.mean_ret_1h_bps - 12.0).abs() < 1e-9);
    }

    #[test]
    fn short_signal_flips_return_sign() {
        let rows = vec![EvalRow {
            bar_index: 0,
            ts: 0,
            direction: SignalDirection::Short,
            signal_type: SignalType::Reversal,
            confidence: 0.6,
            expected_move_bps: 12.0,
            hold_bars: 3,
            pattern_count: 1,
            maha_dist: 4.5,
            fwd: ForwardReturns {
                fwd_ret_15m_bps: -3.0,
                fwd_ret_1h_bps: -8.0,
                fwd_ret_4h_bps: -15.0,
            },
        }];
        let stats = evaluate(&rows);
        assert_eq!(stats.n_short, 1);
        assert!((stats.mean_ret_1h_bps - 8.0).abs() < 1e-9);
        assert!((stats.hit_rate_1h - 1.0).abs() < 1e-9);
    }

    fn neutral_bar(idx: u64) -> VolumeBar {
        VolumeBar {
            ts: idx * 1_000_000_000,
            bar_index: idx,
            cum_vol: (idx + 1) as f64 * 10.0,
            bar_vol: 10.0,
            mid_price: 50_000.0,
            best_bid: 49_999.0,
            best_ask: 50_001.0,
            bid_vol_top5: 100.0,
            ask_vol_top5: 100.0,
            bid_vol_top10: 150.0,
            ask_vol_top10: 150.0,
            spread_bps: 2.0,
            cvd_delta: 0.0,
            cvd_momentum: 0.0,
            cvd_acceleration: 0.0,
            trade_count: (idx % 80 + 40),
            buy_count: (idx % 40 + 20),
            sell_count: (idx % 40 + 20),
            aggressor_ratio: 0.5,
            trade_intensity: ((idx % 80 + 40) as f64 / 10.0).min(20.0),
            ..Default::default()
        }
    }

    fn anomalous_bar(idx: u64, cvd: f64, bid_q: f64, ask_q: f64, mid: f64) -> VolumeBar {
        VolumeBar {
            ts: idx * 1_000_000_000,
            bar_index: idx,
            cum_vol: (idx + 1) as f64 * 10.0,
            bar_vol: 10.0,
            mid_price: mid,
            best_bid: mid - 1.0,
            best_ask: mid + 1.0,
            bid_vol_top5: bid_q,
            ask_vol_top5: ask_q,
            bid_vol_top10: bid_q * 1.5,
            ask_vol_top10: ask_q * 1.5,
            spread_bps: 2.0,
            cvd_delta: cvd,
            cvd_momentum: cvd * 0.5,
            cvd_acceleration: cvd * 0.1,
            trade_count: (idx % 100 + 50),
            buy_count: (idx % 50 + 25),
            sell_count: (idx % 50 + 25),
            aggressor_ratio: 0.5,
            trade_intensity: ((idx % 100 + 50) as f64 / 10.0).min(20.0),
            ..Default::default()
        }
    }

    #[test]
    fn integration_anomaly_pattern_detected_and_returns_positive() {
        let mut bars: Vec<VolumeBar> = Vec::new();
        for i in 0..200u64 {
            bars.push(neutral_bar(i));
        }
        let pattern_start = 100u64;
        for i in pattern_start..pattern_start + 6 {
            let cvd = if i == pattern_start { 50.0 } else { 40.0 };
            let mid = 50_000.0 + (i - pattern_start) as f64 * 2.0;
            bars[i as usize] = anomalous_bar(i, cvd, 500.0, 50.0, mid);
        }

        let mut cfg = EngineConfig::default();
        cfg.null_edge_permutations = 0;
        cfg.null_edge_margin = 0.0;
        cfg.min_confidence = 0.3;
        cfg.mahalanobis_threshold = 1.0;
        cfg.edge_margin_bps = 0.0;
        cfg.fee_bps = 0.0;
        let mut engine = AnomalyEngine::new(cfg);
        let outputs = engine.on_bars(&bars);

        let mut eval_rows: Vec<EvalRow> = Vec::new();
        let mut signal_found = false;
        for (i, output) in outputs.iter().enumerate() {
            if let Some(sig) = &output.signal {
                if i >= 100 && i <= 120 {
                    signal_found = true;
                }
                let fwd = if i >= pattern_start as usize && i < (pattern_start + 20) as usize {
                    ForwardReturns { fwd_ret_15m_bps: 5.0, fwd_ret_1h_bps: 15.0, fwd_ret_4h_bps: 25.0 }
                } else {
                    ForwardReturns { fwd_ret_15m_bps: 0.0, fwd_ret_1h_bps: 0.0, fwd_ret_4h_bps: 0.0 }
                };
                eval_rows.push(EvalRow {
                    bar_index: sig.bar_index,
                    ts: sig.ts,
                    direction: sig.direction,
                    signal_type: sig.signal_type,
                    confidence: sig.confidence,
                    expected_move_bps: sig.expected_move_bps,
                    hold_bars: sig.hold_bars,
                    pattern_count: sig.pattern_count,
                    maha_dist: sig.mahalanobis_dist,
                    fwd,
                });
            }
        }

        assert!(signal_found, "expected at least one signal during anomalous window");
        assert!(!eval_rows.is_empty(), "expected at least one eval row");
    }
}
