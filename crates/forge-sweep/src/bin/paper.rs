//! `forge-paper` - the pre-live gate. Run ONE promoted config over windows and
//! simulate a real EUR account (default 500 / 20x / 10% size / 5% daily-loss-
//! limit) to see if it survives + profits the money you would actually risk.
//!
//!   forge-paper w1.forge w2.forge --strategy ofi --window 100 --threshold 3 \
//!     --hold 5m --cooldown 30s --tp-bps 0 --sl-bps 0 \
//!     --balance 500 --leverage 20 --risk-pct 0.10 --daily-limit 0.05

use std::process::ExitCode;

use forge_core::{Event, Qty};
use forge_data::ForgeReader;
use forge_metrics::{paper_run, PaperConfig};
use forge_sim::{money_to_f64, FeeSchedule, SimConfig, SimEngine};
use forge_strategy::{BasisBot, BasisConfig, ImbalanceConfig, MomentumConfig, ObiBot, OfiMomentum, RegimeFilter, Signal};

/// Parse a duration like `30s`, `5m`, `2h`, `250ms` (bare = ns) into nanoseconds.
fn parse_dur(s: &str) -> Result<u64, String> {
    let s = s.trim();
    let (num, mult): (&str, u64) = if let Some(p) = s.strip_suffix("ms") {
        (p, 1_000_000)
    } else if let Some(p) = s.strip_suffix("us") {
        (p, 1_000)
    } else if let Some(p) = s.strip_suffix("ns") {
        (p, 1)
    } else if let Some(p) = s.strip_suffix('h') {
        (p, 3_600_000_000_000)
    } else if let Some(p) = s.strip_suffix('m') {
        (p, 60_000_000_000)
    } else if let Some(p) = s.strip_suffix('s') {
        (p, 1_000_000_000)
    } else {
        (s, 1)
    };
    let v: f64 = num.trim().parse().map_err(|e| format!("bad duration `{s}`: {e}"))?;
    if !v.is_finite() || v < 0.0 {
        return Err(format!("bad duration `{s}`"));
    }
    Ok((v * mult as f64) as u64)
}

fn load_window(path: &str) -> Result<Vec<Event>, String> {
    let reader = ForgeReader::open(path).map_err(|e| format!("open {path}: {e}"))?;
    let mut evs = Vec::with_capacity(reader.len());
    for rec in reader.records() {
        evs.push(rec.to_event().map_err(|e| format!("decode {path}: {e}"))?);
    }
    Ok(evs)
}

/// Run a strategy over the windows and return its per-trip (close_ts, return%).
fn collect_trips<S: forge_sim::Strategy, F: Fn() -> S>(
    windows: &[Vec<Event>],
    make: F,
    latency_ns: u64,
) -> Vec<(u64, f64)> {
    let mut trips = Vec::new();
    for w in windows {
        let cfg = SimConfig { order_latency_ns: latency_ns, book_max_levels: 20, fees: FeeSchedule::legacy() };
        let mut eng = SimEngine::new(make(), cfg);
        eng.run(w.iter()).expect("monotonic stream");
        let acct = eng.account();
        let tp = acct.trip_pnls();
        let tn = acct.trip_notionals();
        let ts = acct.trip_close_ts();
        for i in 0..tp.len() {
            if tn[i] > 0 {
                trips.push((ts[i], money_to_f64(tp[i]) / money_to_f64(tn[i]) * 100.0));
            }
        }
    }
    trips
}

