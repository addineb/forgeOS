//! Signal source trait and built-in implementations.
//!
//! A `SignalSource` takes a slice of bars and produces a list of raw signal
//! entries `(bar_index, is_long, expected_move_bps, hold_bars, family)`.
//! The same downstream `run_trades` consumes the output regardless of which
//! source produced it.
//!
//! Two built-in sources:
//! - [`GridSignalSource`]: legacy string-match detection against Bar fields
//!   (preserves the existing 28-signal grid scan behavior).
//! - [`AnomalySignalSource`]: runs `forge_anomaly::AnomalyEngine` on the bars
//!   and converts each emitted `AnomalySignal` into one or two raw signals.
//!
//! The two families returned by [`AnomalySignalSource`] — `anomaly_reversal`
//! and `anomaly_momentum` — participate in the per-family verdict + DSR
//! aggregation just like any grid entry.

use crate::barrier::TripleBarrier;
use crate::trade::TradeResult;

use forge_anomaly::{
    AnomalyEngine, AnomalySignal, EngineConfig, SignalDirection, VolumeBar,
};

/// A single detected entry point, ready to be barrier-walked.
#[derive(Debug, Clone)]
pub struct RawSignal {
    pub bar_index: usize,
    pub is_long: bool,
    pub tp_bps: f64,
    pub sl_bps: f64,
    pub hold_bars: usize,
    /// Family label (e.g. `"oi_surge_long_25"`, `"anomaly_momentum"`).
    /// Used by the per-family verdict + DSR aggregation.
    pub family: String,
}

/// Anything that can scan a series of bars and produce raw signals.
///
/// The `Bar` type is the sweepscope bar (the lightweight struct from `main.rs`).
/// For grid sources this is the natural fit. For anomaly sources, the
/// implementation bridges to `forge_anomaly::VolumeBar` via the shared
/// `to_volume_bar()` constructor below.
pub trait SignalSource {
    fn name(&self) -> &str;
    fn collect_signals(&self, bars: &[crate::Bar]) -> Vec<RawSignal>;
}

// ── GridSignalSource: legacy string-match detection ───────────────────────────

/// Wraps the 28-entry macro grid + the giant string-match in `run_trades`.
/// This is what the tool ran before anomaly integration existed.
pub struct GridSignalSource {
    entries: Vec<(String, Vec<f64>)>,
    tp_bps: Vec<f64>,
    sl_bps: Vec<f64>,
    hold_bars: Vec<usize>,
}

impl GridSignalSource {
    #[must_use]
    pub fn from_grid() -> Self {
        Self {
            entries: crate::grid::SweepGrid::default().entries,
            tp_bps: crate::grid::SweepGrid::default().tp_bps,
            sl_bps: crate::grid::SweepGrid::default().sl_bps,
            hold_bars: crate::grid::SweepGrid::default().hold_bars,
        }
    }
}

impl SignalSource for GridSignalSource {
    fn name(&self) -> &str {
        "grid"
    }

    fn collect_signals(&self, bars: &[crate::Bar]) -> Vec<RawSignal> {
        let mut out = Vec::new();
        for (entry_name, thresholds) in &self.entries {
            for &threshold in thresholds {
                for &tp in &self.tp_bps {
                    for &sl in &self.sl_bps {
                        for &hold in &self.hold_bars {
                            if tp > 0.0 && sl > 0.0 && tp < sl * 0.3 {
                                continue;
                            }
                            let entries =
                                grid_entries(bars, entry_name, threshold, tp, sl, hold);
                            out.extend(entries);
                        }
                    }
                }
            }
        }
        out
    }
}

