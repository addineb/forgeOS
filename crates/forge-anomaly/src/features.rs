//! Feature extraction from consecutive volume bars.
//!
//! Computes OFI, CVD, depth imbalance, absorption, liquidity vacuum, and
//! volume-delta divergence. All features are scale-free for fractal use.

use crate::types::{BarFeatures, VolumeBar};

#[derive(Clone, Copy, Default)]
struct TopOfBook {
    bid_px: f64,
    bid_qty: f64,
    ask_px: f64,
    ask_qty: f64,
}

/// Extracts per-bar microstructure features from a stream of volume bars.
pub struct FeatureExtractor {
    depth_top_n: usize,
    ofi_normalized: bool,
    prev_bar: Option<VolumeBar>,
}

impl FeatureExtractor {
    #[must_use]
    pub fn new(depth_top_n: usize, ofi_normalized: bool) -> Self {
        Self {
            depth_top_n: depth_top_n.max(1),
            ofi_normalized,
            prev_bar: None,
        }
    }

    /// Process one bar. Returns `None` on the first bar (seeds state only).
    pub fn observe(&mut self, bar: &VolumeBar) -> Option<BarFeatures> {
        let prev = match self.prev_bar {
            Some(p) => p,
            None => {
                self.prev_bar = Some(*bar);
                return None;
            }
        };
        let features = self.compute(bar, &prev);
        self.prev_bar = Some(*bar);
        Some(features)
    }

    fn compute(&self, cur: &VolumeBar, prev: &VolumeBar) -> BarFeatures {
        let ofi = compute_ofi(cur, prev);
        let avg_depth = (cur.bid_vol_top5 + cur.ask_vol_top5) / 2.0;
        let ofi_normalized = if self.ofi_normalized && avg_depth > 0.0 {
            ofi / avg_depth
        } else {
            ofi
        };

        let vol_scale = if cur.bar_vol > 0.0 { cur.bar_vol } else { 1.0 };
        let cvd_delta = cur.cvd_delta / vol_scale;

        let depth_imbalance = top_n_imbalance(cur, self.depth_top_n);
        let mid_return_bps = if prev.mid_price > 0.0 {
            (cur.mid_price - prev.mid_price) / prev.mid_price * 10_000.0
        } else {
            0.0
        };

        let (bid_absorption, ask_absorption) = compute_absorption(cur, prev, mid_return_bps);
        let absorption = bid_absorption - ask_absorption;

        let (bid_vacuum, ask_vacuum) = compute_vacuum(cur, prev);
        let liquidity_vacuum = bid_vacuum.max(ask_vacuum);

        let vol_delta_divergence =
            compute_vol_delta_divergence(mid_return_bps, cur.cvd_delta, cur.cvd_momentum);

        let large_total = cur.large_buy_vol + cur.large_sell_vol;
        let large_print_imbalance = if large_total > 0.0 {
            (cur.large_buy_vol - cur.large_sell_vol) / large_total
        } else {
            0.0
        };

        BarFeatures {
            ofi,
            ofi_normalized,
            cvd_delta,
            cvd_momentum: cur.cvd_momentum / vol_scale,
            depth_imbalance,
            full_depth_imbalance: cur.full_imbalance,
            absorption,
            bid_absorption,
            ask_absorption,
            liquidity_vacuum,
            bid_vacuum,
            ask_vacuum,
            vol_delta_divergence,
            mid_return_bps,
            cvd_acceleration_normalized: cur.cvd_acceleration / vol_scale,
            aggressor_ratio: cur.aggressor_ratio,
            large_buy_vol: cur.large_buy_vol,
            large_sell_vol: cur.large_sell_vol,
            large_print_imbalance,
            trade_intensity: cur.trade_intensity,
            bar_index: cur.bar_index,
        }
    }
}

fn compute_ofi(cur: &VolumeBar, prev: &VolumeBar) -> f64 {
    let p = TopOfBook {
        bid_px: prev.best_bid,
        bid_qty: prev.bid_vol_top5,
        ask_px: prev.best_ask,
        ask_qty: prev.ask_vol_top5,
    };
    let c = TopOfBook {
        bid_px: cur.best_bid,
        bid_qty: cur.bid_vol_top5,
        ask_px: cur.best_ask,
        ask_qty: cur.ask_vol_top5,
    };
    cks_event(&p, &c)
}

fn cks_event(p: &TopOfBook, c: &TopOfBook) -> f64 {
    let mut e = 0.0;
    if c.bid_px >= p.bid_px {
        e += c.bid_qty;
    }
    if c.bid_px <= p.bid_px {
        e -= p.bid_qty;
    }
    if c.ask_px <= p.ask_px {
        e -= c.ask_qty;
    }
    if c.ask_px >= p.ask_px {
        e += p.ask_qty;
    }
    e
}

