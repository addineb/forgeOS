//! Depth features: aggregated feature vector computed from depth snapshots over time.
//!
//! This is the main output of the depth-pattern study engine. It combines
//! depth shape, CVD, volume profile, and wall tracking into a single feature
//! vector that can be used for macrostructure analysis at 15min-4hr timescales.

use crate::depth::DepthSnapshot;
use crate::cvd::CVD;
use crate::volume_profile::VolumeProfile;
use crate::wall_tracker::WallTracker;

/// Aggregated depth features at a point in time, computed over a rolling window.
/// This is what the study tools will output for analysis.
#[derive(Debug, Clone)]
pub struct DepthFeatures {
    /// Timestamp of this feature snapshot (ns).
    pub ts: u64,
    // --- Depth shape ---
    /// Full-book imbalance (all levels).
    pub full_imbalance: f64,
    /// Top-5 imbalance (what the old studies used).
    pub top5_imbalance: f64,
    /// Weighted imbalance (decay=0.95, near levels weighted more).
    pub weighted_imbalance: f64,
    /// Spread in bps.
    pub spread_bps: f64,
    /// Number of bid levels.
    pub bid_levels: usize,
    /// Number of ask levels.
    pub ask_levels: usize,
    /// Total bid volume.
    pub total_bid_vol: f64,
    /// Total ask volume.
    pub total_ask_vol: f64,
    /// Ask-side depth concentration: volume in top-3 / total ask volume.
    pub ask_concentration: f64,
    /// Bid-side depth concentration: volume in top-3 / total bid volume.
    pub bid_concentration: f64,
    /// Inter-level gap at best (ask[1] - ask[0]) in bps.
    pub best_ask_gap_bps: f64,
    /// Inter-level gap at best (bid[0] - bid[1]) in bps.
    pub best_bid_gap_bps: f64,
    /// Mean inter-level gap on ask side (bps).
    pub mean_ask_gap_bps: f64,
    /// Mean inter-level gap on bid side (bps).
    pub mean_bid_gap_bps: f64,
    // --- CVD ---
    /// CVD delta over the window.
    pub cvd_delta: f64,
    /// CVD ratio over the window (0-1, 0.5 = balanced).
    pub cvd_ratio: f64,
    /// CVD count imbalance over the window.
    pub cvd_count_imbalance: f64,
    // --- Volume profile ---
    /// POC price (highest volume price).
    pub poc_price: f64,
    /// Value area high (70% volume boundary).
    pub va_high: f64,
    /// Value area low (70% volume boundary).
    pub va_low: f64,
    /// Volume concentration (0-1, 1 = all at POC).
    pub concentration: f64,
    /// Distance from current mid to POC in bps (positive = mid above POC).
    pub mid_to_poc_bps: f64,
    // --- Wall tracking ---
    /// Number of active walls (large resting orders).
    pub active_wall_count: usize,
    /// Cancel ratio: fraction of completed walls that were cancelled (not executed).
    pub wall_cancel_ratio: f64,
    /// Average wall lifetime in seconds.
    pub avg_wall_lifetime_s: f64,
    /// Total wall volume on bid side.
    pub bid_wall_vol: f64,
    /// Total wall volume on ask side.
    pub ask_wall_vol: f64,
}