/// Old string-match entry detector, factored out of `run_trades`.
fn grid_entries(
    bars: &[crate::Bar],
    entry_name: &str,
    threshold: f64,
    tp_bps: f64,
    sl_bps: f64,
    hold_bars: usize,
) -> Vec<RawSignal> {
    let mut out = Vec::new();
    let mut i = 0;
    let mut null_ct = 0;
    while i < bars.len() {
        let entry_signal = match entry_name {
            "cvd_delta_long" => bars[i].cvd_delta < threshold,
            "cvd_delta_short" => bars[i].cvd_delta > threshold,
            "cvd_ratio_long" => bars[i].cvd_ratio < threshold,
            "cvd_ratio_short" => bars[i].cvd_ratio > threshold,
            "full_imbalance_long" => bars[i].full_imbalance > threshold,
            "full_imbalance_short" => bars[i].full_imbalance < threshold,
            "ask_skew_short" => bars[i].ask_depth_skew > threshold,
            "ask_skew_absorb" => bars[i].ask_depth_skew > threshold,
            "bid_skew_long" => bars[i].bid_depth_skew > threshold,
            "bid_skew_absorb" => bars[i].bid_depth_skew > threshold,
            "ask_conc_short" => bars[i].ask_conc_ratio > threshold,
            "bid_conc_long" => bars[i].bid_conc_ratio > threshold,
            "ask_breadth_long" => bars[i].depth_breadth_ask < threshold,
            "bid_breadth_short" => bars[i].depth_breadth_bid < threshold,
            "mean_ask_gap_short" => bars[i].mean_ask_gap_bps > threshold,
            "mean_bid_gap_long" => bars[i].mean_bid_gap_bps > threshold,
            "conc_high_short" => bars[i].concentration > threshold,
            "conc_low_long" => bars[i].concentration < threshold,
            "conc_low_short" => bars[i].concentration < threshold,
            "mid_poc_long" => bars[i].mid_to_poc_bps < threshold,
            "mid_poc_short" => bars[i].mid_to_poc_bps > threshold,
            "cvd_mom_long" => bars[i].cvd_momentum < threshold,
            "cvd_mom_short" => bars[i].cvd_momentum > threshold,
            "cvd_accel_long" => bars[i].cvd_acceleration < threshold,
            "cvd_accel_short" => bars[i].cvd_acceleration > threshold,
            "bid_wall_long" => bars[i].bid_wall_vol > threshold,
            "ask_wall_short" => bars[i].ask_wall_vol > threshold,
            "cross_ask_short" => bars[i].cross_ask_ratio > threshold,
            "funding_extreme_long" => {
                !bars[i].funding_rate.is_nan() && bars[i].funding_rate < threshold
            }
            "funding_extreme_short" => {
                !bars[i].funding_rate.is_nan() && bars[i].funding_rate > threshold
            }
            "mark_index_discount_long" => {
                !bars[i].mark_index_bps.is_nan() && bars[i].mark_index_bps < threshold
            }
            "mark_index_premium_short" => {
                !bars[i].mark_index_bps.is_nan() && bars[i].mark_index_bps > threshold
            }
            "oi_build_long" => {
                !bars[i].oi_pct_change.is_nan() && bars[i].oi_pct_change > threshold
            }
            "oi_unwind_short" => {
                !bars[i].oi_pct_change.is_nan() && bars[i].oi_pct_change < threshold
            }
            "liq_sell_short" => bars[i].liq_vol_sell > threshold,
            "liq_buy_long" => bars[i].liq_vol_buy > threshold,
            "liq_imb_long" => {
                !bars[i].liq_imbalance.is_nan() && bars[i].liq_imbalance > threshold
            }
            "liq_imb_short" => {
                !bars[i].liq_imbalance.is_nan() && bars[i].liq_imbalance < threshold
            }
            "basis_wide_short" => {
                !bars[i].basis_bps.is_nan() && bars[i].basis_bps > threshold
            }
            "basis_tight_long" => {
                !bars[i].basis_bps.is_nan() && bars[i].basis_bps < threshold
            }
            "liq_cascade_sell_25" => {
                !bars[i].liq_sell_cum_25.is_nan() && bars[i].liq_sell_cum_25 > threshold
            }
            "liq_cascade_buy_25" => {
                !bars[i].liq_buy_cum_25.is_nan() && bars[i].liq_buy_cum_25 > threshold
            }
            "liq_cascade_sell_50" => {
                !bars[i].liq_sell_cum_50.is_nan() && bars[i].liq_sell_cum_50 > threshold
            }
            "liq_cascade_buy_50" => {
                !bars[i].liq_buy_cum_50.is_nan() && bars[i].liq_buy_cum_50 > threshold
            }
            "liq_flow_sell_25" => {
                !bars[i].liq_flow_imb_25.is_nan() && bars[i].liq_flow_imb_25 < threshold
            }
            "liq_flow_buy_25" => {
                !bars[i].liq_flow_imb_25.is_nan() && bars[i].liq_flow_imb_25 > threshold
            }
            "liq_flow_sell_50" => {
                !bars[i].liq_flow_imb_50.is_nan() && bars[i].liq_flow_imb_50 < threshold
            }
            "liq_flow_buy_50" => {
                !bars[i].liq_flow_imb_50.is_nan() && bars[i].liq_flow_imb_50 > threshold
            }
            "oi_surge_long_25" => {
                !bars[i].oi_change_25.is_nan() && bars[i].oi_change_25 > threshold
            }
            "oi_surge_short_25" => {
                !bars[i].oi_change_25.is_nan() && bars[i].oi_change_25 < threshold
            }
            "oi_surge_long_50" => {
                !bars[i].oi_change_50.is_nan() && bars[i].oi_change_50 > threshold
            }
            "oi_surge_short_50" => {
                !bars[i].oi_change_50.is_nan() && bars[i].oi_change_50 < threshold
            }
            "oi_unwind_long" => {
                !bars[i].oi_pct_change.is_nan() && bars[i].oi_pct_change > threshold
            }
            "funding_crowd_short_25" => {
                !bars[i].funding_avg_25.is_nan() && bars[i].funding_avg_25 > threshold
            }
            "funding_crowd_long_25" => {
                !bars[i].funding_avg_25.is_nan() && bars[i].funding_avg_25 < threshold
            }
            "funding_crowd_short_50" => {
                !bars[i].funding_avg_50.is_nan() && bars[i].funding_avg_50 > threshold
            }
            "funding_crowd_long_50" => {
                !bars[i].funding_avg_50.is_nan() && bars[i].funding_avg_50 < threshold
            }
            "mi_premium_short_25" => {
                !bars[i].mark_index_avg_25.is_nan() && bars[i].mark_index_avg_25 > threshold
            }
            "mi_discount_long_25" => {
                !bars[i].mark_index_avg_25.is_nan() && bars[i].mark_index_avg_25 < threshold
            }
            "mi_premium_short_50" => {
                !bars[i].mark_index_avg_50.is_nan() && bars[i].mark_index_avg_50 > threshold
            }
            "mi_discount_long_50" => {
                !bars[i].mark_index_avg_50.is_nan() && bars[i].mark_index_avg_50 < threshold
            }
            "cvd_push_long_25" => {
                !bars[i].cvd_cum_25.is_nan() && bars[i].cvd_cum_25 > threshold
            }
            "cvd_push_short_25" => {
                !bars[i].cvd_cum_25.is_nan() && bars[i].cvd_cum_25 < threshold
            }
            "cvd_push_long_50" => {
                !bars[i].cvd_cum_50.is_nan() && bars[i].cvd_cum_50 > threshold
            }
            "cvd_push_short_50" => {
                !bars[i].cvd_cum_50.is_nan() && bars[i].cvd_cum_50 < threshold
            }
            "cvd_mom_cum_long_25" => {
                !bars[i].cvd_mom_cum_25.is_nan() && bars[i].cvd_mom_cum_25 > threshold
            }
            "cvd_mom_cum_short_25" => {
                !bars[i].cvd_mom_cum_25.is_nan() && bars[i].cvd_mom_cum_25 < threshold
            }
            "cvd_mom_cum_long_50" => {
                !bars[i].cvd_mom_cum_50.is_nan() && bars[i].cvd_mom_cum_50 > threshold
            }
            "cvd_mom_cum_short_50" => {
                !bars[i].cvd_mom_cum_50.is_nan() && bars[i].cvd_mom_cum_50 < threshold
            }
            "null_random" => i % threshold as usize == 0,
            _ => false,
        };

        if entry_signal {
            let is_long = if entry_name == "null_random" {
                null_ct % 2 == 0
            } else {
                entry_name.contains("_long")
                    || entry_name.contains("_buy")
                    || entry_name.contains("_absorb")
                    || entry_name.contains("_discount")
            };
            null_ct += 1;

            let entry_price = bars[i].mid_price;
            let barrier = TripleBarrier::find(bars, i, entry_price, is_long, tp_bps, sl_bps, hold_bars);
            let exit_idx = barrier.exit_idx;

            out.push(RawSignal {
                bar_index: i,
                is_long,
                tp_bps,
                sl_bps,
                hold_bars,
                family: entry_name.to_string(),
            });

            i = exit_idx + 1;
        } else {
            i += 1;
        }
    }
    out
}