#[allow(clippy::too_many_lines)]
fn run() -> Result<(), String> {
    let mut paths: Vec<String> = Vec::new();
    let mut strategy = "ofi".to_string();
    let mut latency_ns: u64 = 2_000_000;
    let mut qty = 0.01_f64;

    // strategy knobs (single values)
    let mut window = 100usize;
    let mut topn = 5usize;
    let mut threshold = 3.0f64;
    let mut reversion = false;
    let mut hold_ns: u64 = 300_000_000_000;   // 5m
    let mut cooldown_ns: u64 = 30_000_000_000; // 30s
    let mut tp_bps = 0.0f64;
    let mut sl_bps = 0.0f64;
    let mut use_limit = false;

    // account knobs
    let mut pcfg = PaperConfig::default();

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = || args.next().ok_or_else(|| format!("missing value after {a}"));
        match a.as_str() {
            "--strategy" => strategy = val()?,
            "--latency-ns" => latency_ns = val()?.parse().map_err(|e| format!("{e}"))?,
            "--qty" => qty = val()?.parse().map_err(|e| format!("{e}"))?,
            "--window" => window = val()?.parse().map_err(|e| format!("{e}"))?,
            "--topn" => topn = val()?.parse().map_err(|e| format!("{e}"))?,
            "--threshold" => threshold = val()?.parse().map_err(|e| format!("{e}"))?,
            "--reversion" => reversion = val()?.parse().map_err(|e| format!("{e}"))?,
            "--hold" => hold_ns = parse_dur(&val()?)?,
            "--cooldown" => cooldown_ns = parse_dur(&val()?)?,
            "--tp-bps" => tp_bps = val()?.parse().map_err(|e| format!("{e}"))?,
            "--sl-bps" => sl_bps = val()?.parse().map_err(|e| format!("{e}"))?,
            "--limit" => use_limit = true,
            "--balance" => pcfg.start_balance = val()?.parse().map_err(|e| format!("{e}"))?,
            "--leverage" => pcfg.leverage = val()?.parse().map_err(|e| format!("{e}"))?,
            "--risk-pct" => pcfg.risk_pct = val()?.parse().map_err(|e| format!("{e}"))?,
            "--daily-limit" => pcfg.daily_loss_limit_pct = val()?.parse().map_err(|e| format!("{e}"))?,
            s if s.starts_with("--") => return Err(format!("unknown flag {s}")),
            s => paths.push(s.to_string()),
        }
    }
    if paths.is_empty() {
        return Err("usage: forge-paper <window.forge>... --strategy ofi|wall [knobs] [account flags]".into());
    }

    eprintln!("loading {} window(s)...", paths.len());
    let mut windows = Vec::new();
    for p in &paths {
        windows.push(load_window(p)?);
    }
    let q = Qty::from_f64(qty).map_err(|e| format!("qty: {e}"))?;

    let trips = match strategy.as_str() {
        "ofi" => {
            let c = MomentumConfig {
                ofi_window: window, threshold, qty: q, hold_ns, cooldown_ns, tp_bps, sl_bps,
                use_limit, signal: Signal::Real, seed: 1, fill_timeout_ns: 200_000_000, regime_filter: RegimeFilter::Any,
            };
            collect_trips(&windows, || OfiMomentum::new(c), latency_ns)
        }
        "wall" => {
            let c = ImbalanceConfig {
                top_n: topn, threshold, reversion, qty: q, hold_ns, cooldown_ns, tp_bps, sl_bps,
                use_limit, signal: Signal::Real, seed: 1, fill_timeout_ns: 200_000_000, regime_filter: RegimeFilter::Any,
            };
            collect_trips(&windows, || ObiBot::new(c), latency_ns)
        }
        "basis" => {
            let c = BasisConfig {
                top_n: topn, threshold_bps: threshold, window, sample_ns: 500_000_000, reversion,
                qty: q, hold_ns, cooldown_ns, tp_bps, sl_bps, use_limit, signal: Signal::Real,
                seed: 1, fill_timeout_ns: latency_ns.saturating_add(500_000_000).max(200_000_000), regime_filter: RegimeFilter::Any,
            };
            collect_trips(&windows, || BasisBot::new(c), latency_ns)
        }
        other => return Err(format!("unknown --strategy `{other}` (ofi|wall|basis)")),
    };

    // Per-trade significance (the appropriate test for a sparse, minutes-hold
    // strategy; the equity-bucket DSR gate reads ~0 here because the curve is
    // flat ~95% of the time). t-stat = mean/std * sqrt(n); |t| > ~2 ~ p<0.05.
    let rets: Vec<f64> = trips.iter().map(|t| t.1).collect();
    if rets.len() >= 2 {
        let n = rets.len() as f64;
        let mean = rets.iter().sum::<f64>() / n;
        let var = rets.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n - 1.0);
        let sd = var.sqrt();
        let tstat = if sd > 0.0 { mean / sd * n.sqrt() } else { 0.0 };
        let trade_sharpe = if sd > 0.0 { mean / sd } else { 0.0 };
        let wins = rets.iter().filter(|r| **r > 0.0).count();
        println!(
            "per-trade       n={} mean={:.4}% std={:.4}% t-stat={:.2} trade-sharpe={:.3} win={:.1}%",
            rets.len(), mean, sd, tstat, trade_sharpe, wins as f64 / n * 100.0
        );
    }

    let res = paper_run(&trips, &pcfg);

    println!("strategy        {strategy}");
    println!("account         start {:.2}  leverage {}x  size {:.0}%  daily-limit {:.0}%",
        pcfg.start_balance, pcfg.leverage, pcfg.risk_pct * 100.0, pcfg.daily_loss_limit_pct * 100.0);
    println!("trips           {} taken / {} total", res.trips_taken, trips.len());
    println!("ending balance  {:.2}", res.end);
    println!("return          {:.2}%", res.return_pct);
    println!("max drawdown    {:.2}%", res.max_drawdown_pct);
    println!("min balance     {:.2}", res.min_balance);
    println!("halted days     {}", res.halted_days);
    println!("ruined          {}", res.ruined);

    let pass = res.return_pct > 0.0 && !res.ruined;
    if pass {
        println!("PAPER GATE      PASS (profitable + survived; drawdown is reported, not gated)");
        Ok(())
    } else {
        println!("PAPER GATE      FAIL");
        Err("paper gate not cleared".into())
    }
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