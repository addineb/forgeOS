//! Sweep grid: entry parameters × exit parameters.
//!
//! The grid is a Cartesian product of:
//! - Entry signal (which feature + threshold)
//! - Exit parameters (TP, SL, hold duration)
//!
//! From mlfinlab: the triple-barrier method means we sweep TP/SL as
//! WIDTHS (in bps), not as fixed targets. The first barrier hit
//! determines the outcome. This is less overfit-prone than fixed
//! TP/SL because the exit adapts to realized volatility.

/// One config in the sweep grid.
#[derive(Clone, Debug)]
pub struct SweepConfig {
    pub entry_name: String,
    pub entry_threshold: f64,
    pub tp_bps: f64,
    pub sl_bps: f64,
    pub hold_bars: usize,
}

/// The full sweep grid specification.
pub struct SweepGrid {
    pub entries: Vec<(String, Vec<f64>)>,
    pub tp_bps: Vec<f64>,
    pub sl_bps: Vec<f64>,
    pub hold_bars: Vec<usize>,
}

impl SweepGrid {
    /// Direction-aware grid. _long and _short suffixes encode direction.
    /// BUT: verdict is per-family, not per-direction. If a signal only works
    /// one way, the whole family retires — "if it doesn't work both ways,
    /// it doesn't work at all."
    ///
    /// Hold: 100-600 bars (30min-4h). TP/SL: wide for macro moves.
    pub fn default() -> Self {
        Self {
            entries: vec![
                // Shared threshold ranges for each hypothesis family
                // OI collapse/expansion
                ("oi_surge_short_25".into(), vec![-2.0, -1.0]),
                ("oi_surge_short_50".into(), vec![-2.0, -1.0]),
                ("oi_surge_long_25".into(),  vec![0.5, 1.0, 2.0]),
                ("oi_surge_long_50".into(),  vec![0.5, 1.0, 2.0]),
                ("oi_unwind_short".into(),   vec![-1.0, -0.5]),
                ("oi_unwind_long".into(),    vec![0.5, 1.0, 2.0]),

                // CVD sustained momentum
                ("cvd_mom_cum_short_25".into(), vec![-200.0, -100.0]),
                ("cvd_mom_cum_short_50".into(), vec![-200.0, -100.0]),
                ("cvd_mom_cum_long_25".into(),  vec![25.0, 50.0, 100.0]),
                ("cvd_mom_cum_long_50".into(),  vec![50.0, 100.0, 200.0]),
                ("cvd_push_short_25".into(),    vec![-80000.0, -40000.0]),
                ("cvd_push_short_50".into(),    vec![-200000.0, -100000.0]),
                ("cvd_push_long_25".into(),     vec![25000.0, 50000.0]),
                ("cvd_push_long_50".into(),     vec![50000.0, 100000.0]),

                // Sustained depth skew
                ("ask_skew_sust_short_25".into(), vec![0.60, 0.80]),
                ("ask_skew_sust_short_50".into(), vec![0.60, 0.80]),
                ("bid_skew_sust_long_25".into(),  vec![0.40, 0.60, 0.80]),
                ("bid_skew_sust_long_50".into(),  vec![0.40, 0.60, 0.80]),

                // Liquidation cascade
                ("liq_cascade_sell_25".into(), vec![20.0, 35.0, 55.0]),
                ("liq_cascade_sell_50".into(), vec![35.0, 55.0, 80.0]),
                ("liq_cascade_buy_25".into(),  vec![5.0, 10.0, 20.0]),
                ("liq_cascade_buy_50".into(),  vec![10.0, 20.0, 40.0]),

                // Liquidation flow imbalance
                ("liq_flow_sell_25".into(), vec![-0.5, -0.2]),
                ("liq_flow_sell_50".into(), vec![-0.5, -0.2]),
                ("liq_flow_buy_25".into(),  vec![0.2, 0.5]),
                ("liq_flow_buy_50".into(),  vec![0.2, 0.5]),

                // Sustained funding
                ("funding_crowd_short_25".into(), vec![0.0001, 0.0002]),
                ("funding_crowd_short_50".into(), vec![0.0001, 0.0002]),
                ("funding_crowd_long_25".into(),  vec![-0.0001, -0.0002]),
                ("funding_crowd_long_50".into(),  vec![-0.0001, -0.0002]),

                // Mark-index sustained premium/discount
                ("mi_premium_short_25".into(),  vec![5.0, 10.0]),
                ("mi_premium_short_50".into(),  vec![5.0, 10.0]),
                ("mi_discount_long_25".into(),  vec![-5.0, -10.0]),
                ("mi_discount_long_50".into(),  vec![-5.0, -10.0]),

                // Single-bar signals
                ("funding_extreme_long".into(),   vec![-0.0001, -0.0002]),
                ("funding_extreme_short".into(),  vec![0.0001, 0.0002]),
                ("mark_index_discount_long".into(), vec![-5.0, -10.0]),
                ("mark_index_premium_short".into(), vec![5.0, 10.0]),
            ],
            tp_bps: vec![0.0, 20.0, 40.0, 80.0],
            sl_bps: vec![0.0, 40.0, 80.0],
            hold_bars: vec![100, 200, 600],
        }
    }

    /// Expand the grid into concrete configs (Cartesian product).
    pub fn expand(&self) -> Vec<SweepConfig> {
        let mut out = Vec::new();
        for (entry_name, thresholds) in &self.entries {
            for &threshold in thresholds {
                for &tp in &self.tp_bps {
                    for &sl in &self.sl_bps {
                        for &hold in &self.hold_bars {
                            if tp > 0.0 && sl > 0.0 && tp < sl * 0.3 {
                                continue;
                            }
                            out.push(SweepConfig {
                                entry_name: entry_name.clone(),
                                entry_threshold: threshold,
                                tp_bps: tp,
                                sl_bps: sl,
                                hold_bars: hold,
                            });
                        }
                    }
                }
            }
        }
        out
    }
}
