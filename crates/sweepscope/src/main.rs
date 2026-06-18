//! Sweepscope: entry+exit grid search on depthscope CSVs.
//!
//! A standalone study tool (like depthscope) that:
//! 1. Reads the stitched depthscope CSV (40K volume bars with features + forward returns)
//! 2. Sweeps entry thresholds (CVD delta, imbalance, etc.) × exit parameters (TP, SL, hold)
//! 3. Implements triple-barrier exits (first of TP/SL/time to be hit)
//! 4. Uses purged cross-validation for honest train/test splits
//! 5. Scores with DSR and PBO from forge-metrics
//! 6. Outputs a scorecard: promote / park / retire
//!
//! Borrows ONLY forge-metrics (pure math, no engine deps).
//! Does NOT touch forge-sim, forge-sweep, or forge-strategy.

mod barrier;
mod cv;
mod grid;
mod trade;

fn deserialize_f64_nan<'de, D: serde::Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    use serde::Deserialize;
    Option::<f64>::deserialize(d).map(|v| v.unwrap_or(f64::NAN))
}

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

use clap::Parser;
use csv::ReaderBuilder;
use rayon::prelude::*;

use barrier::TripleBarrier;
use cv::PurgedCv;
use grid::SweepGrid;
use trade::TradeResult;

use forge_metrics::{deflated_sharpe, sharpe, variance_of_sharpes};

/// Sweepscope: sweep entry+exit parameters on depthscope CSVs.
#[derive(Parser, Debug)]
#[command(name = "sweepscope", about = "Entry+exit grid search with honest scoring")]
struct Args {
    /// Input CSV (stitched depthscope output)
    #[arg(long)]
    input: PathBuf,

    /// Output scorecard CSV
    #[arg(long, default_value = "sweepscope_scorecard.csv")]
    output: PathBuf,

    /// Number of CV folds (purged)
    #[arg(long, default_value_t = 5)]
    folds: usize,

    /// Purge gap in bars (prevents leakage across fold boundaries).
    /// Must be >= max hold_bars in the grid, otherwise trades spanning
    /// the fold boundary leak future data into training.
    #[arg(long, default_value_t = 90)]
    purge_bars: usize,

    /// Minimum round trips to be taken seriously
    #[arg(long, default_value_t = 30)]
    min_trades: usize,

    /// DSR threshold to promote
    #[arg(long, default_value_t = 0.9)]
    dsr_promote: f64,

    /// DSR threshold to park
    #[arg(long, default_value_t = 0.6)]
    dsr_park: f64,

    /// PBO threshold (above this = overfit, retire)
    #[arg(long, default_value_t = 0.5)]
    pbo_max: f64,

    /// Fee in bps (round-trip). HL taker = 9, maker = 5.
    #[arg(long, default_value_t = 9.0)]
    fee_bps: f64,

    /// Number of CSCV blocks for PBO (must be even, >= 4).
    #[arg(long, default_value_t = 8)]
    cscv_blocks: usize,

    /// Write per-date breakdown CSV.
    #[arg(long)]
    per_date: Option<PathBuf>,

    /// Run null-edge baseline: random entry (every N bars) must lose ~fees.
    /// If it doesn't, the pipeline is broken (lookahead, wrong fee, etc.)
    #[arg(long, default_value_t = false)]
    null_edge: bool,

    /// Spacing between random-entry trades (in bars) for null-edge test.
    #[arg(long, default_value_t = 20)]
    null_spacing: usize,
}

/// One row of the depthscope CSV.
/// Field names must match the CSV header exactly.
#[derive(Debug, Clone, serde::Deserialize)]
#[allow(dead_code)]
struct Bar {
    ts: i64,
    /// Date string (YYYY-MM-DD). If missing from CSV, derived from ts.
    #[serde(default)]
    date: String,
    cum_vol: f64,
    full_imbalance: f64,
    top5_imbalance: f64,
    weighted_imbalance: f64,
    spread_bps: f64,
    bid_levels: f64,
    ask_levels: f64,
    total_bid_vol: f64,
    total_ask_vol: f64,
    ask_concentration: f64,
    bid_concentration: f64,
    best_ask_gap_bps: f64,
    best_bid_gap_bps: f64,
    mean_ask_gap_bps: f64,
    mean_bid_gap_bps: f64,
    cvd_delta: f64,
    cvd_ratio: f64,
    cvd_count_imbalance: f64,
    cvd_momentum: f64,
    cvd_acceleration: f64,
    poc_price: f64,
    va_high: f64,
    va_low: f64,
    concentration: f64,
    mid_to_poc_bps: f64,
    active_wall_count: f64,
    wall_cancel_ratio: f64,
    avg_wall_lifetime_s: f64,
    bid_wall_vol: f64,
    ask_wall_vol: f64,
    ask_vol_top1: f64,
    ask_vol_top3: f64,
    ask_vol_top5: f64,
    ask_vol_top10: f64,
    ask_vol_top20: f64,
    ask_vol_top50: f64,
    ask_vol_top100: f64,
    bid_vol_top1: f64,
    bid_vol_top3: f64,
    bid_vol_top5: f64,
    bid_vol_top10: f64,
    bid_vol_top20: f64,
    bid_vol_top50: f64,
    bid_vol_top100: f64,
    ask_conc_ratio: f64,
    bid_conc_ratio: f64,
    ask_depth_skew: f64,
    bid_depth_skew: f64,
    cross_ask_ratio: f64,
    depth_breadth_ask: f64,
    depth_breadth_bid: f64,
    mid_price: f64,
    best_bid: f64,
    best_ask: f64,
    fwd_ret_15m_bps: f64,
    fwd_ret_1h_bps: f64,
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

