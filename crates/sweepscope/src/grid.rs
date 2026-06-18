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
    /// Minimal focused grid: only the 3 best hypothesis families, 1-2 thresholds each.
    /// DSR ∝ √N_trials — must keep trial count LOW to let real edges survive.
    ///
    /// Hypotheses from v5/v6 results:
    ///   H1: OI collapse → short (momentum of forced closing)
    ///   H2: Sustained CVD sell momentum → short (directional pressure)
    ///   H3: Persistent ask skew → short (supply overhang)
    ///   H4: Liquidation cascade → short (forced selling continuation)
    ///
    /// Hold: 100-600 bars (30min-4h). TP/SL: wide for macro moves.
    pub fn default() -> Self {
        Self {
            entries: vec![
                // H1: OI collapse short (strongest from v5)
                ("oi_surge_short_25".into(), vec![-1.0, -2.0]),
                ("oi_surge_short_50".into(), vec![-1.0, -2.0]),
                ("oi_unwind_short".into(), vec![-0.5, -1.0]),

                // H2: CVD momentum cumulative (sustained selling)
                ("cvd_mom_cum_short_25".into(), vec![-100.0, -200.0]),
                ("cvd_push_short_25".into(), vec![-40000.0, -80000.0]),

                // H3: Persistent depth skew (supply overhang)
                ("ask_skew_sust_short_25".into(), vec![0.60]),
                ("ask_skew_sust_short_50".into(), vec![0.60]),

                // H4: Liquidation cascade
                ("liq_cascade_sell_25".into(), vec![20.0, 35.0]),
                ("liq_cascade_sell_50".into(), vec![35.0, 55.0]),

                // Long-side counterparts (check symmetry)
                ("oi_surge_long_25".into(), vec![1.0, 2.0]),
                ("cvd_mom_cum_long_25".into(), vec![100.0, 200.0]),
                ("bid_skew_sust_long_25".into(), vec![0.60]),
                ("liq_cascade_buy_25".into(), vec![10.0, 20.0]),
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