impl DepthFeatures {
    /// Compute depth features from the current state.
    pub fn compute(
        snapshot: &DepthSnapshot,
        cvd: &CVD,
        vp: &VolumeProfile,
        wall_tracker: &WallTracker,
    ) -> Self {
        let mid = snapshot.mid;

        // Depth concentration: volume in top-3 levels / total
        let ask_concentration = if snapshot.total_ask_vol > 0.0 {
            snapshot.ask_vol_at_band(3) / snapshot.total_ask_vol
        } else { 0.0 };

        let bid_concentration = if snapshot.total_bid_vol > 0.0 {
            snapshot.bid_vol_at_band(3) / snapshot.total_bid_vol
        } else { 0.0 };

        // Inter-level gaps
        let best_ask_gap_bps = snapshot.ask_gap_bps(0).unwrap_or(0.0);
        let best_bid_gap_bps = snapshot.bid_gap_bps(0).unwrap_or(0.0);

        let mean_ask_gap_bps = if snapshot.ask_prices.len() > 1 {
            let n = snapshot.ask_prices.len() - 1;
            let sum: f64 = (0..n).filter_map(|i| snapshot.ask_gap_bps(i)).sum::<f64>();
            sum / n as f64
        } else { 0.0 };

        let mean_bid_gap_bps = if snapshot.bid_prices.len() > 1 {
            let n = snapshot.bid_prices.len() - 1;
            let sum: f64 = (0..n).filter_map(|i| snapshot.bid_gap_bps(i)).sum::<f64>();
            sum / n as f64
        } else { 0.0 };

        // Mid to POC distance in bps
        let mid_to_poc_bps = if mid > 0.0 && vp.poc_price > 0.0 {
            (mid - vp.poc_price) / mid * 10000.0
        } else { 0.0 };

        // Wall stats
        let bid_wall_vol: f64 = wall_tracker.active_walls.iter()
            .filter(|w| w.side == forge_core::Side::Bid)
            .map(|w| w.size)
            .sum();
        let ask_wall_vol: f64 = wall_tracker.active_walls.iter()
            .filter(|w| w.side == forge_core::Side::Ask)
            .map(|w| w.size)
            .sum();

        let avg_wall_lifetime_s = wall_tracker.avg_wall_lifetime_ns() / 1_000_000_000.0;

        Self {
            ts: snapshot.ts,
            full_imbalance: snapshot.full_imbalance,
            top5_imbalance: snapshot.top_n_imbalance,
            weighted_imbalance: snapshot.weighted_imbalance(0.95),
            spread_bps: snapshot.spread_bps,
            bid_levels: snapshot.bid_levels,
            ask_levels: snapshot.ask_levels,
            total_bid_vol: snapshot.total_bid_vol,
            total_ask_vol: snapshot.total_ask_vol,
            ask_concentration,
            bid_concentration,
            best_ask_gap_bps,
            best_bid_gap_bps,
            mean_ask_gap_bps,
            mean_bid_gap_bps,
            cvd_delta: cvd.delta(),
            cvd_ratio: cvd.ratio(),
            cvd_count_imbalance: cvd.count_imbalance(),
            poc_price: vp.poc_price,
            va_high: vp.value_area_high,
            va_low: vp.value_area_low,
            concentration: vp.concentration(),
            mid_to_poc_bps,
            active_wall_count: wall_tracker.active_walls.len(),
            wall_cancel_ratio: wall_tracker.cancel_ratio(),
            avg_wall_lifetime_s,
            bid_wall_vol,
            ask_wall_vol,
        }
    }

    /// Header row for CSV output.
    pub fn csv_header() -> &'static str {
        "ts,full_imbalance,top5_imbalance,weighted_imbalance,spread_bps,bid_levels,ask_levels,\
         total_bid_vol,total_ask_vol,ask_concentration,bid_concentration,\
         best_ask_gap_bps,best_bid_gap_bps,mean_ask_gap_bps,mean_bid_gap_bps,\
         cvd_delta,cvd_ratio,cvd_count_imbalance,\
         poc_price,va_high,va_low,concentration,mid_to_poc_bps,\
         active_wall_count,wall_cancel_ratio,avg_wall_lifetime_s,bid_wall_vol,ask_wall_vol"
    }

    /// CSV row for this feature snapshot.
    pub fn to_csv_row(&self) -> String {
        format!(
            "{},{:.6},{:.6},{:.6},{:.2},{},{},{:.4},{:.4},{:.4},{:.4},{:.2},{:.2},{:.2},{:.2},\
             {:.4},{:.4},{:.4},{:.2},{:.2},{:.2},{:.2},{:.2},{},{:.4},{:.2},{:.4},{:.4}",
            self.ts,
            self.full_imbalance, self.top5_imbalance, self.weighted_imbalance,
            self.spread_bps, self.bid_levels, self.ask_levels,
            self.total_bid_vol, self.total_ask_vol,
            self.ask_concentration, self.bid_concentration,
            self.best_ask_gap_bps, self.best_bid_gap_bps,
            self.mean_ask_gap_bps, self.mean_bid_gap_bps,
            self.cvd_delta, self.cvd_ratio, self.cvd_count_imbalance,
            self.poc_price, self.va_high, self.va_low,
            self.concentration, self.mid_to_poc_bps,
            self.active_wall_count, self.wall_cancel_ratio,
            self.avg_wall_lifetime_s, self.bid_wall_vol, self.ask_wall_vol,
        )
    }
}