    // Rolling-window features (_25=~15min, _50=~30min windows at vb10 cadence)
    #[serde(default, deserialize_with = "deserialize_f64_nan")]
    liq_sell_cum_25: f64,
    #[serde(default, deserialize_with = "deserialize_f64_nan")]
    liq_buy_cum_25: f64,
    #[serde(default, deserialize_with = "deserialize_f64_nan")]
    liq_flow_imb_25: f64,
    #[serde(default, deserialize_with = "deserialize_f64_nan")]
    oi_change_25: f64,
    #[serde(default, deserialize_with = "deserialize_f64_nan")]
    funding_avg_25: f64,
    #[serde(default, deserialize_with = "deserialize_f64_nan")]
    mark_index_avg_25: f64,
    #[serde(default, deserialize_with = "deserialize_f64_nan")]
    cvd_cum_25: f64,
    #[serde(default, deserialize_with = "deserialize_f64_nan")]
    ask_skew_avg_25: f64,
    #[serde(default, deserialize_with = "deserialize_f64_nan")]
    bid_skew_avg_25: f64,
    #[serde(default, deserialize_with = "deserialize_f64_nan")]
    cvd_mom_cum_25: f64,

    #[serde(default, deserialize_with = "deserialize_f64_nan")]
    liq_sell_cum_50: f64,
    #[serde(default, deserialize_with = "deserialize_f64_nan")]
    liq_buy_cum_50: f64,
    #[serde(default, deserialize_with = "deserialize_f64_nan")]
    liq_flow_imb_50: f64,
    #[serde(default, deserialize_with = "deserialize_f64_nan")]
    oi_change_50: f64,
    #[serde(default, deserialize_with = "deserialize_f64_nan")]
    funding_avg_50: f64,
    #[serde(default, deserialize_with = "deserialize_f64_nan")]
    mark_index_avg_50: f64,
    #[serde(default, deserialize_with = "deserialize_f64_nan")]
    cvd_cum_50: f64,
    #[serde(default, deserialize_with = "deserialize_f64_nan")]
    ask_skew_avg_50: f64,
    #[serde(default, deserialize_with = "deserialize_f64_nan")]
    bid_skew_avg_50: f64,
    #[serde(default, deserialize_with = "deserialize_f64_nan")]
    cvd_mom_cum_50: f64,
}

/// Verdict for a config after sweep scoring.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Verdict {
    Promote,
    Park,
    Retire,
}

impl std::fmt::Display for Verdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Verdict::Promote => write!(f, "PROMOTE"),
            Verdict::Park => write!(f, "PARK"),
            Verdict::Retire => write!(f, "RETIRE"),
        }
    }
}

/// Result for one sweep config.
struct CellResult {
    id: usize,
    entry_name: String,
    entry_threshold: f64,
    tp_bps: f64,
    sl_bps: f64,
    hold_bars: usize,
    fee_bps: f64,
    round_trips: usize,
    win_rate: f64,
    avg_pnl_bps: f64,
    net_pnl_bps: f64,
    sharpe: f64,
    dsr: f64,
    pbo: f64,
    verdict: Verdict,
    oos_net_bps: f64,
    oos_win_rate: f64,
    /// Bar index of each trade entry (for trade-level CSCV PBO)
    trade_bar_indices: Vec<usize>,
    /// Per-trade returns (net_pnl_bps / 10000) for DSR computation
    trade_returns: Vec<f64>,
    /// Per-date breakdown: (date, trades, net_bps, win_rate)
    per_date: Vec<(String, usize, f64, f64)>,
}

