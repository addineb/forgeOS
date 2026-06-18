//! Load enriched depthscope volume-bar CSVs into [`VolumeBar`] records.
//!
//! Accepts both plain depthscope output and enriched files (extra columns are
//! ignored). Computes `bar_vol` from successive `cum_vol` deltas.

use std::io;
use std::path::Path;

use csv::ReaderBuilder;
use serde::Deserialize;

use crate::backtest::ForwardReturns;
use crate::types::VolumeBar;

fn deserialize_f64_nan<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    Option::<f64>::deserialize(d).map(|v| v.unwrap_or(0.0))
}

/// One CSV row from depthscope / enrich_depthscope output.
#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct DepthscopeRow {
    ts: u64,
    cum_vol: f64,
    full_imbalance: f64,
    top5_imbalance: f64,
    weighted_imbalance: f64,
    spread_bps: f64,
    #[serde(default)]
    total_bid_vol: f64,
    #[serde(default)]
    total_ask_vol: f64,
    #[serde(default)]
    mean_ask_gap_bps: f64,
    #[serde(default)]
    mean_bid_gap_bps: f64,
    cvd_delta: f64,
    #[serde(default)]
    cvd_ratio: f64,
    #[serde(default)]
    cvd_momentum: f64,
    #[serde(default)]
    cvd_acceleration: f64,
    #[serde(default)]
    depth_breadth_ask: f64,
    #[serde(default)]
    depth_breadth_bid: f64,
    #[serde(default)]
    bid_vol_top5: f64,
    #[serde(default)]
    ask_vol_top5: f64,
    #[serde(default)]
    bid_vol_top10: f64,
    #[serde(default)]
    ask_vol_top10: f64,
    mid_price: f64,
    best_bid: f64,
    best_ask: f64,
    // Forward returns and enriched columns — present in some files, ignored here.
    #[serde(default, deserialize_with = "deserialize_f64_nan")]
    fwd_ret_15m_bps: f64,
    #[serde(default, deserialize_with = "deserialize_f64_nan")]
    fwd_ret_1h_bps: f64,
    #[serde(default, deserialize_with = "deserialize_f64_nan")]
    fwd_ret_4h_bps: f64,
    #[serde(default, deserialize_with = "deserialize_f64_nan")]
    funding_rate: f64,
    #[serde(default, deserialize_with = "deserialize_f64_nan")]
    mark_index_bps: f64,
    #[serde(default, deserialize_with = "deserialize_f64_nan")]
    oi: f64,
    #[serde(default, deserialize_with = "deserialize_f64_nan")]
    oi_pct_change: f64,
    #[serde(default, deserialize_with = "deserialize_f64_nan")]
    liq_vol_buy: f64,
    #[serde(default, deserialize_with = "deserialize_f64_nan")]
    liq_vol_sell: f64,
    #[serde(default, deserialize_with = "deserialize_f64_nan")]
    liq_imbalance: f64,
    #[serde(default, deserialize_with = "deserialize_f64_nan")]
    basis_bps: f64,
    #[serde(default)]
    trade_count: u64,
    #[serde(default)]
    buy_count: u64,
    #[serde(default)]
    sell_count: u64,
    #[serde(default)]
    aggressor_ratio: f64,
    #[serde(default)]
    large_buy_count: u64,
    #[serde(default)]
    large_sell_count: u64,
    #[serde(default, deserialize_with = "deserialize_f64_nan")]
    large_buy_vol: f64,
    #[serde(default, deserialize_with = "deserialize_f64_nan")]
    large_sell_vol: f64,
    #[serde(default)]
    large_aggressor_ratio: f64,
    #[serde(default, deserialize_with = "deserialize_f64_nan")]
    max_trade_size: f64,
    #[serde(default, deserialize_with = "deserialize_f64_nan")]
    trade_intensity: f64,
}

impl From<DepthscopeRow> for VolumeBar {
    fn from(row: DepthscopeRow) -> Self {
        Self {
            ts: row.ts,
            bar_index: 0,
            cum_vol: row.cum_vol,
            bar_vol: 0.0,
            mid_price: row.mid_price,
            best_bid: row.best_bid,
            best_ask: row.best_ask,
            spread_bps: row.spread_bps,
            full_imbalance: row.full_imbalance,
            top5_imbalance: row.top5_imbalance,
            weighted_imbalance: row.weighted_imbalance,
            total_bid_vol: row.total_bid_vol,
            total_ask_vol: row.total_ask_vol,
            bid_vol_top5: row.bid_vol_top5,
            ask_vol_top5: row.ask_vol_top5,
            bid_vol_top10: row.bid_vol_top10,
            ask_vol_top10: row.ask_vol_top10,
            depth_breadth_bid: row.depth_breadth_bid,
            depth_breadth_ask: row.depth_breadth_ask,
            mean_bid_gap_bps: row.mean_bid_gap_bps,
            mean_ask_gap_bps: row.mean_ask_gap_bps,
            cvd_delta: row.cvd_delta,
            cvd_ratio: row.cvd_ratio,
            cvd_momentum: row.cvd_momentum,
            cvd_acceleration: row.cvd_acceleration,
            trade_count: row.trade_count,
            buy_count: row.buy_count,
            sell_count: row.sell_count,
            aggressor_ratio: row.aggressor_ratio,
            large_buy_count: row.large_buy_count,
            large_sell_count: row.large_sell_count,
            large_buy_vol: row.large_buy_vol,
            large_sell_vol: row.large_sell_vol,
            large_aggressor_ratio: row.large_aggressor_ratio,
            max_trade_size: row.max_trade_size,
            trade_intensity: row.trade_intensity,
            liq_imbalance: row.liq_imbalance,
            funding_rate: row.funding_rate,
            oi_pct_change: row.oi_pct_change,
        }
    }
}

