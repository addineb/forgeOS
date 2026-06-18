//! Sweepscope: entry+exit grid search on depthscope CSVs.
//!
//! EUR500 account, 20x leverage. Reports EUR PnL + EUR drawdown.
//! Verdict is per-family, not per-direction.
//! Configs with max DD > EUR150 are auto-retired.

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

#[derive(Parser, Debug)]
#[command(name = "sweepscope")]
struct Args {
    #[arg(long)]
    input: PathBuf,
    #[arg(long, default_value = "sweepscope_scorecard.csv")]
    output: PathBuf,
    #[arg(long, default_value_t = 5)]
    folds: usize,
    #[arg(long, default_value_t = 90)]
    purge_bars: usize,
    #[arg(long, default_value_t = 15)]
    min_trades: usize,
    #[arg(long, default_value_t = 0.9)]
    dsr_promote: f64,
    #[arg(long, default_value_t = 0.6)]
    dsr_park: f64,
    #[arg(long, default_value_t = 9.0)]
    fee_bps: f64,
    #[arg(long, default_value_t = false)]
    null_edge: bool,
    #[arg(long, default_value_t = 20)]
    null_spacing: usize,
    #[arg(long)]
    per_date: Option<PathBuf>,
    #[arg(long, default_value_t = 500.0)]
    eur_capital: f64,
    #[arg(long, default_value_t = 20.0)]
    eur_leverage: f64,
    #[arg(long, default_value_t = 150.0)]
    eur_max_dd: f64,
}

#[derive(Debug, Clone, serde::Deserialize)]
#[allow(dead_code)]
struct Bar {
    ts: i64,
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
    oos_net_bps: f64,
    oos_win_rate: f64,
    eur_total_pnl: f64,
    eur_per_trade: f64,
    eur_max_dd: f64,
    eur_liquidation_risk: bool,
    trades_per_day: f64,
    verdict: Verdict,
    trade_bar_indices: Vec<usize>,
    trade_returns: Vec<f64>,
    per_date: Vec<(String, usize, f64, f64)>,
}