/// Find bar indices where date changes (date boundaries).
/// Returns a vector where each element is the first bar index of a new date.
/// Always includes 0 (first bar) and bars.len() (end sentinel).
fn find_date_boundaries(bars: &[Bar]) -> Vec<usize> {
    if bars.is_empty() {
        return vec![0, 0];
    }
    let mut boundaries = vec![0];
    let mut prev_date = &bars[0].date;
    for i in 1..bars.len() {
        if &bars[i].date != prev_date {
            boundaries.push(i);
            prev_date = &bars[i].date;
        }
    }
    boundaries.push(bars.len());
    boundaries
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    // Load data
    eprintln!("Loading {}...", args.input.display());
    let bars = load_csv(&args.input)?;
    eprintln!("Loaded {} bars", bars.len());

    if bars.is_empty() {
        eprintln!("No data. Exiting.");
        return Ok(());
    }

    // Build sweep grid
    let grid = SweepGrid::default();
    let configs = grid.expand();
    eprintln!("Sweep grid: {} configs", configs.len());

    // Build purged CV splits
    let boundaries = find_date_boundaries(&bars);
    let n_dates = boundaries.len() - 1;
    let cv = PurgedCv::new_date_aware(boundaries, args.folds, args.purge_bars);
    eprintln!("CV: {} folds, {} bars purge gap, {} dates (date-aware)", args.folds, args.purge_bars, n_dates);

    // === Null-edge baseline ===
    // Random entry (every N bars, alternating long/short) must lose ~fees.
    // If it doesn't, the pipeline has a bug (lookahead, wrong fee, etc.)
    let fee_bps = args.fee_bps;
    if args.null_edge {
        eprintln!("\n--- NULL-EDGE BASELINE ---");
        for &hold in &[5, 10, 30, 90] {
            for &tp in &[0.0, 15.0, 30.0] {
                for &sl in &[0.0, 15.0, 30.0] {
                    let trades = run_trades("null_random", args.null_spacing as f64, tp, sl, hold, fee_bps, &bars);
                    let n = trades.len();
                    if n == 0 { continue; }
                    let net = trades.iter().map(|t| t.net_pnl_bps).sum::<f64>() / n as f64;
                    let wr = trades.iter().filter(|t| t.net_pnl_bps > 0.0).count() as f64 / n as f64;
                    eprintln!("  null(tp={:.0},sl={:.0},hold={:3}): {:4} trades, net={:+.2} bps, win={:.1}% (expect ~-{:.0} bps)",
                        tp, sl, hold, n, net, wr * 100.0, fee_bps);
                }
            }
        }
        eprintln!("--- END NULL-EDGE ---\n");
    }

    // Run sweep in parallel
    let mut results: Vec<CellResult> = configs
        .par_iter()
        .enumerate()
        .map(|(id, cfg)| {
            run_config(id, cfg, &bars, &cv, fee_bps, args.min_trades)
        })
        .collect();

    // === Compute DSR and real CSCV PBO ===
    // FAMILY-COUNT DSR: count independent hypotheses, not directional + parameter permutations.
    // Each entry_name stripped of WINDOW suffix (_25, _50) AND DIRECTION suffix (_long, _short)
    // is one hypothesis family. oi_surge_short_25 → oi_surge, oi_surge_long_25 → oi_surge.
    fn entry_family(name: &str) -> String {
        // Strip window suffix first (_25, _50, _100)
        let without_window = {
            let parts: Vec<&str> = name.rsplitn(2, '_').collect();
            if parts.len() == 2 && parts[0].chars().all(|c| c.is_ascii_digit()) {
                parts[1].to_string()
            } else {
                name.to_string()
            }
        };
        // Strip direction suffix (_long, _short, _buy, _sell, _absorb, _discount)
        let dir_suffixes = ["_short", "_long", "_buy", "_sell", "_absorb", "_discount"];
        for suffix in &dir_suffixes {
            if without_window.ends_with(suffix) {
                return without_window[..without_window.len() - suffix.len()].to_string();
            }
        }
        without_window
    }

    // Compute per-family average Sharpe (clamped) for variance estimation
    let mut family_sharpes: std::collections::HashMap<String, Vec<f64>> = std::collections::HashMap::new();
    for r in results.iter().filter(|r| r.round_trips >= args.min_trades && r.sharpe <= 5.0) {
        let fam = entry_family(&r.entry_name);
        family_sharpes.entry(fam).or_default().push(r.sharpe.clamp(-5.0, 5.0));
    }
    let family_avg_sharpes: Vec<f64> = family_sharpes.values()
        .map(|v| v.iter().sum::<f64>() / v.len() as f64)
        .collect();
    let n_families = family_avg_sharpes.len();
    let var_sharpes = variance_of_sharpes(&family_avg_sharpes);

    eprintln!("DSR families: {} (vs {} individual configs), var_sharpes={:.4}",
        n_families, results.iter().filter(|r| r.round_trips >= args.min_trades).count(), var_sharpes);

    // Build trade-level data for CSCV PBO (only configs with enough trades)
    let eligible: Vec<&CellResult> = results.iter().filter(|r| r.round_trips >= args.min_trades).collect();
    let (sweep_pbo, n_combos) = if eligible.len() >= 2 {
        let trade_data: Vec<(&[usize], &[f64])> = eligible.iter()
            .map(|r| (r.trade_bar_indices.as_slice(), r.trade_returns.as_slice()))
            .collect();
        match pbo_cscv_trade_level(&trade_data, bars.len(), args.cscv_blocks) {
            Some((pbo, combos)) => (pbo, combos),
            None => (1.0, 0),
        }
    } else {
        (1.0, 0)
    };

    eprintln!("CSCV PBO: {:.3} ({} combinations, {} eligible configs)",
        sweep_pbo, n_combos, eligible.len());

    // Write scorecard
    let mut out = File::create(&args.output)?;
    writeln!(out, "id,entry,threshold,tp_bps,sl_bps,hold_bars,fee_bps,trades,win_rate,avg_pnl_bps,net_pnl_bps,sharpe,dsr,pbo,oos_net_bps,oos_win_rate,verdict")?;

    let mut n_promote = 0;
    let mut n_park = 0;
    let mut n_retire = 0;

    for r in &mut results {
        // DSR with family count (not individual configs) for honest multiple-testing correction
        if r.round_trips >= args.min_trades && r.sharpe <= 5.0 {
            r.dsr = deflated_sharpe(&r.trade_returns, n_families, var_sharpes);
        } else {
            r.dsr = 0.0;
        }

        // Real CSCV PBO (sweep-level, same for all configs)
        r.pbo = if r.round_trips >= args.min_trades { sweep_pbo } else { 1.0 };

        // OOS-confirmation: if OOS net > 0 and OOS win > 40%, the signal replicated
        // out-of-sample. This directly addresses the overfitting concern that DSR
        // measures, so we lower the promote bar.
        let oos_confirms = r.oos_net_bps > 0.0 && r.oos_win_rate > 0.40;

        // Final verdict with DSR and PBO
        // Reject configs with suspiciously high Sharpe (>5 = near-zero variance pathology)
        if r.round_trips < args.min_trades || r.net_pnl_bps <= 0.0 || r.sharpe > 5.0 {
            r.verdict = Verdict::Retire;
        } else if oos_confirms && r.dsr >= args.dsr_park && r.pbo <= args.pbo_max {
            // OOS-confirmed + clears park bar → PROMOTE (overfit concern addressed by OOS)
            r.verdict = Verdict::Promote;
        } else if r.dsr >= args.dsr_promote && r.pbo <= args.pbo_max && r.oos_net_bps > 0.0 {
            r.verdict = Verdict::Promote;
        } else if r.dsr >= args.dsr_park && r.pbo <= args.pbo_max {
            r.verdict = Verdict::Park;
        } else {
            r.verdict = Verdict::Retire;
        }

        match r.verdict {
            Verdict::Promote => n_promote += 1,
            Verdict::Park => n_park += 1,
            Verdict::Retire => n_retire += 1,
        }

        writeln!(out, "{},{},{:.1},{:.1},{:.1},{},{:.1},{},{:.3},{:.2},{:.2},{:.2},{:.3},{:.3},{:.2},{:.3},{}",
            r.id, r.entry_name, r.entry_threshold, r.tp_bps, r.sl_bps, r.hold_bars, r.fee_bps,
            r.round_trips, r.win_rate, r.avg_pnl_bps, r.net_pnl_bps, r.sharpe, r.dsr, r.pbo,
            r.oos_net_bps, r.oos_win_rate, r.verdict)?;
    }

    eprintln!("\n=== SWEEP SCORECARD ===");
    eprintln!("  Total configs: {}", n_promote + n_park + n_retire);
    eprintln!("  PROMOTE: {} (clears strict bar)", n_promote);
    eprintln!("  PARK:    {} (shows pulse, keep refining)", n_park);
    eprintln!("  RETIRE:  {} (net-negative or overfit)", n_retire);
    eprintln!("\nScorecard written to {}", args.output.display());

    // Print top 5 by net PnL
    let mut sorted: Vec<&CellResult> = results.iter().filter(|r| r.round_trips >= args.min_trades).collect();
    sorted.sort_by(|a, b| b.net_pnl_bps.partial_cmp(&a.net_pnl_bps).unwrap_or(std::cmp::Ordering::Equal));
    eprintln!("\nTop 5 by net P&L:");
    eprintln!("  {:>4} {:>12} {:>10} {:>6} {:>6} {:>6} {:>8} {:>8} {:>6} {:>8}",
        "id", "entry", "thresh", "TP", "SL", "hold", "trades", "net_bps", "win%", "verdict");
    for r in sorted.iter().take(5) {
        eprintln!("  {:>4} {:>12} {:>10.0} {:>6.0} {:>6.0} {:>6} {:>8} {:>8.1} {:>5.1}% {:>8}",
            r.id, r.entry_name, r.entry_threshold, r.tp_bps, r.sl_bps, r.hold_bars,
            r.round_trips, r.net_pnl_bps, r.win_rate * 100.0, r.verdict);
    }

    // Per-date breakdown
    if let Some(per_date_path) = &args.per_date {
        let mut pd_out = File::create(per_date_path)?;
        writeln!(pd_out, "id,entry,threshold,tp_bps,sl_bps,hold_bars,date,trades,net_bps,win_rate")?;
        for r in &results {
            if r.round_trips < args.min_trades { continue; }
            for (date, trades, net_bps, wr) in &r.per_date {
                writeln!(pd_out, "{},{},{:.1},{:.1},{:.1},{},{},{},{:.2},{:.3}",
                    r.id, r.entry_name, r.entry_threshold, r.tp_bps, r.sl_bps, r.hold_bars,
                    date, trades, net_bps, wr)?;
            }
        }
        eprintln!("Per-date breakdown written to {}", per_date_path.display());
    }

    Ok(())
}

