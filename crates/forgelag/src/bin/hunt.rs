//! `lag-hunt` - sweep basis-reversion variants across many days at a given
//! ORDER LATENCY. Judged on the PER-TRADE t-stat (the correct lens for a sparse
//! minutes-hold strategy) + an EUR-account paper result + a shuffle control.
//! Memory-efficient: one day in memory at a time; configs run in parallel
//! within each day, then the window is dropped.
//!
//!   lag-hunt --root /root/chd/fresh/ticks --dates 2026-05-20,... \
//!     --latency-ns 300000000 --depths 5 --thresholds 8,10,12 --windows 500 \
//!     --reversions true --holds 2m,3m --cooldowns 30s [--shuffle]

use std::process::ExitCode;

use forge_core::Qty;
use forge_metrics::{paper_run, PaperConfig};
use rayon::prelude::*;

use forgelag::{
    load_window, BasisConfig, BasisSignal, FeeSchedule, FeedConfig, LagConfig, LagEngine, Managed,
    ManagedConfig,
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
fn fmt_dur(ns: u64) -> String {
    if ns >= 60_000_000_000 && ns.is_multiple_of(60_000_000_000) { format!("{}m", ns / 60_000_000_000) }
    else if ns >= 1_000_000_000 && ns.is_multiple_of(1_000_000_000) { format!("{}s", ns / 1_000_000_000) }
    else { format!("{}ms", ns / 1_000_000) }
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

    let mut depths = vec![5usize];
    let mut thr_ax = vec![8.0, 10.0, 12.0];
    let mut win_ax = vec![500usize];
    let mut rev_ax = vec![true];
    let mut hold_ax = vec![120_000_000_000u64, 180_000_000_000];
    let mut cd_ax = vec![30_000_000_000u64];
    let mut lim_ax = vec![false];

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

    let mut cells: Vec<Cell> = Vec::new();
    for &d in &depths {
        for &th in &thr_ax {
            for &w in &win_ax {
                for &rv in &rev_ax {
                    for &h in &hold_ax {
                        for &cd in &cd_ax {
                            for &lim in &lim_ax {
                                cells.push(Cell {
                                    basis: BasisConfig { top_n: d, threshold_bps: th, window: w, sample_ns: 500_000_000, reversion: rv, shuffle, seed: 1, vel_gate: velgate, vel_min_bps: velmin, vel_lookback: vellb, exit_revert: exitrevert, exit_bps: exitbps, zscore, z_k: zk, confirm, confirm_imb: confimb, mag_size: magsize, mag_cap: magcap, fund_gate: fundgate, fund_min: fundmin, fund_align: fundalign, xlead, xlead_bps: xleadbps, xlead_lookback: xleadlb },
                                    managed: ManagedConfig { qty: q, hold_ns: h, cooldown_ns: cd, tp_bps: 0.0, sl_bps: 0.0, fill_timeout_ns: latency_ns.saturating_add(500_000_000).max(200_000_000), use_limit: lim },
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
                let mut eng = LagEngine::new(strat, LagConfig { order_latency_ns: latency_ns, exec_book_levels: 20, fees: FeeSchedule::legacy() });
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

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}