//! `forge-nulledge` - run the seeded coinflip over a *.forge stream and report
//! the net edge. The verdict: in an honest engine the coinflip MUST lose.
//!
//! Usage: forge-nulledge <path.forge> [qty] [hold_ns] [cooldown_ns] [seed] [latency_ns]

use std::process::ExitCode;

use forge_core::Qty;
use forge_data::ForgeReader;
use forge_sim::{money_to_f64, Coinflip, FeeSchedule, SimConfig, SimEngine};

fn arg_f64(a: Option<String>, default: f64) -> Result<f64, String> {
    match a {
        Some(s) => s.parse().map_err(|e| format!("bad number: {e}")),
        None => Ok(default),
    }
}
fn arg_u(a: Option<String>, default: u64) -> Result<u64, String> {
    match a {
        Some(s) => s.parse().map_err(|e| format!("bad number: {e}")),
        None => Ok(default),
    }
}

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: forge-nulledge <path.forge> [qty] [hold_ns] [cooldown_ns] [seed] [latency_ns]")?;
    let qty_f = arg_f64(args.next(), 0.01)?;
    let hold_ns = arg_u(args.next(), 5_000_000_000)?;
    let cooldown_ns = arg_u(args.next(), 1_000_000_000)?;
    let seed = arg_u(args.next(), 1)?;
    let latency_ns = arg_u(args.next(), 0)?;

    let reader = ForgeReader::open(&path).map_err(|e| format!("open {path}: {e}"))?;
    let qty = Qty::from_f64(qty_f).map_err(|e| format!("bad qty: {e}"))?;
    let cfg = SimConfig { order_latency_ns: latency_ns, book_max_levels: 20, fees: FeeSchedule::legacy() };
    let mut eng = SimEngine::new(Coinflip::new(seed, qty, hold_ns, cooldown_ns), cfg);
    for rec in reader.records() {
        let ev = rec.to_event().map_err(|e| format!("decode: {e}"))?;
        eng.step(&ev).map_err(|e| format!("step: {e}"))?;
    }
    let r = eng.finish();

    println!("file            {path}");
    println!("coinflip        qty={qty_f} hold_ns={hold_ns} cooldown_ns={cooldown_ns} seed={seed} latency_ns={latency_ns}");
    println!("events          {}", r.events);
    println!("round trips     {}", r.round_trips);
    println!("orders filled   {}", r.orders_filled);
    println!("orders rejected {}", r.orders_rejected);
    println!("realized gross  {:.4}", money_to_f64(r.realized));
    println!("fees paid       {:.4}", money_to_f64(r.fees));
    println!("NET P&L         {:.4}", money_to_f64(r.net_pnl));
    if r.net_pnl < 0 {
        println!("VERDICT         NULL-EDGE OK - the coinflip lost, as an honest engine demands");
        Ok(())
    } else {
        Err(format!(
            "VERDICT  NULL-EDGE FAIL - coinflip profited ({:.4}); the engine is lying",
            money_to_f64(r.net_pnl)
        ))
    }
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