/// Load volume bars from a depthscope CSV path.
///
/// Rows are sorted by `ts` if the file is out of order. `bar_index` and
/// `bar_vol` are assigned sequentially after load.
pub fn load_volume_bars(path: impl AsRef<Path>) -> io::Result<Vec<VolumeBar>> {
    let path = path.as_ref();
    let mut reader = ReaderBuilder::new().from_path(path)?;
    let mut bars: Vec<VolumeBar> = Vec::new();

    for result in reader.deserialize::<DepthscopeRow>() {
        let row: DepthscopeRow = result?;
        bars.push(row.into());
    }

    bars.sort_by_key(|b| b.ts);

    let mut prev_cum = 0.0;
    for (i, bar) in bars.iter_mut().enumerate() {
        bar.bar_index = i as u64;
        bar.bar_vol = if i == 0 {
            bar.cum_vol
        } else {
            (bar.cum_vol - prev_cum).max(0.0)
        };
        prev_cum = bar.cum_vol;
    }

    Ok(bars)
}

/// Load volume bars with forward returns from enriched depthscope CSV.
pub fn load_volume_bars_with_fwd(path: impl AsRef<Path>) -> io::Result<Vec<(VolumeBar, ForwardReturns)>> {
    let path = path.as_ref();
    let mut reader = ReaderBuilder::new().from_path(path)?;
    let mut rows: Vec<DepthscopeRow> = Vec::new();

    for result in reader.deserialize::<DepthscopeRow>() {
        rows.push(result?);
    }

    rows.sort_by_key(|r| r.ts);

    let mut prev_cum = 0.0;
    let mut pairs: Vec<(VolumeBar, ForwardReturns)> = Vec::with_capacity(rows.len());
    for (i, row) in rows.into_iter().enumerate() {
        let fwd = ForwardReturns {
            fwd_ret_15m_bps: row.fwd_ret_15m_bps,
            fwd_ret_1h_bps: row.fwd_ret_1h_bps,
            fwd_ret_4h_bps: row.fwd_ret_4h_bps,
        };
        let mut bar: VolumeBar = row.into();
        bar.bar_index = i as u64;
        bar.bar_vol = if i == 0 {
            bar.cum_vol
        } else {
            (bar.cum_vol - prev_cum).max(0.0)
        };
        prev_cum = bar.cum_vol;
        pairs.push((bar, fwd));
    }

    Ok(pairs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn loads_csv_and_assigns_bar_vol() {
        let dir = std::env::temp_dir().join("forge_anomaly_csv_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("sample.csv");
        let mut f = File::create(&path).unwrap();
        writeln!(
            f,
            "ts,cum_vol,full_imbalance,top5_imbalance,weighted_imbalance,spread_bps,\
             bid_levels,ask_levels,total_bid_vol,total_ask_vol,ask_concentration,bid_concentration,\
             best_ask_gap_bps,best_bid_gap_bps,mean_ask_gap_bps,mean_bid_gap_bps,\
             cvd_delta,cvd_ratio,cvd_count_imbalance,cvd_momentum,cvd_acceleration,\
             poc_price,va_high,va_low,concentration,mid_to_poc_bps,\
             active_wall_count,wall_cancel_ratio,avg_wall_lifetime_s,bid_wall_vol,ask_wall_vol,\
             ask_vol_top1,ask_vol_top3,ask_vol_top5,ask_vol_top10,ask_vol_top20,ask_vol_top50,ask_vol_top100,\
             bid_vol_top1,bid_vol_top3,bid_vol_top5,bid_vol_top10,bid_vol_top20,bid_vol_top50,bid_vol_top100,\
             ask_conc_ratio,bid_conc_ratio,ask_depth_skew,bid_depth_skew,cross_ask_ratio,depth_breadth_ask,depth_breadth_bid,\
             mid_price,best_bid,best_ask"
        ).unwrap();
        writeln!(
            f,
            "1000,10.0,0.1,0.1,0.1,1.0,50,50,100,100,0.5,0.5,1,1,1,1,\
             0,0.5,0,0,0,50000,50000,50000,0.5,0,0,0,0,0,0,\
             1,3,5,10,20,50,100,1,3,5,10,20,50,100,\
             0.1,0.1,0.5,0.5,1,0.5,0.5,50000,49999,50001"
        ).unwrap();
        writeln!(
            f,
            "2000,25.0,0.2,0.2,0.2,1.0,50,50,100,100,0.5,0.5,1,1,1,1,\
             5,0.6,0,1,0,50000,50000,50000,0.5,0,0,0,0,0,0,\
             1,3,5,10,20,50,100,1,3,5,10,20,50,100,\
             0.1,0.1,0.5,0.5,1,0.5,0.5,50001,50000,50002"
        ).unwrap();

        let bars = load_volume_bars(&path).unwrap();
        assert_eq!(bars.len(), 2);
        assert_eq!(bars[0].bar_vol, 10.0);
        assert_eq!(bars[1].bar_vol, 15.0);
        assert_eq!(bars[1].bar_index, 1);
    }
}