fn find_date_boundaries(bars: &[Bar]) -> Vec<usize> {
    if bars.is_empty() { return vec![0, 0]; }
    let mut boundaries = vec![0];
    let mut prev_date = &bars[0].date;
    for i in 1..bars.len() {
        if &bars[i].date != prev_date { boundaries.push(i); prev_date = &bars[i].date; }
    }
    boundaries.push(bars.len());
    boundaries
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    eprintln!("Loading {}...", args.input.display());
    let bars = load_csv(&args.input)?;
    eprintln!("Loaded {} bars", bars.len());
    if bars.is_empty() { eprintln!("No data."); return Ok(()); }

    let grid = SweepGrid::default();
    let configs = grid.expand();
    eprintln!("Sweep grid: {} configs (TP {:?} SL {:?} hold {:?})", configs.len(), grid.tp_bps, grid.sl_bps, grid.hold_bars);

    let boundaries = find_date_boundaries(&bars);
    let n_dates = boundaries.len() - 1;
    let cv = PurgedCv::new_date_aware(boundaries, args.folds, args.purge_bars);
    eprintln!("CV: {} folds, {} dates, {} bars purge", args.folds, n_dates, args.purge_bars);

    let position_eur = args.eur_capital * args.eur_leverage;
    eprintln!("EUR account: capital={:.0} leverage={:.0}x position={:.0} max_dd={:.0}",
        args.eur_capital, args.eur_leverage, position_eur, args.eur_max_dd);

    let fee_bps = args.fee_bps;
    if args.null_edge {
        eprintln!("\n--- NULL-EDGE ---");
        for &hold in &[5, 10, 30, 90] {
            for &tp in &[0.0, 15.0, 30.0] {
                for &sl in &[0.0, 15.0, 30.0] {
                    let trades = run_trades("null_random", args.null_spacing as f64, tp, sl, hold, fee_bps, &bars, position_eur);
                    let n = trades.len();
                    if n == 0 { continue; }
                    let net = trades.iter().map(|t| t.net_pnl_bps).sum::<f64>() / n as f64;
                    eprintln!("  null(tp={:.0},sl={:.0},hold={}): {:3} tr, net={:+.1}bps", tp, sl, hold, n, net);
                }
            }
        }
        eprintln!("--- END NULL-EDGE ---\n");
    }

    let mut results: Vec<CellResult> = configs
        .par_iter()
        .enumerate()
        .map(|(id, cfg)| {
            run_config(id, cfg, &bars, &cv, fee_bps, args.min_trades, args.eur_capital, args.eur_leverage, args.eur_max_dd, position_eur)
        })
        .collect();

    fn entry_family(name: &str) -> String {
        let without_window = {
            let parts: Vec<&str> = name.rsplitn(2, '_').collect();
            if parts.len() == 2 && parts[0].chars().all(|c| c.is_ascii_digit()) { parts[1].to_string() } else { name.to_string() }
        };
        for suffix in &["_short", "_long", "_buy", "_sell", "_absorb", "_discount"] {
            if without_window.ends_with(suffix) { return without_window[..without_window.len() - suffix.len()].to_string(); }
        }
        without_window
    }

    let mut family_sharpes: std::collections::HashMap<String, Vec<f64>> = std::collections::HashMap::new();
    for r in results.iter().filter(|r| r.round_trips >= args.min_trades && r.sharpe <= 5.0 && !r.eur_liquidation_risk) {
        let fam = entry_family(&r.entry_name);
        family_sharpes.entry(fam).or_default().push(r.sharpe.clamp(-5.0, 5.0));
    }
    let family_avg_sharpes: Vec<f64> = family_sharpes.values().map(|v| v.iter().sum::<f64>() / v.len() as f64).collect();
    let n_families = family_avg_sharpes.len();
    let var_sharpes = variance_of_sharpes(&family_avg_sharpes);
    eprintln!("DSR families: {}  var_sharpes={:.4}", n_families, var_sharpes);

    for r in &mut results {
        if r.round_trips >= args.min_trades && r.sharpe <= 5.0 && !r.eur_liquidation_risk {
            r.dsr = deflated_sharpe(&r.trade_returns, n_families, var_sharpes);
        } else { r.dsr = 0.0; }
    }

    // Simple verdict: survive EUR DD + profitable + spread across enough dates
    // PROMOTE = EUR>0, DD<limit, ≥3 dates with trades, OOS>0
    // PARK = EUR>0, DD<limit, but <3 dates or OOS≤0
    // RETIRE = DD>limit or EUR≤0
    let mut family_dates: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut family_eur: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    let mut family_dd: std::collections::HashMap<String, f64> = std::collections::HashMap::new();
    let mut family_oos: std::collections::HashMap<String, f64> = std::collections::HashMap::new();

    for r in &results {
        if r.round_trips < args.min_trades { continue; }
        let fam = entry_family(&r.entry_name);
        let dates = r.per_date.iter().filter(|(_, n, _, _)| *n > 0).count();
        family_dates.entry(fam.clone()).or_insert(0);
        if dates > family_dates[&fam] { *family_dates.get_mut(&fam).unwrap() = dates; }
        let e = family_eur.entry(fam.clone()).or_insert(f64::NEG_INFINITY);
        if r.eur_total_pnl > *e { *e = r.eur_total_pnl; }
        let d = family_dd.entry(fam.clone()).or_insert(f64::MAX);
        if r.eur_max_dd < *d { *d = r.eur_max_dd; }
        let o = family_oos.entry(fam.clone()).or_insert(f64::NEG_INFINITY);
        if r.oos_net_bps > *o { *o = r.oos_net_bps; }
    }

    // For each family, check if both directions have trades
    let mut family_has_long: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
    let mut family_has_short: std::collections::HashMap<String, bool> = std::collections::HashMap::new();
    for r in &results {
        if r.round_trips < args.min_trades { continue; }
        let fam = entry_family(&r.entry_name);
        let is_long = r.entry_name.contains("_long") || r.entry_name.contains("_buy") || r.entry_name.contains("_absorb") || r.entry_name.contains("_discount");
        if is_long { family_has_long.insert(fam.clone(), true); }
        else { family_has_short.insert(fam.clone(), true); }
    }

    let mut family_verdicts: std::collections::HashMap<String, Verdict> = std::collections::HashMap::new();
    let mut promoted: Vec<String> = Vec::new();
    let mut parked: Vec<String> = Vec::new();
    for fam in family_dates.keys() {
        let nd = family_dates.get(fam).copied().unwrap_or(0);
        let eur = family_eur.get(fam).copied().unwrap_or(f64::NEG_INFINITY);
        let dd = family_dd.get(fam).copied().unwrap_or(f64::MAX);
        let oos = family_oos.get(fam).copied().unwrap_or(f64::NEG_INFINITY);
        let has_long = family_has_long.get(fam).copied().unwrap_or(false);
        let has_short = family_has_short.get(fam).copied().unwrap_or(false);
        let has_both = has_long && has_short;
        let v = if dd <= args.eur_max_dd && eur > 0.0 && nd >= 3 && oos > 0.0 && has_both {
            Verdict::Promote
        } else if dd <= args.eur_max_dd && eur > 0.0 {
            Verdict::Park
        } else {
            Verdict::Retire
        };
        family_verdicts.insert(fam.clone(), v);
        match v { Verdict::Promote => promoted.push(fam.clone()), Verdict::Park => parked.push(fam.clone()), _ => {} }
    }

    for r in &mut results {
        let fam = entry_family(&r.entry_name);
        r.verdict = family_verdicts.get(&fam).copied().unwrap_or(Verdict::Retire);
    }

    // Write scorecard
    let mut out = File::create(&args.output)?;
    writeln!(out, "id,entry,threshold,tp_bps,sl_bps,hold_bars,trades,win_rate,net_pnl_bps,sharpe,dsr,oos_net_bps,oos_win_rate,eur_total_pnl,eur_per_trade,eur_max_dd,eur_dd_pct,trades_per_day,verdict,family")?;

    let mut n_p = 0; let mut n_k = 0; let mut n_r = 0;
    for r in &results {
        match r.verdict { Verdict::Promote => n_p += 1, Verdict::Park => n_k += 1, Verdict::Retire => n_r += 1, }
        let fam = entry_family(&r.entry_name);
        writeln!(out, "{},{},{:.1},{:.1},{:.1},{},{},{:.3},{:.2},{:.2},{:.3},{:.2},{:.3},{:.2},{:.2},{:.2},{:.1},{:.2},{},{}",
            r.id, r.entry_name, r.entry_threshold, r.tp_bps, r.sl_bps, r.hold_bars,
            r.round_trips, r.win_rate, r.net_pnl_bps, r.sharpe, r.dsr,
            r.oos_net_bps, r.oos_win_rate,
            r.eur_total_pnl, r.eur_per_trade, r.eur_max_dd, r.eur_max_dd / args.eur_capital * 100.0,
            r.trades_per_day, r.verdict, fam)?;
    }

    eprintln!("\n=== SWEEP SCORECARD ===");
    eprintln!("  Total: {}  PROMOTE: {} ({} families)  PARK: {} ({} families)  RETIRE: {}",
        n_p + n_k + n_r, n_p, promoted.len(), n_k, parked.len(), n_r);
    if !promoted.is_empty() { eprintln!("  Promoted: {:?}", promoted); }
    if !parked.is_empty() { eprintln!("  Parked: {:?}", parked); }
    eprintln!("  Scorecard: {}", args.output.display());

    // Top 10 by EUR total PnL
    let mut sorted: Vec<&CellResult> = results.iter().filter(|r| r.round_trips >= args.min_trades && !r.eur_liquidation_risk).collect();
    sorted.sort_by(|a, b| b.eur_total_pnl.partial_cmp(&a.eur_total_pnl).unwrap_or(std::cmp::Ordering::Equal));
    eprintln!("\nTop 10 by EUR total PnL:");
    for r in sorted.iter().take(10) {
        eprintln!("  {:>12}  tp={:>3.0} sl={:>3.0} hold={:>3}  EUR={:+7.0}  DD={:.0}  DSR={:.3}  {}/d  {}",
            r.entry_name, r.tp_bps, r.sl_bps, r.hold_bars, r.eur_total_pnl, r.eur_max_dd, r.dsr, r.trades_per_day, r.verdict);
    }

    // Per-date
    if let Some(per_date_path) = &args.per_date {
        let mut pd_out = File::create(per_date_path)?;
        writeln!(pd_out, "id,entry,threshold,tp_bps,sl_bps,hold_bars,date,trades,net_bps,win_rate,verdict,family")?;
        for r in &results {
            if r.round_trips < args.min_trades { continue; }
            for (date, trades, net_bps, wr) in &r.per_date {
                writeln!(pd_out, "{},{},{:.1},{:.1},{:.1},{},{},{},{:.2},{:.3},{},{}",
                    r.id, r.entry_name, r.entry_threshold, r.tp_bps, r.sl_bps, r.hold_bars,
                    date, trades, net_bps, wr, r.verdict, entry_family(&r.entry_name))?;
            }
        }
        eprintln!("Per-date: {}", per_date_path.display());
    }

    Ok(())
}