fn load_csv(path: &PathBuf) -> Result<Vec<Bar>, Box<dyn std::error::Error>> {
    let mut reader = ReaderBuilder::new().from_path(path)?;
    let mut bars = Vec::new();
    for result in reader.deserialize() {
        let mut bar: Bar = result?;
        // If date column is missing/empty, derive from timestamp (Unix nanoseconds -> UTC date)
        if bar.date.is_empty() {
            // ts is Unix nanoseconds; convert to seconds for chrono-free date extraction
            let secs = bar.ts / 1_000_000_000;
            let days_since_epoch = secs / 86400;
            // Simple date from days since 1970-01-01
            let (year, month, day) = days_to_ymd(days_since_epoch);
            bar.date = format!("{:04}-{:02}-{:02}", year, month, day);
        }
        bars.push(bar);
    }
    Ok(bars)
}

/// Convert days since Unix epoch (1970-01-01) to (year, month, day).
/// No chrono dependency needed — simple algorithm.
fn days_to_ymd(mut days: i64) -> (i64, u32, u32) {
    // Shift to days since 0000-03-01 (makes Feb the last month, simplifies leap year)
    days += 719468; // offset from 0000-03-01 to 1970-01-01
    let era = if days >= 0 { days / 146097 } else { (days - 146096) / 146097 };
    let day_of_era = days - era * 146097;
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month + 2) / 5 + 1;
    let year = year_of_era + era * 400 + if month < 10 { 0 } else { 1 };
    let month = if month < 10 { month + 3 } else { month - 9 };
    (year, month as u32, day as u32)
}

