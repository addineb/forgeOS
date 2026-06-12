//! `lag-hunt` - sweep basis-reversion variants across many days at a given
//! ORDER LATENCY. Judged on the PER-TRADE t-stat (the correct lens for a sparse
//! minutes-hold strategy) + an EUR-account paper result + a shuffle control.
//! Memory-efficient: one day in memory at a time; configs run in parallel
//! within each day, then the window is dropped.
//!
//!   lag-hunt --root /root/chd/fresh/ticks --dates 2026-05-20,... \
//!     --latency-ns 300000000 --depths 5 --thresholds 8,10,12 --windows 500 \
//!     --reversions true --holds 2m,3m --cooldowns 30s [--shuffle]
//!
//! MAKER MODE (`--maker`): sweep the resting-limit `MakerQuoter` instead of the
//! taker `BasisSignal`/`Managed`, reusing the SAME day-in-memory + rayon-per-cell
//! structure (load_window + the per-day loop). Maker axes (comma lists):
//! --quote-offset --entry-thr --maker-exit --reprice-tol --danger --cancel-lat
//! --pos-cap --inv-skew; plus reused --windows/--depths (FairValue window/top_n).
//! Scalars: --staleness --sample --hold. --qty sets the quote size (wired to BOTH
//! QuoteConfig.quote_qty and InventoryConfig.quote_qty so try_new's consistency
//! check passes). Reports net expectancy %, per-trade t-stat, fill rate %, max abs
//! inventory, max drawdown %, round trips, paper %, plus a KNOB-BITE verdict per
//! swept axis (a "no edge" verdict only counts where the dial moved trades).
//!
//! FEES (matters for an honest Task 12): the engine defaults to
//! `FeeSchedule::legacy()`, whose maker rate is a REBATE (-0.2 bps) that FLATTERS
//! a maker run. `--maker-fee-bps` / `--taker-fee-bps` override it so a realistic
//! positive Hyperliquid maker fee (e.g. 1.5 bps) can be used. The maker fee in
//! use is ALWAYS printed so the rebate can never hide in a maker result.

use std::process::ExitCode;

use forge_core::{Qty, SCALE_F64};
use forge_metrics::{paper_run, PaperConfig};
use rayon::prelude::*;

use forgelag::{
    load_window, BasisConfig, BasisSignal, FairValueConfig, FeeSchedule, FeedConfig,
    InventoryConfig, LagConfig, LagEngine, MakerQuoter, MakerQuoterConfig, Managed, ManagedConfig,
    QuoteConfig, DEFAULT_STALENESS_NS,
};

fn parse_f64s(s: &str) -> Result<Vec<f64>, String> {
    s.split(',').map(|x| x.trim().parse().map_err(|e| format!("bad number `{x}`: {e}"))).collect()
}
fn parse_usizes(s: &str) -> Result<Vec<usize>, String> {
    s.split(',').map(|x| x.trim().parse().map_err(|e| format!("bad int `{x}`: {e}"))).collect()
}
fn parse_bools(s: &str) -> Result<Vec<bool>, String> {
    s.split(',').map(|x| match x.trim() { "true" | "1" => Ok(true), "false" | "0" => Ok(false), o => Err(format!("bad bool `{o}`")) }).collect()
}
fn parse_dur(s: &str) -> Result<u64, String> {
    let s = s.trim();
    let (num, mult): (&str, u64) = if let Some(p) = s.strip_suffix("ms") { (p, 1_000_000) }
        else if let Some(p) = s.strip_suffix("us") { (p, 1_000) }
        else if let Some(p) = s.strip_suffix("ns") { (p, 1) }
        else if let Some(p) = s.strip_suffix('h') { (p, 3_600_000_000_000) }
        else if let Some(p) = s.strip_suffix('m') { (p, 60_000_000_000) }
        else if let Some(p) = s.strip_suffix('s') { (p, 1_000_000_000) }
        else { (s, 1) };
    let v: f64 = num.trim().parse().map_err(|e| format!("bad duration `{s}`: {e}"))?;
    if !v.is_finite() || v < 0.0 { return Err(format!("bad duration `{s}`")); }
    Ok((v * mult as f64) as u64)
}
fn parse_durs(s: &str) -> Result<Vec<u64>, String> {
    s.split(',').map(parse_dur).collect()
}
/// pos-cap axis: parse as BASE-UNIT quantities (same units as --qty), converted
/// to raw via `Qty::from_f64`, so a cap stays consistent with the quote size and
/// is never accidentally below one quote (which would suppress every quote).
fn parse_poscaps(s: &str) -> Result<Vec<i64>, String> {
    s.split(',').map(|x| {
        let v: f64 = x.trim().parse().map_err(|e| format!("bad pos-cap `{x}`: {e}"))?;
        Qty::from_f64(v).map(|q| q.raw()).map_err(|e| format!("pos-cap `{x}`: {e}"))
    }).collect()
}
fn parse_latdist(s: &str) -> Result<Vec<u64>, String> {
    if s.trim() == "real" {
        return Ok(vec![737_700_000, 874_200_000, 716_100_000, 722_100_000, 727_600_000, 798_600_000, 854_000_000, 702_200_000, 698_300_000, 884_700_000, 1_101_200_000, 797_600_000, 836_200_000, 690_500_000, 1_106_500_000, 736_400_000, 793_500_000, 717_100_000, 894_100_000, 732_500_000]);
    }
    s.split(',').map(|x| { let v: f64 = x.trim().parse().map_err(|e| format!("bad latdist `{x}`: {e}"))?; Ok((v * 1_000_000.0) as u64) }).collect()
}
fn fmt_dur(ns: u64) -> String {
    if ns == 0 { "0".to_string() }
    else if ns >= 60_000_000_000 && ns.is_multiple_of(60_000_000_000) { format!("{}m", ns / 60_000_000_000) }
    else if ns >= 1_000_000_000 && ns.is_multiple_of(1_000_000_000) { format!("{}s", ns / 1_000_000_000) }
    else { format!("{}ms", ns / 1_000_000) }
}