fn load_csv(path: &PathBuf) -> Result<Vec<Bar>, Box<dyn std::error::Error>> {
    let mut reader = ReaderBuilder::new().from_path(path)?;
    let mut bars = Vec::new();
    for result in reader.deserialize() {
        let mut bar: Bar = result?;
        if bar.date.is_empty() {
            let secs = bar.ts / 1_000_000_000;
            let days_since_epoch = secs / 86400;
            let (y, m, d) = days_to_ymd(days_since_epoch);
            bar.date = format!("{:04}-{:02}-{:02}", y, m, d);
        }
        bars.push(bar);
    }
    Ok(bars)
}

fn days_to_ymd(mut days: i64) -> (i64, u32, u32) {
    days += 719468;
    let era = if days >= 0 { days / 146097 } else { (days - 146096) / 146097 };
    let doe = days - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let month = (5 * doy + 2) / 153;
    let day = doy - (153 * month + 2) / 5 + 1;
    let year = yoe + era * 400 + if month < 10 { 0 } else { 1 };
    (year, if month < 10 { month + 3 } else { month - 9 } as u32, day as u32)
}

fn run_config(id: usize, cfg: &grid::SweepConfig, bars: &[Bar], cv: &PurgedCv,
    fee_bps: f64, _min_trades: usize, eur_capital: f64, _eur_leverage: f64, eur_max_dd: f64, position_eur: f64) -> CellResult {

    let all_trades = run_trades(&cfg.entry_name, cfg.entry_threshold, cfg.tp_bps, cfg.sl_bps, cfg.hold_bars, fee_bps, bars, position_eur);
    let trade_bar_indices: Vec<usize> = all_trades.iter().map(|t| t.entry_idx).collect();
    let trade_returns: Vec<f64> = all_trades.iter().map(|t| t.net_pnl_bps / 10000.0).collect();

    let mut oos_trades_all = Vec::new();
    for fi in 0..cv.n_folds() {
        let oos_range = cv.oos_range(fi);
        let oos_bars = &bars[oos_range.clone()];
        oos_trades_all.extend(run_trades(&cfg.entry_name, cfg.entry_threshold, cfg.tp_bps, cfg.sl_bps, cfg.hold_bars, fee_bps, oos_bars, position_eur));
    }

    let mut date_map: std::collections::BTreeMap<String, Vec<&TradeResult>> = std::collections::BTreeMap::new();
    for t in &all_trades {
        date_map.entry(bars[t.entry_idx].date.clone()).or_default().push(t);
    }
    let per_date: Vec<(String, usize, f64, f64)> = date_map.iter().map(|(date, trades)| {
        let n = trades.len();
        let net = trades.iter().map(|t| t.net_pnl_bps).sum::<f64>() / n as f64;
        let wins = trades.iter().filter(|t| t.net_pnl_bps > 0.0).count();
        (date.clone(), n, net, wins as f64 / n as f64)
    }).collect();

    let round_trips = all_trades.len();
    let (win_rate, avg_pnl_bps, net_pnl_bps, sharpe_val) = if round_trips == 0 { (0.0, 0.0, 0.0, 0.0) }
    else {
        let wins = all_trades.iter().filter(|t| t.net_pnl_bps > 0.0).count();
        let wr = wins as f64 / round_trips as f64;
        let avg = all_trades.iter().map(|t| t.gross_pnl_bps).sum::<f64>() / round_trips as f64;
        let net = all_trades.iter().map(|t| t.net_pnl_bps).sum::<f64>() / round_trips as f64;
        let rets: Vec<f64> = all_trades.iter().map(|t| t.net_pnl_bps / 10000.0).collect();
        (wr, avg, net, sharpe(&rets))
    };

    let (oos_net_bps, oos_win_rate) = if oos_trades_all.is_empty() { (0.0, 0.0) }
    else {
        let oos_net = oos_trades_all.iter().map(|t| t.net_pnl_bps).sum::<f64>() / oos_trades_all.len() as f64;
        let oos_wins = oos_trades_all.iter().filter(|t| t.net_pnl_bps > 0.0).count();
        (oos_net, oos_wins as f64 / oos_trades_all.len() as f64)
    };

    // EUR500 computations
    let eur_total_pnl = if round_trips > 0 {
        all_trades.iter().map(|t| t.eur_pnl).sum::<f64>()
    } else { 0.0 };
    let eur_per_trade = if round_trips > 0 { eur_total_pnl / round_trips as f64 } else { 0.0 };

    // Compute max drawdown: equity starts at EUR500 capital, not zero.
    // If equity drops below zero, account is liquidated — max DD = EUR500.
    let start_capital = eur_capital;
    let mut equity = start_capital;
    let mut peak = start_capital;
    let mut max_dd = 0.0f64;
    let mut liquidated = false;
    for t in &all_trades {
        equity += t.eur_pnl;
        if equity <= 0.0 { liquidated = true; break; }
        if equity > peak { peak = equity; }
        let dd = peak - equity;
        if dd > max_dd { max_dd = dd; }
    }
    let eur_max_dd_val = if liquidated { 9999.0 } else { max_dd }; // 9999 = liquidated flag
    let liquidation_risk = liquidated || eur_max_dd_val > eur_max_dd;
    let trades_per_day = if n_dates_sorted(&date_map) > 0 {
        round_trips as f64 / n_dates_sorted(&date_map) as f64
    } else { 0.0 };

    CellResult {
        id, entry_name: cfg.entry_name.clone(), entry_threshold: cfg.entry_threshold,
        tp_bps: cfg.tp_bps, sl_bps: cfg.sl_bps, hold_bars: cfg.hold_bars,
        fee_bps, round_trips, win_rate, avg_pnl_bps, net_pnl_bps, sharpe: sharpe_val,
        dsr: 0.0, oos_net_bps, oos_win_rate,
        eur_total_pnl, eur_per_trade, eur_max_dd: eur_max_dd_val, eur_liquidation_risk: liquidation_risk,
        trades_per_day,
        verdict: Verdict::Retire, trade_bar_indices, trade_returns, per_date,
    }
}