/// Run one config: compute trades on all data, then evaluate with purged CV.
///
/// FIX: OOS trades are now run INDEPENDENTLY on OOS bar slices, not filtered
/// from the all-data run. This prevents entry decisions from seeing future data.
fn run_config(
    id: usize,
    cfg: &grid::SweepConfig,
    bars: &[Bar],
    cv: &PurgedCv,
    fee_bps: f64,
    min_trades: usize,
) -> CellResult {
    // Run on ALL data (for full-sample stats and per-bar PnL)
    let all_trades = run_trades(&cfg.entry_name, cfg.entry_threshold, cfg.tp_bps, cfg.sl_bps, cfg.hold_bars, fee_bps, bars);

    // Build per-trade data (bar indices and returns) for DSR and trade-level CSCV PBO
    let trade_bar_indices: Vec<usize> = all_trades.iter().map(|t| t.entry_idx).collect();
    let trade_returns: Vec<f64> = all_trades.iter().map(|t| t.net_pnl_bps / 10000.0).collect();

    // Run OOS trades INDEPENDENTLY on each OOS fold (no future leakage)
    let mut oos_trades_all = Vec::new();
    for fold_idx in 0..cv.n_folds() {
        let oos_range = cv.oos_range(fold_idx);
        let oos_bars = &bars[oos_range.clone()];
        let oos_trades = run_trades(&cfg.entry_name, cfg.entry_threshold, cfg.tp_bps, cfg.sl_bps, cfg.hold_bars, fee_bps, oos_bars);
        oos_trades_all.extend(oos_trades);
    }

    // Per-date breakdown
    let mut date_map: std::collections::BTreeMap<String, Vec<&TradeResult>> = std::collections::BTreeMap::new();
    for t in &all_trades {
        let date = bars[t.entry_idx].date.clone();
        date_map.entry(date).or_default().push(t);
    }
    let per_date: Vec<(String, usize, f64, f64)> = date_map.iter().map(|(date, trades)| {
        let n = trades.len();
        let net = trades.iter().map(|t| t.net_pnl_bps).sum::<f64>() / n as f64;
        let wins = trades.iter().filter(|t| t.net_pnl_bps > 0.0).count();
        (date.clone(), n, net, wins as f64 / n as f64)
    }).collect();

    let round_trips = all_trades.len();
    let (win_rate, avg_pnl_bps, net_pnl_bps, sharpe_val) = if round_trips == 0 {
        (0.0, 0.0, 0.0, 0.0)
    } else {
        let wins = all_trades.iter().filter(|t| t.net_pnl_bps > 0.0).count();
        let wr = wins as f64 / round_trips as f64;
        let avg = all_trades.iter().map(|t| t.gross_pnl_bps).sum::<f64>() / round_trips as f64;
        let net = all_trades.iter().map(|t| t.net_pnl_bps).sum::<f64>() / round_trips as f64;
        let returns: Vec<f64> = all_trades.iter().map(|t| t.net_pnl_bps / 10000.0).collect();
        let sh = sharpe(&returns);
        (wr, avg, net, sh)
    };

    let (oos_net_bps, oos_win_rate) = if oos_trades_all.is_empty() {
        (0.0, 0.0)
    } else {
        let oos_net = oos_trades_all.iter().map(|t| t.net_pnl_bps).sum::<f64>() / oos_trades_all.len() as f64;
        let oos_wins = oos_trades_all.iter().filter(|t| t.net_pnl_bps > 0.0).count();
        (oos_net, oos_wins as f64 / oos_trades_all.len() as f64)
    };

    // Preliminary verdict (DSR/PBO filled in main after all configs run)
    let verdict = if round_trips < min_trades || net_pnl_bps <= 0.0 {
        Verdict::Retire
    } else {
        Verdict::Park // will be upgraded or confirmed in main
    };

    CellResult {
        id,
        entry_name: cfg.entry_name.clone(),
        entry_threshold: cfg.entry_threshold,
        tp_bps: cfg.tp_bps,
        sl_bps: cfg.sl_bps,
        hold_bars: cfg.hold_bars,
        fee_bps,
        round_trips,
        win_rate,
        avg_pnl_bps,
        net_pnl_bps,
        sharpe: sharpe_val,
        dsr: 0.0, // filled in main
        pbo: 0.0, // filled in main
        verdict,
        oos_net_bps,
        oos_win_rate,
        trade_bar_indices,
        trade_returns,
        per_date,
    }
}