// ── AnomalySignalSource: in-process forge_anomaly engine ─────────────────────

/// Runs `forge_anomaly::AnomalyEngine` over the bars and emits each `AnomalySignal`
/// as a raw entry. Direction comes from the signal. Family label encodes the
/// `SignalType` (Reversal vs MomentumContinuation) so per-family aggregation
/// in `sweepscope` produces two distinct families.
pub struct AnomalySignalSource {
    config: EngineConfig,
    default_tp_bps: f64,
    default_sl_bps: f64,
    default_hold_bars: usize,
    min_confidence: f64,
}

impl AnomalySignalSource {
    #[must_use]
    pub fn new(config: EngineConfig) -> Self {
        Self {
            config,
            default_tp_bps: 25.0,
            default_sl_bps: 15.0,
            default_hold_bars: 24,
            min_confidence: 0.0,
        }
    }

    #[must_use]
    pub fn with_params(
        config: EngineConfig,
        default_tp_bps: f64,
        default_sl_bps: f64,
        default_hold_bars: usize,
        min_confidence: f64,
    ) -> Self {
        Self {
            config,
            default_tp_bps,
            default_sl_bps,
            default_hold_bars,
            min_confidence,
        }
    }
}

impl SignalSource for AnomalySignalSource {
    fn name(&self) -> &str {
        "anomaly"
    }

