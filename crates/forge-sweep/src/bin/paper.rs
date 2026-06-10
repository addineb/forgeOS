//! `forge-paper` - the pre-live gate. Run ONE promoted config over windows and
//! simulate a real EUR account (default 500 / 20x / 10% size / 5% daily-loss-
//! limit) to see if it survives + profits the money you would actually risk.
//!
//!   forge-paper w1.forge w2.forge --strategy ofi --window 100 --threshold 3 \
//!     --hold 60000 --cooldown 1000 --tp-bps 0 --sl-bps 0 \
//!     --balance 500 --leverage 20 --risk-pct 0.10 --daily-limit 0.05

use std::process::ExitCode;

use forge_core::{Event, Qty};
use forge_data::ForgeReader;
use forge_metrics::{paper_run, PaperConfig};
use forge_sim::{money_to_f64, FeeSchedule, SimConfig, SimEngine};
use forge_strategy::{ImbalanceConfig, MomentumConfig, ObiBot, OfiMomentum, Signal};

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
    let mut hold = 60_000u32;
    let mut cooldown = 1_000u32;
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
            "--hold" => hold = val()?.parse().map_err(|e| format!("{e}"))?,
            "--cooldown" => cooldown = val()?.parse().map_err(|e| format!("{e}"))?,
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
                ofi_window: window, threshold, qty: q, hold, cooldown, tp_bps, sl_bps,
                use_limit, signal: Signal::Real, seed: 1, fill_timeout_ns: 200_000_000,
            };
            collect_trips(&windows, || OfiMomentum::new(c), latency_ns)
        }
        "wall" => {
            let c = ImbalanceConfig {
                top_n: topn, threshold, reversion, qty: q, hold, cooldown, tp_bps, sl_bps,
                use_limit, signal: Signal::Real, seed: 1, fill_timeout_ns: 200_000_000,
            };
            collect_trips(&windows, || ObiBot::new(c), latency_ns)
        }
        other => return Err(format!("unknown --strategy `{other}` (ofi|wall)")),
    };

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

    let pass = res.return_pct > 0.0 && !res.ruined && res.max_drawdown_pct < 50.0;
    if pass {
        println!("PAPER GATE      PASS (profitable, survived, drawdown < 50%)");
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