#[must_use]
pub fn top_n_imbalance(bar: &VolumeBar, n: usize) -> f64 {
    let (bid, ask) = if n >= 10 {
        (bar.bid_vol_top10, bar.ask_vol_top10)
    } else {
        (bar.bid_vol_top5, bar.ask_vol_top5)
    };
    let tot = bid + ask;
    if tot <= 0.0 {
        0.0
    } else {
        (bid - ask) / tot
    }
}

fn compute_absorption(cur: &VolumeBar, prev: &VolumeBar, mid_return_bps: f64) -> (f64, f64) {
    let sell_pressure = (-cur.cvd_delta).max(0.0);
    let buy_pressure = cur.cvd_delta.max(0.0);
    let bid_held = cur.best_bid >= prev.best_bid;
    let ask_held = cur.best_ask <= prev.best_ask;
    let vol_scale = if cur.bar_vol > 0.0 { cur.bar_vol } else { 1.0 };
    let scale = (1.0 + mid_return_bps.abs() / 100.0).min(3.0);

    let bid_absorption = if bid_held && sell_pressure > 0.0 {
        (sell_pressure / vol_scale) * scale
    } else {
        0.0
    };
    let ask_absorption = if ask_held && buy_pressure > 0.0 {
        (buy_pressure / vol_scale) * scale
    } else {
        0.0
    };
    (bid_absorption, ask_absorption)
}

fn compute_vacuum(cur: &VolumeBar, prev: &VolumeBar) -> (f64, f64) {
    let bid_depth_drop = relative_drop(prev.bid_vol_top10, cur.bid_vol_top10);
    let ask_depth_drop = relative_drop(prev.ask_vol_top10, cur.ask_vol_top10);
    let bid_gap_widen = relative_rise(prev.mean_bid_gap_bps, cur.mean_bid_gap_bps);
    let ask_gap_widen = relative_rise(prev.mean_ask_gap_bps, cur.mean_ask_gap_bps);
    let spread_widen = relative_rise(prev.spread_bps, cur.spread_bps);

    let bid_vacuum = bid_depth_drop * 0.5 + bid_gap_widen * 0.3 + spread_widen * 0.2;
    let ask_vacuum = ask_depth_drop * 0.5 + ask_gap_widen * 0.3 + spread_widen * 0.2;
    (bid_vacuum, ask_vacuum)
}

/// Volume-delta divergence: price and CVD disagree.
///
/// Bullish divergence: price falls while CVD rises (buying under the surface).
/// Bearish divergence: price rises while CVD falls.
fn compute_vol_delta_divergence(mid_return_bps: f64, cvd_delta: f64, cvd_momentum: f64) -> f64 {
    let price_sign = mid_return_bps.signum();
    let flow_sign = cvd_delta.signum();
    if price_sign == 0.0 || flow_sign == 0.0 || price_sign == flow_sign {
        return 0.0;
    }
    let mag = mid_return_bps.abs().max(cvd_delta.abs() / 10.0);
    let mom_boost = 1.0 + cvd_momentum.abs() / (cvd_delta.abs() + 1.0);
    mag * mom_boost
}

fn relative_drop(prev: f64, cur: f64) -> f64 {
    if prev <= 0.0 {
        0.0
    } else {
        ((prev - cur) / prev).max(0.0)
    }
}

fn relative_rise(prev: f64, cur: f64) -> f64 {
    if prev <= 0.0 {
        if cur > 0.0 { 1.0 } else { 0.0 }
    } else {
        ((cur - prev) / prev).max(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bar(idx: u64, bid: f64, ask: f64, bid_q: f64, ask_q: f64, cvd: f64) -> VolumeBar {
        VolumeBar {
            bar_index: idx,
            mid_price: (bid + ask) / 2.0,
            best_bid: bid,
            best_ask: ask,
            bid_vol_top5: bid_q,
            ask_vol_top5: ask_q,
            bid_vol_top10: bid_q * 1.5,
            ask_vol_top10: ask_q * 1.5,
            cvd_delta: cvd,
            bar_vol: 10.0,
            ..Default::default()
        }
    }

    #[test]
    fn divergence_when_price_and_cvd_disagree() {
        let mut fx = FeatureExtractor::new(5, true);
        let mut b0 = bar(0, 100.0, 101.0, 10.0, 10.0, 0.0);
        b0.mid_price = 100.5;
        let mut b1 = bar(1, 99.0, 100.0, 10.0, 10.0, 50.0);
        b1.mid_price = 99.5;
        fx.observe(&b0);
        let f = fx.observe(&b1).unwrap();
        assert!(f.vol_delta_divergence > 0.0);
    }
}