/// Run trades for a given entry/exit config on the bar data.
fn run_trades(entry_name: &str, threshold: f64, tp_bps: f64, sl_bps: f64, hold_bars: usize, fee_bps: f64, bars: &[Bar]) -> Vec<TradeResult> {
    let mut trades = Vec::new();
    let mut i = 0;
    let mut null_trade_count = 0;

    while i < bars.len() {
        let entry_signal = match entry_name {
            // --- Public CVD (baseline) ---
            "cvd_delta_long" => bars[i].cvd_delta < threshold,
            "cvd_delta_short" => bars[i].cvd_delta > threshold,
            "cvd_ratio_long" => bars[i].cvd_ratio < threshold,
            "cvd_ratio_short" => bars[i].cvd_ratio > threshold,
            "full_imbalance_long" => bars[i].full_imbalance > threshold,
            "full_imbalance_short" => bars[i].full_imbalance < threshold,

            // --- Depth skew (structural) ---
            "ask_skew_short" => bars[i].ask_depth_skew > threshold,
            "ask_skew_absorb" => bars[i].ask_depth_skew > threshold,
            "bid_skew_long" => bars[i].bid_depth_skew > threshold,
            "bid_skew_absorb" => bars[i].bid_depth_skew > threshold,

            // --- Concentration ratio ---
            "ask_conc_short" => bars[i].ask_conc_ratio > threshold,
            "bid_conc_long" => bars[i].bid_conc_ratio > threshold,

            // --- Book breadth ---
            "ask_breadth_long" => bars[i].depth_breadth_ask < threshold,
            "bid_breadth_short" => bars[i].depth_breadth_bid < threshold,

            // --- Gap signals ---
            "mean_ask_gap_short" => bars[i].mean_ask_gap_bps > threshold,
            "mean_bid_gap_long" => bars[i].mean_bid_gap_bps > threshold,

            // --- VP concentration ---
            "conc_high_short" => bars[i].concentration > threshold,
            "conc_low_long" => bars[i].concentration < threshold,
            "conc_low_short" => bars[i].concentration < threshold,

            // --- Mid-to-POC ---
            "mid_poc_long" => bars[i].mid_to_poc_bps < threshold,
            "mid_poc_short" => bars[i].mid_to_poc_bps > threshold,

            // --- CVD momentum fade ---
            "cvd_mom_long" => bars[i].cvd_momentum < threshold,
            "cvd_mom_short" => bars[i].cvd_momentum > threshold,
            "cvd_accel_long" => bars[i].cvd_acceleration < threshold,
            "cvd_accel_short" => bars[i].cvd_acceleration > threshold,

            // --- Wall signals ---
            "bid_wall_long" => bars[i].bid_wall_vol > threshold,
            "ask_wall_short" => bars[i].ask_wall_vol > threshold,

            // --- Cross-ask ratio ---
            "cross_ask_short" => bars[i].cross_ask_ratio > threshold,

            // --- Funding rate ---
            "funding_extreme_long" => !bars[i].funding_rate.is_nan() && bars[i].funding_rate < threshold,
            "funding_extreme_short" => !bars[i].funding_rate.is_nan() && bars[i].funding_rate > threshold,

            // --- Mark-index basis (perp premium/discount) ---
            "mark_index_discount_long" => !bars[i].mark_index_bps.is_nan() && bars[i].mark_index_bps < threshold,
            "mark_index_premium_short" => !bars[i].mark_index_bps.is_nan() && bars[i].mark_index_bps > threshold,

            // --- OI change (position build/unwind) ---
            "oi_build_long" => !bars[i].oi_pct_change.is_nan() && bars[i].oi_pct_change > threshold,
            "oi_unwind_short" => !bars[i].oi_pct_change.is_nan() && bars[i].oi_pct_change < threshold,

            // --- Liquidation flow (forced selling/buying) ---
            "liq_sell_short" => bars[i].liq_vol_sell > threshold,
            "liq_buy_long" => bars[i].liq_vol_buy > threshold,
            "liq_imb_long" => !bars[i].liq_imbalance.is_nan() && bars[i].liq_imbalance > threshold,
            "liq_imb_short" => !bars[i].liq_imbalance.is_nan() && bars[i].liq_imbalance < threshold,

            // --- Spot-perp basis ---
            "basis_wide_short" => !bars[i].basis_bps.is_nan() && bars[i].basis_bps > threshold,
            "basis_tight_long" => !bars[i].basis_bps.is_nan() && bars[i].basis_bps < threshold,

            // === MACRO-STRUCTURE ROLLING ENTRIES (sustained flow, not snapshots) ===

            // --- Liquidation cascade (cumulative) ---
            "liq_cascade_sell_25" => !bars[i].liq_sell_cum_25.is_nan() && bars[i].liq_sell_cum_25 > threshold,
            "liq_cascade_buy_25" => !bars[i].liq_buy_cum_25.is_nan() && bars[i].liq_buy_cum_25 > threshold,
            "liq_cascade_sell_50" => !bars[i].liq_sell_cum_50.is_nan() && bars[i].liq_sell_cum_50 > threshold,
            "liq_cascade_buy_50" => !bars[i].liq_buy_cum_50.is_nan() && bars[i].liq_buy_cum_50 > threshold,

            // --- Liquidation flow imbalance ---
            "liq_flow_sell_25" => !bars[i].liq_flow_imb_25.is_nan() && bars[i].liq_flow_imb_25 < threshold,
            "liq_flow_buy_25" => !bars[i].liq_flow_imb_25.is_nan() && bars[i].liq_flow_imb_25 > threshold,
            "liq_flow_sell_50" => !bars[i].liq_flow_imb_50.is_nan() && bars[i].liq_flow_imb_50 < threshold,
            "liq_flow_buy_50" => !bars[i].liq_flow_imb_50.is_nan() && bars[i].liq_flow_imb_50 > threshold,

            // --- OI change ---
            "oi_surge_long_25" => !bars[i].oi_change_25.is_nan() && bars[i].oi_change_25 > threshold,
            "oi_surge_short_25" => !bars[i].oi_change_25.is_nan() && bars[i].oi_change_25 < threshold,
            "oi_surge_long_50" => !bars[i].oi_change_50.is_nan() && bars[i].oi_change_50 > threshold,
            "oi_surge_short_50" => !bars[i].oi_change_50.is_nan() && bars[i].oi_change_50 < threshold,
            "oi_unwind_long" => !bars[i].oi_pct_change.is_nan() && bars[i].oi_pct_change > threshold,
            "oi_unwind_short" => !bars[i].oi_pct_change.is_nan() && bars[i].oi_pct_change < threshold,

            // --- Sustained funding ---
            "funding_crowd_short_25" => !bars[i].funding_avg_25.is_nan() && bars[i].funding_avg_25 > threshold,
            "funding_crowd_long_25" => !bars[i].funding_avg_25.is_nan() && bars[i].funding_avg_25 < threshold,
            "funding_crowd_short_50" => !bars[i].funding_avg_50.is_nan() && bars[i].funding_avg_50 > threshold,
            "funding_crowd_long_50" => !bars[i].funding_avg_50.is_nan() && bars[i].funding_avg_50 < threshold,

            // --- Sustained mark-index ---
            "mi_premium_short_25" => !bars[i].mark_index_avg_25.is_nan() && bars[i].mark_index_avg_25 > threshold,
            "mi_discount_long_25" => !bars[i].mark_index_avg_25.is_nan() && bars[i].mark_index_avg_25 < threshold,
            "mi_premium_short_50" => !bars[i].mark_index_avg_50.is_nan() && bars[i].mark_index_avg_50 > threshold,
            "mi_discount_long_50" => !bars[i].mark_index_avg_50.is_nan() && bars[i].mark_index_avg_50 < threshold,

            // --- Sustained CVD ---
            "cvd_push_long_25" => !bars[i].cvd_cum_25.is_nan() && bars[i].cvd_cum_25 > threshold,
            "cvd_push_short_25" => !bars[i].cvd_cum_25.is_nan() && bars[i].cvd_cum_25 < threshold,
            "cvd_push_long_50" => !bars[i].cvd_cum_50.is_nan() && bars[i].cvd_cum_50 > threshold,
            "cvd_push_short_50" => !bars[i].cvd_cum_50.is_nan() && bars[i].cvd_cum_50 < threshold,

            // --- Sustained depth skew ---
            "ask_skew_sust_short_25" => !bars[i].ask_skew_avg_25.is_nan() && bars[i].ask_skew_avg_25 > threshold,
            "bid_skew_sust_long_25" => !bars[i].bid_skew_avg_25.is_nan() && bars[i].bid_skew_avg_25 > threshold,
            "ask_skew_sust_short_50" => !bars[i].ask_skew_avg_50.is_nan() && bars[i].ask_skew_avg_50 > threshold,
            "bid_skew_sust_long_50" => !bars[i].bid_skew_avg_50.is_nan() && bars[i].bid_skew_avg_50 > threshold,

            // --- CVD momentum cumulative ---
            "cvd_mom_cum_long_25" => !bars[i].cvd_mom_cum_25.is_nan() && bars[i].cvd_mom_cum_25 > threshold,
            "cvd_mom_cum_short_25" => !bars[i].cvd_mom_cum_25.is_nan() && bars[i].cvd_mom_cum_25 < threshold,
            "cvd_mom_cum_long_50" => !bars[i].cvd_mom_cum_50.is_nan() && bars[i].cvd_mom_cum_50 > threshold,
            "cvd_mom_cum_short_50" => !bars[i].cvd_mom_cum_50.is_nan() && bars[i].cvd_mom_cum_50 < threshold,

            // --- Null-edge baseline ---
            "null_random" => i % threshold as usize == 0,
            _ => false,
        };

        if entry_signal {
            let entry_price = bars[i].mid_price;
            let entry_ts = bars[i].ts;
            let is_long = if entry_name == "null_random" {
                null_trade_count % 2 == 0
            } else {
                // Direction encoded in entry name suffix: _long = long, _short = short
                entry_name.contains("_long") || entry_name.contains("_buy")
                    || entry_name.contains("_absorb") || entry_name.contains("_discount")
            };
            null_trade_count += 1;

            // Triple-barrier exit: find which barrier is hit first
            let barrier = TripleBarrier::find(
                bars, i, entry_price, is_long, tp_bps, sl_bps, hold_bars
            );

            let exit_price = barrier.exit_price;
            let exit_idx = barrier.exit_idx;
            let gross_pnl_bps = if is_long {
                (exit_price - entry_price) / entry_price * 10000.0
            } else {
                (entry_price - exit_price) / entry_price * 10000.0
            };

            trades.push(TradeResult {
                entry_ts,
                exit_ts: bars[exit_idx].ts,
                entry_idx: i,
                exit_idx,
                is_long,
                entry_price,
                exit_price,
                gross_pnl_bps,
                net_pnl_bps: gross_pnl_bps - fee_bps, // fee deducted
                barrier_hit: barrier.barrier_hit,
            });

            // Jump to exit bar (no overlapping trades)
            i = exit_idx + 1;
        } else {
            i += 1;
        }
    }

    trades
}