/// bps -> fee `rate_raw` (scaled by forge-core `SCALE`): `bps/1e4 * SCALE`.
/// Legacy taker 2.5bps -> 25_000; legacy maker -0.2bps (a REBATE) -> -2_000.
fn rate_raw_from_bps(bps: f64) -> i64 {
    (bps / 1e4 * SCALE_F64).round() as i64
}

/// (per-trip (close_ts, return%), per-trip entry notional).
type TripData = (Vec<(u64, f64)>, Vec<f64>);

#[derive(Clone, Copy)]
struct Cell {
    basis: BasisConfig,
    managed: ManagedConfig,
    depth: usize,
    thr: f64,
    win: usize,
    rev: bool,
    hold: u64,
    lim: bool,
}

/// One maker sweep point (a cartesian-product cell over the maker axes).
#[derive(Clone, Copy)]
struct MakerCell {
    qoff: f64,
    ethr: f64,
    exit: f64,
    rtol: f64,
    danger: f64,
    clat: u64,
    poscap: i64,
    invskew: f64,
    depth: usize,
    win: usize,
}

/// Per-cell trade-behavior fingerprint for the knob-bite rule (Req 8.1): the
/// quote/signal activity that MUST change between adjacent sweep points for a
/// "no edge" verdict at that point to be valid. Compares count + a cheap
/// timestamp sum (a change in side/price/timing shifts fills -> shifts trips ->
/// shifts the close-timestamp sum).
#[derive(Clone, Copy, PartialEq, Eq, Default)]
struct Fp {
    q_sub: u64,
    round_trips: u64,
    maker_fills: u64,
    ts_sum: u64,
}

/// Per-day, per-cell maker run output (returned from the rayon closure).
struct MakerOut {
    trip_returns: Vec<(u64, f64)>,
    trip_notionals: Vec<f64>,
    q_sub: u64,
    q_fill: u64,
    maker_fills: u64,
    round_trips: u64,
    max_inv: i64,
    max_dd: f64,
    ts_sum: u64,
}

/// Everything the maker sweep needs (kept in a struct so `run_maker` stays a
/// single-argument function).
struct MakerRun {
    root: String,
    coin: String,
    symbol: String,
    dates: Vec<String>,
    hours: Vec<String>,
    latency_ns: u64,
    q: Qty,
    fees: FeeSchedule,
    top: usize,
    shuffle: bool,
    latdist: Vec<u64>,
    staleness_ns: u64,
    sample_ns: u64,
    hold_ns: u64,
    maker_fee_set: bool,
    taker_fee_set: bool,
    ax_depth: Vec<usize>,
    ax_win: Vec<usize>,
    ax_qoff: Vec<f64>,
    ax_ethr: Vec<f64>,
    ax_exit: Vec<f64>,
    ax_rtol: Vec<f64>,
    ax_danger: Vec<f64>,
    ax_clat: Vec<u64>,
    ax_poscap: Vec<i64>,
    ax_invskew: Vec<f64>,
}

