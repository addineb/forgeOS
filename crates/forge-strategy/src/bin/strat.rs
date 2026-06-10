//! `forge-strat` - run the OFI-momentum strategy over a *.forge stream and
//! report the edge. Use --shuffle to run the random-direction control.
//!
//! Usage: forge-strat <path.forge> [--window N] [--threshold X] [--qty Q]
//!        [--hold H] [--cooldown C] [--tp-bps T] [--sl-bps S] [--limit] [--shuffle]

use std::process::ExitCode;

use forge_core::Qty;
use forge_data::ForgeReader;
use forge_sim::{money_to_f64, FeeSchedule, SimConfig, SimEngine};
use forge_strategy::{MomentumConfig, OfiMomentum, Signal};

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let path = args.next().ok_or("usage: forge-strat <path.forge> [flags]")?;

    let mut c = MomentumConfig::default();
    let mut latency_ns: u64 = 2_000_000;
    while let Some(a) = args.next() {
        let mut val = || args.next().ok_or_else(|| format!("missing value after {a}"));
        match a.as_str() {
            "--window" => c.ofi_window = val()?.parse().map_err(|e| format!("window: {e}"))?,
            "--threshold" => c.threshold = val()?.parse().map_err(|e| format!("threshold: {e}"))?,
            "--qty" => c.qty = Qty::from_f64(val()?.parse().map_err(|e| format!("qty: {e}"))?).map_err(|e| format!("qty: {e}"))?,
            "--hold" => c.hold = val()?.parse().map_err(|e| format!("hold: {e}"))?,
            "--cooldown" => c.cooldown = val()?.parse().map_err(|e| format!("cooldown: {e}"))?,
            "--tp-bps" => c.tp_bps = val()?.parse().map_err(|e| format!("tp: {e}"))?,
            "--sl-bps" => c.sl_bps = val()?.parse().map_err(|e| format!("sl: {e}"))?,
            "--seed" => c.seed = val()?.parse().map_err(|e| format!("seed: {e}"))?,
            "--latency-ns" => latency_ns = val()?.parse().map_err(|e| format!("latency: {e}"))?,
            "--limit" => c.use_limit = true,
            "--shuffle" => c.signal = Signal::Shuffled,
            other => return Err(format!("unknown flag {other}")),
        }
    }

    let reader = ForgeReader::open(&path).map_err(|e| format!("open {path}: {e}"))?;
    let sim_cfg = SimConfig { order_latency_ns: latency_ns, book_max_levels: 20, fees: FeeSchedule::legacy() };
    let mut eng = SimEngine::new(OfiMomentum::new(c), sim_cfg);
    for rec in reader.records() {
        let ev = rec.to_event().map_err(|e| format!("decode: {e}"))?;
        eng.step(&ev).map_err(|e| format!("step: {e}"))?;
    }
    let tp = eng.account().trip_pnls();
    let wins = tp.iter().filter(|&&v| v > 0).count();
    let losses = tp.iter().filter(|&&v| v < 0).count();
    let n_trips = tp.len();
    let avg_trip = if n_trips > 0 { money_to_f64(tp.iter().sum::<i128>()) / n_trips as f64 } else { 0.0 };
    let win_rate = if n_trips > 0 { wins as f64 / n_trips as f64 * 100.0 } else { 0.0 };
    let r = eng.finish();

    println!("file            {path}");
    println!("signal          {:?}", c.signal);
    println!("knobs           window={} thr={} hold={} cd={} tp={} sl={} limit={}", c.ofi_window, c.threshold, c.hold, c.cooldown, c.tp_bps, c.sl_bps, c.use_limit);
    println!("events          {}", r.events);
    println!("round trips     {}", r.round_trips);
    println!("orders filled   {} (maker {})", r.orders_filled, r.maker_fills);
    println!("orders rejected {}", r.orders_rejected);
    println!("realized gross  {:.4}", money_to_f64(r.realized));
    println!("fees paid       {:.4}", money_to_f64(r.fees));
    println!("NET P&L         {:.4}", money_to_f64(r.net_pnl));
    println!("trips {n_trips}  wins {wins}  losses {losses}  win% {win_rate:.2}  avg/trip {avg_trip:.4}");
    Ok(())
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("{msg}");
            ExitCode::FAILURE
        }
    }
}