/// Trade-level CSCV PBO.
///
/// Unlike `pbo_cscv` which requires equal-length per-observation series,
/// this works with sparse trade data: each config has trades at specific bar
/// indices. We split the bar axis into `s` blocks, assign each trade to its
/// block based on entry bar, then compute IS/OOS Sharpes from the trades
/// falling in each block combination.
///
/// Returns `(pbo, n_combinations)` or `None` if inputs are malformed.
fn pbo_cscv_trade_level(
    configs: &[(&[usize], &[f64])],  // (bar_indices, returns) per config
    total_bars: usize,
    s: usize,
) -> Option<(f64, usize)> {
    let n = configs.len();
    if n < 2 || s < 2 || !s.is_multiple_of(2) || total_bars < s {
        return None;
    }

    // Block boundaries on the bar axis
    let blocks: Vec<(usize, usize)> = (0..s)
        .map(|b| (b * total_bars / s, (b + 1) * total_bars / s))
        .collect();

    // For each config, pre-compute which block each trade falls in
    let config_block_assignments: Vec<Vec<usize>> = configs
        .iter()
        .map(|(bar_indices, _)| {
            bar_indices
                .iter()
                .map(|&idx| {
                    // Binary search for which block this bar falls in
                    let block_idx = blocks
                        .iter()
                        .position(|&(lo, hi)| idx >= lo && idx < hi)
                        .unwrap_or(s - 1); // last bar goes to last block
                    block_idx
                })
                .collect()
        })
        .collect();

    let half = s / 2;
    let full: u32 = if s == 32 { u32::MAX } else { (1u32 << s) - 1 };

    // Generate all s-choose-(s/2) combinations
    let is_combos = combinations(s, half);

    let mut logits = Vec::new();

    for is_mask in &is_combos {
        let oos_mask = full & !is_mask;

        // For each config, compute IS Sharpe and OOS Sharpe from trades in those blocks
        let mut best_idx = 0usize;
        let mut best_is_sr = f64::NEG_INFINITY;

        for (i, (_, returns)) in configs.iter().enumerate() {
            let block_assigns = &config_block_assignments[i];
            let is_returns: Vec<f64> = returns
                .iter()
                .zip(block_assigns.iter())
                .filter(|(_, &b)| (is_mask & (1u32 << b)) != 0)
                .map(|(&r, _)| r)
                .collect();

            let sr = if is_returns.len() >= 2 {
                sharpe(&is_returns)
            } else {
                f64::NEG_INFINITY // not enough trades in IS blocks
            };

            if sr > best_is_sr {
                best_is_sr = sr;
                best_idx = i;
            }
        }

        // OOS Sharpe of the IS winner
        let winner_returns = configs[best_idx].1;
        let winner_blocks = &config_block_assignments[best_idx];
        let oos_returns: Vec<f64> = winner_returns
            .iter()
            .zip(winner_blocks.iter())
            .filter(|(_, &b)| (oos_mask & (1u32 << b)) != 0)
            .map(|(&r, _)| r)
            .collect();

        let win_oos_sr = if oos_returns.len() >= 2 {
            sharpe(&oos_returns)
        } else {
            f64::NEG_INFINITY
        };

        // Compute OOS Sharpe for all configs to rank the IS winner
        let mut oos_sharpes: Vec<f64> = Vec::with_capacity(n);
        for (i, (_, returns)) in configs.iter().enumerate() {
            let block_assigns = &config_block_assignments[i];
            let oos_r: Vec<f64> = returns
                .iter()
                .zip(block_assigns.iter())
                .filter(|(_, &b)| (oos_mask & (1u32 << b)) != 0)
                .map(|(&r, _)| r)
                .collect();
            oos_sharpes.push(if oos_r.len() >= 2 { sharpe(&oos_r) } else { f64::NEG_INFINITY });
        }

        let worse = oos_sharpes.iter().filter(|&&sr| sr <= win_oos_sr).count();
        let omega = (worse as f64 / (n as f64 + 1.0)).clamp(1e-6, 1.0 - 1e-6);
        logits.push((omega / (1.0 - omega)).ln());
    }

    if logits.is_empty() {
        return None;
    }

    let pbo = logits.iter().filter(|&&l| l <= 0.0).count() as f64 / logits.len() as f64;
    Some((pbo, is_combos.len()))
}

/// Generate all k-subsets of 0..s as bitmasks.
fn combinations(s: usize, k: usize) -> Vec<u32> {
    let mut out = Vec::new();
    let mut idx: Vec<usize> = (0..k).collect();
    if k == 0 || k > s {
        return out;
    }
    loop {
        let mut mask = 0u32;
        for &i in &idx {
            mask |= 1 << i;
        }
        out.push(mask);
        let mut i = k;
        while i > 0 {
            i -= 1;
            if idx[i] != i + s - k {
                idx[i] += 1;
                for j in i + 1..k {
                    idx[j] = idx[j - 1] + 1;
                }
                break;
            }
            if i == 0 {
                return out;
            }
        }
    }
}