    fn collect_signals(&self, bars: &[crate::Bar]) -> Vec<RawSignal> {
        let mut engine = AnomalyEngine::new(self.config.clone());
        let mut out = Vec::new();
        for (i, b) in bars.iter().enumerate() {
            let vb: VolumeBar = b.to_volume_bar();
            let output = engine.on_bar(&vb);
            if let Some(sig) = output.signal {
                if sig.confidence < self.min_confidence {
                    continue;
                }
                let (is_long, family) = match sig.direction {
                    SignalDirection::Long => (true, "anomaly_momentum".to_string()),
                    SignalDirection::Short => (false, "anomaly_momentum".to_string()),
                    SignalDirection::Neutral => continue,
                };
                let tp = if sig.expected_move_bps > 0.0 {
                    sig.expected_move_bps
                } else {
                    self.default_tp_bps
                };
                let hold = if sig.hold_bars > 0 {
                    sig.hold_bars as usize
                } else {
                    self.default_hold_bars
                };
                let family = match sig.signal_type {
                    forge_anomaly::SignalType::Reversal => {
                        let dir = if is_long { "_long" } else { "_short" };
                        format!("anomaly_reversal{dir}")
                    }
                    forge_anomaly::SignalType::MomentumContinuation => {
                        let dir = if is_long { "_long" } else { "_short" };
                        format!("anomaly_momentum{dir}")
                    }
                };
                out.push(RawSignal {
                    bar_index: i,
                    is_long,
                    tp_bps: tp,
                    sl_bps: self.default_sl_bps,
                    hold_bars: hold,
                    family,
                });
            }
        }
        out
    }
}