fn n_dates_sorted(date_map: &std::collections::BTreeMap<String, Vec<&TradeResult>>) -> usize {
    date_map.len()
}

fn run_trades(entry_name: &str, threshold: f64, tp_bps: f64, sl_bps: f64, hold_bars: usize,
    fee_bps: f64, bars: &[Bar], position_eur: f64) -> Vec<TradeResult> {

    let mut trades = Vec::new();
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
            "funding_extreme_long" => !bars[i].funding_rate.is_nan() && bars[i].funding_rate < threshold,
            "funding_extreme_short" => !bars[i].funding_rate.is_nan() && bars[i].funding_rate > threshold,
            "mark_index_discount_long" => !bars[i].mark_index_bps.is_nan() && bars[i].mark_index_bps < threshold,
            "mark_index_premium_short" => !bars[i].mark_index_bps.is_nan() && bars[i].mark_index_bps > threshold,
            "oi_build_long" => !bars[i].oi_pct_change.is_nan() && bars[i].oi_pct_change > threshold,
            "oi_unwind_short" => !bars[i].oi_pct_change.is_nan() && bars[i].oi_pct_change < threshold,
            "liq_sell_short" => bars[i].liq_vol_sell > threshold,
            "liq_buy_long" => bars[i].liq_vol_buy > threshold,
            "liq_imb_long" => !bars[i].liq_imbalance.is_nan() && bars[i].liq_imbalance > threshold,
            "liq_imb_short" => !bars[i].liq_imbalance.is_nan() && bars[i].liq_imbalance < threshold,
            "basis_wide_short" => !bars[i].basis_bps.is_nan() && bars[i].basis_bps > threshold,
            "basis_tight_long" => !bars[i].basis_bps.is_nan() && bars[i].basis_bps < threshold,
            "liq_cascade_sell_25" => !bars[i].liq_sell_cum_25.is_nan() && bars[i].liq_sell_cum_25 > threshold,
            "liq_cascade_buy_25" => !bars[i].liq_buy_cum_25.is_nan() && bars[i].liq_buy_cum_25 > threshold,
            "liq_cascade_sell_50" => !bars[i].liq_sell_cum_50.is_nan() && bars[i].liq_sell_cum_50 > threshold,
            "liq_cascade_buy_50" => !bars[i].liq_buy_cum_50.is_nan() && bars[i].liq_buy_cum_50 > threshold,
            "liq_flow_sell_25" => !bars[i].liq_flow_imb_25.is_nan() && bars[i].liq_flow_imb_25 < threshold,
            "liq_flow_buy_25" => !bars[i].liq_flow_imb_25.is_nan() && bars[i].liq_flow_imb_25 > threshold,
            "liq_flow_sell_50" => !bars[i].liq_flow_imb_50.is_nan() && bars[i].liq_flow_imb_50 < threshold,
            "liq_flow_buy_50" => !bars[i].liq_flow_imb_50.is_nan() && bars[i].liq_flow_imb_50 > threshold,
            "oi_surge_long_25" => !bars[i].oi_change_25.is_nan() && bars[i].oi_change_25 > threshold,
            "oi_surge_short_25" => !bars[i].oi_change_25.is_nan() && bars[i].oi_change_25 < threshold,
            "oi_surge_long_50" => !bars[i].oi_change_50.is_nan() && bars[i].oi_change_50 > threshold,
            "oi_surge_short_50" => !bars[i].oi_change_50.is_nan() && bars[i].oi_change_50 < threshold,
            "oi_unwind_long" => !bars[i].oi_pct_change.is_nan() && bars[i].oi_pct_change > threshold,
            "funding_crowd_short_25" => !bars[i].funding_avg_25.is_nan() && bars[i].funding_avg_25 > threshold,
            "funding_crowd_long_25" => !bars[i].funding_avg_25.is_nan() && bars[i].funding_avg_25 < threshold,
            "funding_crowd_short_50" => !bars[i].funding_avg_50.is_nan() && bars[i].funding_avg_50 > threshold,
            "funding_crowd_long_50" => !bars[i].funding_avg_50.is_nan() && bars[i].funding_avg_50 < threshold,
            "mi_premium_short_25" => !bars[i].mark_index_avg_25.is_nan() && bars[i].mark_index_avg_25 > threshold,
            "mi_discount_long_25" => !bars[i].mark_index_avg_25.is_nan() && bars[i].mark_index_avg_25 < threshold,
            "mi_premium_short_50" => !bars[i].mark_index_avg_50.is_nan() && bars[i].mark_index_avg_50 > threshold,
            "mi_discount_long_50" => !bars[i].mark_index_avg_50.is_nan() && bars[i].mark_index_avg_50 < threshold,
            "cvd_push_long_25" => !bars[i].cvd_cum_25.is_nan() && bars[i].cvd_cum_25 > threshold,
            "cvd_push_short_25" => !bars[i].cvd_cum_25.is_nan() && bars[i].cvd_cum_25 < threshold,
            "cvd_push_long_50" => !bars[i].cvd_cum_50.is_nan() && bars[i].cvd_cum_50 > threshold,
            "cvd_push_short_50" => !bars[i].cvd_cum_50.is_nan() && bars[i].cvd_cum_50 < threshold,
            "cvd_mom_cum_long_25" => !bars[i].cvd_mom_cum_25.is_nan() && bars[i].cvd_mom_cum_25 > threshold,
            "cvd_mom_cum_short_25" => !bars[i].cvd_mom_cum_25.is_nan() && bars[i].cvd_mom_cum_25 < threshold,
            "cvd_mom_cum_long_50" => !bars[i].cvd_mom_cum_50.is_nan() && bars[i].cvd_mom_cum_50 > threshold,
            "cvd_mom_cum_short_50" => !bars[i].cvd_mom_cum_50.is_nan() && bars[i].cvd_mom_cum_50 < threshold,
            "null_random" => i % threshold as usize == 0,
            _ => false,
        };

        if entry_signal {
            let entry_price = bars[i].mid_price;
            let entry_ts = bars[i].ts;
            let is_long = if entry_name == "null_random" { null_ct % 2 == 0 } else {
                entry_name.contains("_long") || entry_name.contains("_buy") || entry_name.contains("_absorb") || entry_name.contains("_discount")
            };
            null_ct += 1;

            let barrier = TripleBarrier::find(bars, i, entry_price, is_long, tp_bps, sl_bps, hold_bars);
            let exit_price = barrier.exit_price;
            let exit_idx = barrier.exit_idx;
            let gross_pnl_bps = if is_long { (exit_price - entry_price) / entry_price * 10000.0 }
                else { (entry_price - exit_price) / entry_price * 10000.0 };
            let net_pnl_bps = gross_pnl_bps - fee_bps;

            // EUR profit = net_bps / 10000 * position_eur
            let eur_pnl = net_pnl_bps / 10000.0 * position_eur;

            trades.push(TradeResult {
                entry_ts, exit_ts: bars[exit_idx].ts, entry_idx: i, exit_idx,
                is_long, entry_price, exit_price, gross_pnl_bps, net_pnl_bps,
                eur_pnl, barrier_hit: barrier.barrier_hit,
            });

            i = exit_idx + 1;
        } else { i += 1; }
    }
    trades
}