/// Build the three validated configs for a maker cell. `--qty` (`q`) is wired to
/// BOTH `QuoteConfig.quote_qty` and `InventoryConfig.quote_qty` so the
/// cross-config consistency check in `try_new` passes. `ack_timeout_ns` tracks
/// `cancel_latency_ns` (both stay within the 0-5000ms validated range together).
fn maker_cfgs(
    c: &MakerCell,
    q: Qty,
    staleness_ns: u64,
    sample_ns: u64,
    hold_ns: u64,
) -> (MakerQuoterConfig, FairValueConfig, InventoryConfig) {
    let qcfg = QuoteConfig {
        quote_offset_bps: c.qoff,
        entry_threshold_bps: c.ethr,
        exit_bps: c.exit,
        reprice_tol_bps: c.rtol,
        danger_bps: c.danger,
        cancel_latency_ns: c.clat,
        ack_timeout_ns: c.clat,
        quote_qty: q,
    };
    let mcfg = MakerQuoterConfig { quote: qcfg, hold_ns };
    let fvcfg = FairValueConfig { top_n: c.depth, window: c.win, sample_ns, staleness_ns };
    let invcfg = InventoryConfig { pos_cap: c.poscap, inv_skew_bps: c.invskew, quote_qty: q.raw() };
    (mcfg, fvcfg, invcfg)
}
#[allow(clippy::too_many_lines)]
fn run() -> Result<(), String> {
    let mut root = "/root/chd/fresh/ticks".to_string();
    let mut coin = "BTC".to_string();
    let mut symbol = "BTCUSDT".to_string();
    let mut dates: Vec<String> = Vec::new();
    let mut hours_arg = "all".to_string();
    let mut latency_ns: u64 = 300_000_000;
    let mut qty = 0.01_f64;
    let mut shuffle = false;
    let mut top = 16usize;
    let mut velgate = false;
    let mut velmin = 3.0_f64;
    let mut vellb = 4usize;
    let mut exitrevert = false;
    let mut exitbps = 2.0_f64;
    let mut zscore = false;
    let mut zk = 3.0_f64;
    let mut confirm = false;
    let mut confimb = 0.2_f64;
    let mut magsize = false;
    let mut magcap = 4.0_f64;
    let mut dumptrips: Option<String> = None;
    let mut fundgate = false;
    let mut fundmin = 0.00002_f64;
    let mut fundalign = false;
    let mut leadsym = String::new();
    let mut xlead = false;
    let mut xleadbps = 5.0_f64;
    let mut xleadlb = 4usize;
    let mut tp = 0.0_f64;
    let mut sl = 0.0_f64;
    let mut regime = false;
    let mut regimebps = 8.0_f64;
    let mut regimelb = 10usize;
    let mut latdist: Vec<u64> = Vec::new();

    let mut depths = vec![5usize];
    let mut thr_ax = vec![8.0, 10.0, 12.0];
    let mut win_ax = vec![500usize];
    let mut rev_ax = vec![true];
    let mut hold_ax = vec![120_000_000_000u64, 180_000_000_000];
    let mut cd_ax = vec![30_000_000_000u64];
    let mut lim_ax = vec![false];

    // MAKER mode + axes (Task 11). All default to a single point so a plain
    // `--maker` run is one cell; pass comma lists to sweep an axis.
    let mut maker = false;
    let mut maker_fee_bps: Option<f64> = None;
    let mut taker_fee_bps: Option<f64> = None;
    let mut qoff_ax = vec![2.0_f64];
    let mut ethr_ax = vec![16.0_f64];
    let mut exit_ax = vec![2.0_f64];
    let mut rtol_ax = vec![1.0_f64];
    let mut danger_ax = vec![40.0_f64];
    let mut clat_ax = vec![0u64];
    let mut poscap_ax: Vec<i64> = vec![Qty::from_f64(10.0).expect("const pos-cap default").raw()];
    let mut invskew_ax = vec![0.0_f64];
    let mut staleness_ns = DEFAULT_STALENESS_NS;
    let mut sample_ns = 500_000_000u64;
    let mut maker_hold_ns = 0u64;

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = || args.next().ok_or_else(|| format!("missing value after {a}"));
        match a.as_str() {
            "--root" => root = val()?,
            "--coin" => coin = val()?,
            "--symbol" => symbol = val()?,
            "--dates" => dates = val()?.split(',').map(|s| s.trim().to_string()).collect(),
            "--hours" => hours_arg = val()?,
            "--latency-ns" => latency_ns = val()?.parse().map_err(|e| format!("latency: {e}"))?,
            "--qty" => qty = val()?.parse().map_err(|e| format!("qty: {e}"))?,
            "--depths" => depths = parse_usizes(&val()?)?,
            "--thresholds" => thr_ax = parse_f64s(&val()?)?,
            "--windows" => win_ax = parse_usizes(&val()?)?,
            "--reversions" => rev_ax = parse_bools(&val()?)?,
            "--holds" => hold_ax = parse_durs(&val()?)?,
            "--cooldowns" => cd_ax = parse_durs(&val()?)?,
            "--limits" => lim_ax = parse_bools(&val()?)?,
            "--shuffle" => shuffle = true,
            "--top" => top = val()?.parse().map_err(|e| format!("top: {e}"))?,
            "--velgate" => velgate = true,
            "--velmin" => velmin = val()?.parse().map_err(|e| format!("velmin: {e}"))?,
            "--vellb" => vellb = val()?.parse().map_err(|e| format!("vellb: {e}"))?,
            "--exitrevert" => exitrevert = true,
            "--exitbps" => exitbps = val()?.parse().map_err(|e| format!("exitbps: {e}"))?,
            "--zscore" => zscore = true,
            "--zk" => zk = val()?.parse().map_err(|e| format!("zk: {e}"))?,
            "--confirm" => confirm = true,
            "--confimb" => confimb = val()?.parse().map_err(|e| format!("confimb: {e}"))?,
            "--magsize" => magsize = true,
            "--magcap" => magcap = val()?.parse().map_err(|e| format!("magcap: {e}"))?,
            "--dumptrips" => dumptrips = Some(val()?),
            "--fundgate" => fundgate = true,
            "--fundmin" => fundmin = val()?.parse().map_err(|e| format!("fundmin: {e}"))?,
            "--fundalign" => fundalign = true,
            "--leadsym" => leadsym = val()?,
            "--xlead" => xlead = true,
            "--xleadbps" => xleadbps = val()?.parse().map_err(|e| format!("xleadbps: {e}"))?,
            "--xleadlb" => xleadlb = val()?.parse().map_err(|e| format!("xleadlb: {e}"))?,
            "--tp" => tp = val()?.parse().map_err(|e| format!("tp: {e}"))?,
            "--sl" => sl = val()?.parse().map_err(|e| format!("sl: {e}"))?,
            "--regime" => regime = true,
            "--regimebps" => regimebps = val()?.parse().map_err(|e| format!("regimebps: {e}"))?,
            "--regimelb" => regimelb = val()?.parse().map_err(|e| format!("regimelb: {e}"))?,
            "--latdist" => { let v = val()?; latdist = parse_latdist(&v)?; }
            // ---- maker (Task 11) ----
            "--maker" => maker = true,
            "--maker-fee-bps" => maker_fee_bps = Some(val()?.parse().map_err(|e| format!("maker-fee-bps: {e}"))?),
            "--taker-fee-bps" => taker_fee_bps = Some(val()?.parse().map_err(|e| format!("taker-fee-bps: {e}"))?),
            "--quote-offset" => qoff_ax = parse_f64s(&val()?)?,
            "--entry-thr" => ethr_ax = parse_f64s(&val()?)?,
            "--maker-exit" => exit_ax = parse_f64s(&val()?)?,
            "--reprice-tol" => rtol_ax = parse_f64s(&val()?)?,
            "--danger" => danger_ax = parse_f64s(&val()?)?,
            "--cancel-lat" => clat_ax = parse_durs(&val()?)?,
            "--pos-cap" => poscap_ax = parse_poscaps(&val()?)?,
            "--inv-skew" => invskew_ax = parse_f64s(&val()?)?,
            "--staleness" => staleness_ns = parse_dur(&val()?)?,
            "--sample" => sample_ns = parse_dur(&val()?)?,
            "--hold" => maker_hold_ns = parse_dur(&val()?)?,
            other => return Err(format!("unknown arg {other}")),
        }
    }
    if dates.is_empty() {
        return Err("missing --dates (comma list)".into());
    }
    let hours: Vec<String> = if hours_arg == "all" {
        (0..24).map(|h| format!("{h:02}")).collect()
    } else {
        hours_arg.split(',').map(|s| s.trim().to_string()).collect()
    };
    let q = Qty::from_f64(qty).map_err(|e| format!("qty: {e}"))?;

    // Fee schedule (shared by both paths). Default = legacy() so the taker path
    // is byte-for-byte unchanged when no fee flag is passed. When set, the maker
    // and/or taker rate is overridden (Task 11/12: realistic positive maker fee).
    let fees = if maker_fee_bps.is_some() || taker_fee_bps.is_some() {
        let base = FeeSchedule::legacy();
        let tr = taker_fee_bps.map_or(base.taker_rate_raw, rate_raw_from_bps);
        let mr = maker_fee_bps.map_or(base.maker_rate_raw, rate_raw_from_bps);
        FeeSchedule::new(tr, mr)
    } else {
        FeeSchedule::legacy()
    };

    if maker {
        let cx = MakerRun {
            root, coin, symbol, dates, hours, latency_ns, q, fees, top, shuffle,
            latdist, staleness_ns, sample_ns, hold_ns: maker_hold_ns,
            maker_fee_set: maker_fee_bps.is_some(), taker_fee_set: taker_fee_bps.is_some(),
            ax_depth: depths, ax_win: win_ax, ax_qoff: qoff_ax, ax_ethr: ethr_ax,
            ax_exit: exit_ax, ax_rtol: rtol_ax, ax_danger: danger_ax, ax_clat: clat_ax,
            ax_poscap: poscap_ax, ax_invskew: invskew_ax,
        };
        return run_maker(&cx);
    }

    let mut cells: Vec<Cell> = Vec::new();
    for &d in &depths {
        for &th in &thr_ax {
            for &w in &win_ax {
                for &rv in &rev_ax {
                    for &h in &hold_ax {
                        for &cd in &cd_ax {
                            for &lim in &lim_ax {
                                cells.push(Cell {
                                    basis: BasisConfig { top_n: d, threshold_bps: th, window: w, sample_ns: 500_000_000, reversion: rv, shuffle, seed: 1, vel_gate: velgate, vel_min_bps: velmin, vel_lookback: vellb, exit_revert: exitrevert, exit_bps: exitbps, zscore, z_k: zk, confirm, confirm_imb: confimb, mag_size: magsize, mag_cap: magcap, fund_gate: fundgate, fund_min: fundmin, fund_align: fundalign, xlead, xlead_bps: xleadbps, xlead_lookback: xleadlb, regime, regime_bps: regimebps, regime_lookback: regimelb },
                                    managed: ManagedConfig { qty: q, hold_ns: h, cooldown_ns: cd, tp_bps: tp, sl_bps: sl, fill_timeout_ns: latency_ns.saturating_add(500_000_000).max(200_000_000), use_limit: lim },
                                    depth: d, thr: th, win: w, rev: rv, hold: h, lim,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    eprintln!("hunt: {} configs x {} days  latency={}ms  shuffle={}", cells.len(), dates.len(), latency_ns / 1_000_000, shuffle);
    let mut agg: Vec<Vec<(u64, f64)>> = vec![Vec::new(); cells.len()];
    let mut agg_notl: Vec<Vec<f64>> = vec![Vec::new(); cells.len()];
    let mut days_used = 0usize;
    for d in &dates {
        let cfg = FeedConfig {
            root: root.clone().into(),
            coin: coin.clone(),
            ref_symbols: symbol.split(',').map(|s| s.trim().to_string()).collect(),
            lead_symbols: if leadsym.is_empty() { Vec::new() } else { leadsym.split(',').map(|s| s.trim().to_string()).collect() },
            date: d.clone(),
            hours: hours.clone(),
            exec_latency_ns: 0,
            ref_latency_ns: 0,
        };
        let evs = match load_window(&cfg) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("  skip {d}: {e}");
                continue;
            }
        };
        if evs.is_empty() {
            eprintln!("  skip {d}: empty");
            continue;
        }
        let per: Vec<TripData> = cells
            .par_iter()
            .map(|c| {
                let sig = BasisSignal::new(c.basis);
                let strat = Managed::new(sig, c.managed);
                let mut eng = LagEngine::new(strat, LagConfig { order_latency_ns: latency_ns, cancel_latency_ns: 0, exec_book_levels: 20, fees });
                if !latdist.is_empty() { eng.set_latency_samples(latdist.clone()); }
                eng.run(evs.iter()).expect("monotonic stream");
                let r = eng.finish();
                (r.trip_returns, r.trip_notionals)
            })
            .collect();
        for (i, (r, nl)) in per.iter().enumerate() {
            agg[i].extend(r);
            agg_notl[i].extend(nl);
        }
        days_used += 1;
        eprintln!("  {d}: {} events", evs.len());
    }

    // EUR500 / 20x / 20% size / 5% daily-loss-limit (the standing policy = the default).
    let pcfg = PaperConfig::default();

    struct Row {
        n: usize,
        mean: f64,
        t: f64,
        win: f64,
        avg_w: f64,
        avg_l: f64,
        rr: f64,
        paper: f64,
        maxdd: f64,
        knobs: String,
    }
    let mut rows: Vec<Row> = Vec::new();
    for (i, c) in cells.iter().enumerate() {
        let rets: Vec<f64> = agg[i].iter().map(|x| x.1).collect();
        let n = rets.len();
        if n == 0 {
            continue;
        }
        let mean = rets.iter().sum::<f64>() / n as f64;
        let sd = if n >= 2 {
            (rets.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n as f64 - 1.0)).sqrt()
        } else {
            0.0
        };
        let t = if sd > 0.0 { mean / sd * (n as f64).sqrt() } else { 0.0 };
        let win = rets.iter().filter(|r| **r > 0.0).count() as f64 / n as f64 * 100.0;
        let wv: Vec<f64> = rets.iter().filter(|r| **r > 0.0).copied().collect();
        let lv: Vec<f64> = rets.iter().filter(|r| **r < 0.0).copied().collect();
        let avg_w = if wv.is_empty() { 0.0 } else { wv.iter().sum::<f64>() / wv.len() as f64 };
        let avg_l = if lv.is_empty() { 0.0 } else { lv.iter().sum::<f64>() / lv.len() as f64 };
        let rr = if avg_l < 0.0 { avg_w / -avg_l } else { 0.0 };
        let notl = &agg_notl[i];
        let mut trips: Vec<(u64, f64)> = if magsize && !notl.is_empty() {
            let mean = notl.iter().sum::<f64>() / notl.len() as f64;
            agg[i].iter().zip(notl.iter()).map(|((ts, r), &w)| (*ts, if mean > 0.0 { r * (w / mean) } else { *r })).collect()
        } else {
            agg[i].clone()
        };
        trips.sort_by_key(|x| x.0);
        let pr = paper_run(&trips, &pcfg);
        rows.push(Row {
            n, mean, t, win, avg_w, avg_l, rr, paper: pr.return_pct, maxdd: pr.max_drawdown_pct,
            knobs: format!("d={} thr={}bps win={} rev={} hold={} lim={}", c.depth, c.thr, c.win, c.rev, fmt_dur(c.hold), c.lim),
        });
    }
    rows.sort_by(|a, b| b.t.abs().partial_cmp(&a.t.abs()).unwrap_or(std::cmp::Ordering::Equal));

    println!("days used        {days_used}/{}", dates.len());
    println!("order latency    {}ms", latency_ns / 1_000_000);
    println!("mode             {}", if shuffle { "SHUFFLE control (direction randomized)" } else { "REAL basis-reversion" });
    println!("velocity gate    {}", if velgate { format!("ON (min {velmin}bps over {vellb} samples)") } else { "off".to_string() });
    println!("revert-exit      {}", if exitrevert { format!("ON (exit |dev|<={exitbps}bps)") } else { "off".to_string() });
    println!("z-score trigger  {}", if zscore { format!("ON (k={zk})") } else { "off".to_string() });
    println!("book confirm     {}", if confirm { format!("ON (imb>={confimb})") } else { "off".to_string() });
    println!("magnitude sizing {}", if magsize { format!("ON (cap {magcap}x)") } else { "off".to_string() });
    println!("funding gate     {}", if fundgate { format!("ON (|fund|>={fundmin})") } else { "off".to_string() });
    println!("funding align    {}", if fundalign { "ON".to_string() } else { "off".to_string() });
    println!("cross-asset lead {}", if xlead { format!("ON (lead={leadsym} |ret|>={xleadbps}bps/{xleadlb}smp)") } else { "off".to_string() });
    println!("stop / take     SL={sl}bps TP={tp}bps (0=off)");
    println!("regime filter   {}", if regime { format!("ON (skip fade vs |HL mom|>={regimebps}bps/{regimelb}smp)") } else { "off".to_string() });
    println!("latency dist     {}", if latdist.is_empty() { "off (fixed)".to_string() } else { format!("ON ({} samples)", latdist.len()) });
    println!("{:>6} {:>8} {:>6} {:>6} {:>8} {:>8} {:>5} {:>8} {:>7}  knobs", "n", "mean%", "t-stat", "win%", "avgW%", "avgL%", "RR", "paper%", "maxDD%");
    for r in rows.iter().take(top) {
        println!("{:>6} {:>8.4} {:>6.2} {:>5.1}% {:>8.4} {:>8.4} {:>5.2} {:>8.1} {:>7.1}  {}", r.n, r.mean, r.t, r.win, r.avg_w, r.avg_l, r.rr, r.paper, r.maxdd, r.knobs);
    }
    println!("(t-stat is the significance test for this sparse strategy; |t|>~2 ~ p<0.05)");
    if let Some(path) = &dumptrips {
        use std::io::Write;
        let mut f = std::fs::File::create(path).map_err(|e| format!("dumptrips: {e}"))?;
        for (ts, r) in &agg[0] {
            writeln!(f, "{ts},{r}").map_err(|e| format!("dumptrips write: {e}"))?;
        }
        eprintln!("dumped {} trips (cell 0) to {path}", agg[0].len());
    }
    Ok(())
}
/// Value of the primary swept axis for a maker cell (for knob-bite ordering).
fn maker_axis_value(c: &MakerCell, axis: &str) -> f64 {
    match axis {
        "entry-thr" => c.ethr,
        "quote-offset" => c.qoff,
        "danger" => c.danger,
        "reprice-tol" => c.rtol,
        "maker-exit" => c.exit,
        "cancel-lat" => c.clat as f64,
        "pos-cap" => c.poscap as f64,
        "inv-skew" => c.invskew,
        "windows" => c.win as f64,
        "depths" => c.depth as f64,
        _ => 0.0,
    }
}

/// Group key = every maker axis value EXCEPT the primary, so the knob-bite rule
/// compares sweep points that differ ONLY in the primary axis (Req 8.2).
fn maker_group_key(c: &MakerCell, primary: &str) -> String {
    let all = [
        ("entry-thr", c.ethr),
        ("quote-offset", c.qoff),
        ("danger", c.danger),
        ("reprice-tol", c.rtol),
        ("maker-exit", c.exit),
        ("cancel-lat", c.clat as f64),
        ("pos-cap", c.poscap as f64),
        ("inv-skew", c.invskew),
        ("windows", c.win as f64),
        ("depths", c.depth as f64),
    ];
    let mut s = String::new();
    for (name, v) in all {
        if name != primary {
            s.push_str(&format!("{name}={v};"));
        }
    }
    s
}

/// MAKER sweep path (Task 11): run the VALIDATED `MakerQuoter` across the
/// cartesian product of the maker axes, over each day (same day-in-memory +
/// rayon-per-cell + aggregate structure as the taker path), then report the
/// maker metrics (Req 8.6 / 10) and a knob-bite verdict per swept axis (Req
/// 8.1-8.5). Config errors are surfaced (named) before any run (Req 3.6).
#[allow(clippy::too_many_lines)]
fn run_maker(cx: &MakerRun) -> Result<(), String> {
    // ---- cartesian product of the maker axes into cells ----
    let mut cells: Vec<MakerCell> = Vec::new();
    for &depth in &cx.ax_depth {
        for &win in &cx.ax_win {
            for &qoff in &cx.ax_qoff {
                for &ethr in &cx.ax_ethr {
                    for &exit in &cx.ax_exit {
                        for &rtol in &cx.ax_rtol {
                            for &danger in &cx.ax_danger {
                                for &clat in &cx.ax_clat {
                                    for &poscap in &cx.ax_poscap {
                                        for &invskew in &cx.ax_invskew {
                                            cells.push(MakerCell { qoff, ethr, exit, rtol, danger, clat, poscap, invskew, depth, win });
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    if cells.is_empty() {
        return Err("maker: empty sweep (an axis had no values)".into());
    }

    // ---- validate EVERY cell up front (Req 3.6): a bad knob is reported,
    //      NAMED, before any run rather than panicking inside the parallel
    //      closure. `try_new` also enforces the quote_qty consistency check. ----
    for c in &cells {
        let (mcfg, fvcfg, invcfg) = maker_cfgs(c, cx.q, cx.staleness_ns, cx.sample_ns, cx.hold_ns);
        MakerQuoter::try_new(mcfg, fvcfg, invcfg).map_err(|e| {
            format!(
                "invalid maker config (off={} entry={} exit={} rtol={} danger={} cap={}): {e}",
                c.qoff, c.ethr, c.exit, c.rtol, c.danger, Qty::from_raw(c.poscap).to_f64()
            )
        })?;
    }

    let n = cells.len();
    eprintln!("maker-hunt: {n} cells x {} days  exec-latency={}ms  qty={}", cx.dates.len(), cx.latency_ns / 1_000_000, cx.q.to_f64());

    // ---- per-cell aggregates across days ----
    let mut agg_ret: Vec<Vec<(u64, f64)>> = vec![Vec::new(); n];
    let mut agg_notl: Vec<Vec<f64>> = vec![Vec::new(); n];
    let mut q_sub = vec![0u64; n];
    let mut q_fill = vec![0u64; n];
    let mut m_fill = vec![0u64; n];
    let mut rtrips = vec![0u64; n];
    let mut max_inv = vec![0i64; n];
    let mut max_dd = vec![0f64; n];
    let mut ts_sum = vec![0u64; n];
    let mut days_used = 0usize;

    for d in &cx.dates {
        let cfg = FeedConfig {
            root: cx.root.clone().into(),
            coin: cx.coin.clone(),
            ref_symbols: cx.symbol.split(',').map(|s| s.trim().to_string()).collect(),
            lead_symbols: Vec::new(),
            date: d.clone(),
            hours: cx.hours.clone(),
            exec_latency_ns: 0,
            ref_latency_ns: 0,
        };
        let evs = match load_window(&cfg) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("  skip {d}: {e}");
                continue;
            }
        };
        if evs.is_empty() {
            eprintln!("  skip {d}: empty");
            continue;
        }
        let per: Vec<MakerOut> = cells
            .par_iter()
            .map(|c| {
                let (mcfg, fvcfg, invcfg) = maker_cfgs(c, cx.q, cx.staleness_ns, cx.sample_ns, cx.hold_ns);
                let strat = MakerQuoter::try_new(mcfg, fvcfg, invcfg).expect("pre-validated maker cfg");
                let mut eng = LagEngine::new(
                    strat,
                    LagConfig { order_latency_ns: cx.latency_ns, cancel_latency_ns: c.clat, exec_book_levels: 20, fees: cx.fees },
                );
                if !cx.latdist.is_empty() {
                    eng.set_latency_samples(cx.latdist.clone());
                }
                eng.run(evs.iter()).expect("monotonic stream");
                let r = eng.finish();
                let tss = r.trip_returns.iter().fold(0u64, |a, &(t, _)| a.wrapping_add(t));
                MakerOut {
                    trip_returns: r.trip_returns,
                    trip_notionals: r.trip_notionals,
                    q_sub: r.quotes_submitted,
                    q_fill: r.quotes_filled,
                    maker_fills: r.maker_fills,
                    round_trips: r.round_trips,
                    max_inv: r.max_abs_inventory,
                    max_dd: r.max_drawdown_pct,
                    ts_sum: tss,
                }
            })
            .collect();
        for (i, o) in per.into_iter().enumerate() {
            agg_ret[i].extend(o.trip_returns);
            agg_notl[i].extend(o.trip_notionals);
            q_sub[i] += o.q_sub;
            q_fill[i] += o.q_fill;
            m_fill[i] += o.maker_fills;
            rtrips[i] += o.round_trips;
            if o.max_inv > max_inv[i] {
                max_inv[i] = o.max_inv;
            }
            if o.max_dd > max_dd[i] {
                max_dd[i] = o.max_dd;
            }
            ts_sum[i] = ts_sum[i].wrapping_add(o.ts_sum);
        }
        days_used += 1;
        eprintln!("  {d}: {} events", evs.len());
    }

    // EUR500 / 20x / 20% size / 5% daily-loss-limit (standing policy = default).
    let pcfg = PaperConfig::default();
    // Maker/taker fee actually in use (raw = bps * 1e4, since SCALE = 1e8 and
    // rate_raw = bps/1e4 * SCALE). Printed ALWAYS so the legacy maker REBATE can
    // never silently flatter a maker result.
    let maker_bps = cx.fees.maker_rate_raw as f64 / 1e4;
    let taker_bps = cx.fees.taker_rate_raw as f64 / 1e4;

    // ---- per-cell metrics + knob-bite fingerprints ----
    struct MRow {
        n: usize,
        mean: f64,
        t: f64,
        win: f64,
        rr: f64,
        paper: f64,
        maxdd: f64,
        fillr: f64,
        qsub: u64,
        qfill: u64,
        trips: u64,
        maxinv: f64,
        knobs: String,
    }
    let mut rows: Vec<MRow> = Vec::with_capacity(n);
    let mut fps: Vec<Fp> = Vec::with_capacity(n);
    let mut t_by_cell: Vec<f64> = Vec::with_capacity(n);
    let mut fill_by_cell: Vec<f64> = Vec::with_capacity(n);
    for i in 0..n {
        let rets: Vec<f64> = agg_ret[i].iter().map(|x| x.1).collect();
        let nn = rets.len();
        let mean = if nn > 0 { rets.iter().sum::<f64>() / nn as f64 } else { 0.0 };
        let sd = if nn >= 2 {
            (rets.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (nn as f64 - 1.0)).sqrt()
        } else {
            0.0
        };
        let t = if sd > 0.0 { mean / sd * (nn as f64).sqrt() } else { 0.0 };
        let win = if nn > 0 {
            rets.iter().filter(|r| **r > 0.0).count() as f64 / nn as f64 * 100.0
        } else {
            0.0
        };
        let wv: Vec<f64> = rets.iter().filter(|r| **r > 0.0).copied().collect();
        let lv: Vec<f64> = rets.iter().filter(|r| **r < 0.0).copied().collect();
        let avg_w = if wv.is_empty() { 0.0 } else { wv.iter().sum::<f64>() / wv.len() as f64 };
        let avg_l = if lv.is_empty() { 0.0 } else { lv.iter().sum::<f64>() / lv.len() as f64 };
        let rr = if avg_l < 0.0 { avg_w / -avg_l } else { 0.0 };
        // agg_notl is aggregated for completeness (entry notionals) but maker
        // sizing is fixed, so the paper run uses the raw per-trip returns.
        let _ = &agg_notl[i];
        let mut trips_v = agg_ret[i].clone();
        trips_v.sort_by_key(|x| x.0);
        let pr = paper_run(&trips_v, &pcfg);
        let fillr = if q_sub[i] == 0 {
            0.0
        } else {
            (q_fill[i] as f64 / q_sub[i] as f64 * 100.0).clamp(0.0, 100.0)
        };
        let maxinv = Qty::from_raw(max_inv[i]).to_f64();
        let c = &cells[i];
        fps.push(Fp { q_sub: q_sub[i], round_trips: rtrips[i], maker_fills: m_fill[i], ts_sum: ts_sum[i] });
        t_by_cell.push(t);
        fill_by_cell.push(fillr);
        rows.push(MRow {
            n: nn,
            mean,
            t,
            win,
            rr,
            paper: pr.return_pct,
            maxdd: max_dd[i],
            fillr,
            qsub: q_sub[i],
            qfill: q_fill[i],
            trips: rtrips[i],
            maxinv,
            knobs: format!(
                "off={} ent={} exit={} rtol={} dgr={} clat={}ms cap={} skew={} d={} win={}",
                c.qoff, c.ethr, c.exit, c.rtol, c.danger, c.clat / 1_000_000, Qty::from_raw(c.poscap).to_f64(), c.invskew, c.depth, c.win
            ),
        });
    }

    // Print the table sorted by |t|; keep cell order for the knob-bite section.
    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by(|&a, &b| rows[b].t.abs().partial_cmp(&rows[a].t.abs()).unwrap_or(std::cmp::Ordering::Equal));

    println!("days used        {days_used}/{}", cx.dates.len());
    println!(
        "exec latency     {}ms{}",
        cx.latency_ns / 1_000_000,
        if cx.latdist.is_empty() { String::new() } else { format!(" (+dist {} samples)", cx.latdist.len()) }
    );
    println!(
        "maker fee        {maker_bps:+.3} bps {}",
        if cx.maker_fee_set { "(OVERRIDE)" } else { "(legacy REBATE - FLATTERS maker; set --maker-fee-bps for a realistic HL fee)" }
    );
    println!("taker fee        {taker_bps:+.3} bps {}", if cx.taker_fee_set { "(override)" } else { "(legacy)" });
    println!("staleness        {}ms   sample {}ms   hold {}", cx.staleness_ns / 1_000_000, cx.sample_ns / 1_000_000, fmt_dur(cx.hold_ns));
    if cx.shuffle {
        println!("note             --shuffle has no effect in maker mode (ignored)");
    }
    println!("strategy         MAKER (resting-limit MakerQuoter; entries=Place, exits=taker Market)");
    println!(
        "{:>6} {:>8} {:>6} {:>6} {:>5} {:>8} {:>7} {:>6} {:>7} {:>7} {:>6} {:>9}  knobs",
        "n", "mean%", "t-stat", "win%", "RR", "paper%", "maxDD%", "fill%", "quotes", "fills", "trips", "maxInv"
    );
    for &i in order.iter().take(cx.top) {
        let r = &rows[i];
        println!(
            "{:>6} {:>8.4} {:>6.2} {:>5.1}% {:>5.2} {:>8.1} {:>7.2} {:>5.1}% {:>7} {:>7} {:>6} {:>9.4}  {}",
            r.n, r.mean, r.t, r.win, r.rr, r.paper, r.maxdd, r.fillr, r.qsub, r.qfill, r.trips, r.maxinv, r.knobs
        );
    }
    println!("(t-stat is the per-trade significance test; |t|>~2 ~ p<0.05. fill% = filled/submitted quotes.)");

    // ---- KNOB-BITE (Req 8.1-8.5) ----
    // Primary swept axis = the first axis (declared priority) with >1 value.
    let axes: [(&str, usize); 10] = [
        ("entry-thr", cx.ax_ethr.len()),
        ("quote-offset", cx.ax_qoff.len()),
        ("danger", cx.ax_danger.len()),
        ("reprice-tol", cx.ax_rtol.len()),
        ("maker-exit", cx.ax_exit.len()),
        ("cancel-lat", cx.ax_clat.len()),
        ("pos-cap", cx.ax_poscap.len()),
        ("inv-skew", cx.ax_invskew.len()),
        ("windows", cx.ax_win.len()),
        ("depths", cx.ax_depth.len()),
    ];
    match axes.iter().find(|(_, l)| *l > 1) {
        None => {
            println!("\nknob-bite: cannot establish (<2 sweep points)");
        }
        Some(&(primary, _)) => {
            use std::collections::BTreeMap;
            // Group cells that differ ONLY in the primary axis, compare adjacent
            // points' (quotes, fills, trips, trip-timestamp-sum) signatures.
            let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
            for (i, c) in cells.iter().enumerate() {
                groups.entry(maker_group_key(c, primary)).or_default().push(i);
            }
            println!("\nKNOB-BITE on `{primary}` (Req 8.1-8.5): a `no edge` verdict counts ONLY if the dial moved trades");
            println!("{:>14} {:>7} {:>7} {:>6} {:>7} {:>6}  verdict", primary, "quotes", "fills", "trips", "t", "fill%");
            for idxs in groups.values() {
                let mut sorted = idxs.clone();
                sorted.sort_by(|&a, &b| {
                    maker_axis_value(&cells[a], primary)
                        .partial_cmp(&maker_axis_value(&cells[b], primary))
                        .unwrap_or(std::cmp::Ordering::Equal)
                });
                for (k, &ci) in sorted.iter().enumerate() {
                    let pv = maker_axis_value(&cells[ci], primary);
                    let edge = t_by_cell[ci].abs() >= 2.0;
                    let verdict = if k == 0 {
                        if edge { "EDGE (|t|>=2) [baseline pt]" } else { "no edge [baseline pt]" }
                    } else if fps[ci] == fps[sorted[k - 1]] {
                        // Identical trade behavior vs the adjacent lower point.
                        if edge { "EDGE but dial INERT (suspect)" } else { "INERT (no knob bite)" }
                    } else if edge {
                        "EDGE (|t|>=2)"
                    } else {
                        "no edge (VALID - dial moved)"
                    };
                    println!(
                        "{:>14} {:>7} {:>7} {:>6} {:>7.2} {:>5.1}%  {}",
                        pv, q_sub[ci], q_fill[ci], rtrips[ci], t_by_cell[ci], fill_by_cell[ci], verdict
                    );
                }
            }
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}