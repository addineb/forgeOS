//! `forge-sim-replay` - run the deterministic replay engine over a *.forge file
//! with the no-op strategy, printing the report. Run twice to confirm the
//! determinism hash is stable. Validates the Phase 1 replay core on real data.
//!
//! Usage: forge-sim-replay <path.forge> [order_latency_ns] [book_max_levels]

use std::process::ExitCode;

use forge_data::ForgeReader;
use forge_sim::{FeeSchedule, NoopStrategy, SimConfig, SimEngine};

fn run() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let path = args
        .next()
        .ok_or("usage: forge-sim-replay <path.forge> [order_latency_ns] [book_max_levels]")?;
    let order_latency_ns: u64 = match args.next() {
        Some(s) => s.parse().map_err(|e| format!("bad order_latency_ns: {e}"))?,
        None => 2_000_000,
    };
    let book_max_levels: usize = match args.next() {
        Some(s) => s.parse().map_err(|e| format!("bad book_max_levels: {e}"))?,
        None => 20,
    };

    let reader = ForgeReader::open(&path).map_err(|e| format!("open {path}: {e}"))?;
    let cfg = SimConfig { order_latency_ns, book_max_levels, fees: FeeSchedule::legacy() };
    let mut engine = SimEngine::new(NoopStrategy, cfg);
    for (decoded, rec) in reader.records().iter().enumerate() {
        let ev = rec.to_event().map_err(|e| format!("decode at #{decoded}: {e}"))?;
        engine.step(&ev).map_err(|e| format!("step at #{decoded}: {e}"))?;
    }
    let report = engine.finish();

    println!("file              {path}");
    println!("order_latency_ns  {order_latency_ns}");
    println!("book_max_levels   {book_max_levels}");
    println!("events            {}", report.events);
    println!("last_ts (ns)      {}", report.last_ts);
    println!("orders submitted  {}", report.orders_submitted);
    println!("orders reached    {}", report.orders_reached);
    println!("det_hash          {:#018x}", report.det_hash);
    println!("book_hash         {:#018x}", report.book_hash);
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