// ── Convert sweepscope Bar to forge_anomaly VolumeBar ─────────────────────────

impl crate::Bar {
    /// Project a sweepscope bar into the anomaly engine's input type.
    /// Fields not present in the sweepscope schema default to 0.
    #[must_use]
    pub fn to_volume_bar(&self) -> VolumeBar {
        let bar_vol = if self.bar_vol > 0.0 {
            self.bar_vol
        } else {
            // Fallback: derive from cum_vol; the engine only needs *some* nonzero volume.
            self.cum_vol.max(1.0)
        };
        VolumeBar {
            ts: self.ts.max(0) as u64,
            bar_index: self.bar_index,
            cum_vol: self.cum_vol,
            bar_vol,
            mid_price: self.mid_price,
            best_bid: self.best_bid,
            best_ask: self.best_ask,
            spread_bps: self.spread_bps,
            full_imbalance: self.full_imbalance,
            top5_imbalance: self.top5_imbalance,
            weighted_imbalance: self.weighted_imbalance,
            total_bid_vol: self.total_bid_vol,
            total_ask_vol: self.total_ask_vol,
            bid_vol_top5: self.bid_vol_top5,
            ask_vol_top5: self.ask_vol_top5,
            bid_vol_top10: self.bid_vol_top10,
            ask_vol_top10: self.ask_vol_top10,
            depth_breadth_bid: self.depth_breadth_bid,
            depth_breadth_ask: self.depth_breadth_ask,
            mean_bid_gap_bps: self.mean_bid_gap_bps,
            mean_ask_gap_bps: self.mean_ask_gap_bps,
            cvd_delta: self.cvd_delta,
            cvd_ratio: self.cvd_ratio,
            cvd_momentum: self.cvd_momentum,
            cvd_acceleration: self.cvd_acceleration,
            trade_count: self.trade_count,
            buy_count: self.buy_count,
            sell_count: self.sell_count,
            aggressor_ratio: self.aggressor_ratio,
            large_buy_count: self.large_buy_count,
            large_sell_count: self.large_sell_count,
            large_buy_vol: self.large_buy_vol,
            large_sell_vol: self.large_sell_vol,
            large_aggressor_ratio: self.large_aggressor_ratio,
            max_trade_size: self.max_trade_size,
            trade_intensity: self.trade_intensity,
            liq_imbalance: self.liq_imbalance,
            funding_rate: self.funding_rate,
            oi_pct_change: self.oi_pct_change,
        }
    }
}

// ── Barrier-walk helper for any signal source ─────────────────────────────────

/// Convert `(RawSignal, Bar) → Option<TradeResult>`.
pub fn barrier_walk(
    raw: &RawSignal,
    bars: &[crate::Bar],
    fee_bps: f64,
    position_eur: f64,
) -> Option<TradeResult> {
    let entry_price = bars[raw.bar_index].mid_price;
    let barrier = TripleBarrier::find(
        bars,
        raw.bar_index,
        entry_price,
        raw.is_long,
        raw.tp_bps,
        raw.sl_bps,
        raw.hold_bars,
    );
    let gross_pnl_bps = if raw.is_long {
        (barrier.exit_price - entry_price) / entry_price * 10000.0
    } else {
        (entry_price - barrier.exit_price) / entry_price * 10000.0
    };
    let net_pnl_bps = gross_pnl_bps - fee_bps;
    let eur_pnl = net_pnl_bps / 10000.0 * position_eur;

    Some(TradeResult {
        entry_ts: bars[raw.bar_index].ts,
        exit_ts: bars[barrier.exit_idx].ts,
        entry_idx: raw.bar_index,
        exit_idx: barrier.exit_idx,
        is_long: raw.is_long,
        entry_price,
        exit_price: barrier.exit_price,
        gross_pnl_bps,
        net_pnl_bps,
        eur_pnl,
        barrier_hit: barrier.barrier_hit,
    })
}

