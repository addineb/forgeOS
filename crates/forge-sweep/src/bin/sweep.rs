//! `forge-sweep` - sweep OFI-momentum configs over one or more *.forge windows
//! and print the promote/park/retire scorecard. Run detached in tmux for big
//! grids (see tools/sweep.sh).
//!
//! Usage: forge-sweep <window1.forge> [window2.forge ...] [--shuffle]
//!        [--sample-ns N] [--latency-ns N]

use std::process::ExitCode;

use forge_core::{Event, Qty};
use forge_data::ForgeReader;
use forge_strategy::Signal;
use forge_sweep::{expand, run_sweep, GridSpec, Thresholds, Verdict};

fn load_window(path: &str) -> Result<Vec<Event>, String> {
    let reader = ForgeReader::open(path).map_err(|e| format!("open {path}: {e}"))?;
    let mut evs = Vec::with_capacity(reader.len());
    for rec in reader.records() {
        evs.push(rec.to_event().map_err(|e| format!("decode {path}: {e}"))?);
    }
    Ok(evs)
}

fn run() -> Result<(), String> {
    let mut paths: Vec<String> = Vec::new();
    let mut signal = Signal::Real;
    let mut sample_ns: u64 = 60_000_000_000; // 60s buckets
    let mut latency_ns: u64 = 2_000_000;

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--shuffle" => signal = Signal::Shuffled,
            "--sample-ns" => {
                sample_ns = args.next().ok_or("missing --sample-ns")?.parse().map_err(|e| format!("sample-ns: {e}"))?;
            }
            "--latency-ns" => {
                latency_ns = args.next().ok_or("missing --latency-ns")?.parse().map_err(|e| format!("latency-ns: {e}"))?;
            }
            s if s.starts_with("--") => return Err(format!("unknown flag {s}")),
            s => paths.push(s.to_string()),
        }
    }
    if paths.is_empty() {
        return Err("usage: forge-sweep <window.forge> [more...] [--shuffle] [--sample-ns N] [--latency-ns N]".into());
    }

    eprintln!("loading {} window(s)...", paths.len());
    let mut windows = Vec::new();
    for p in &paths {
        let w = load_window(p)?;
        eprintln!("  {p}: {} events", w.len());
        windows.push(w);
    }

    let spec = GridSpec {
        ofi_window: vec![20, 50, 100],
        threshold: vec![1.0, 1.5, 2.5],
        qty: Qty::from_f64(0.01).unwrap(),
        hold: vec![100, 300],
        cooldown: vec![50],
        tp_bps: vec![0.0, 8.0],
        sl_bps: vec![0.0, 8.0],
        use_limit: vec![false],
        signal,
        seed: 1,
        fill_timeout_ns: 200_000_000,
    };
    let grid = expand(&spec);
    eprintln!("grid: {} configs x {} window(s) = {} runs", grid.len(), windows.len(), grid.len() * windows.len());

    let rep = run_sweep(&windows, &grid, sample_ns, forge_sim::FeeSchedule::legacy(), latency_ns, 20, Thresholds::default());

    // rank by net for display
    let mut order: Vec<usize> = (0..rep.cells.len()).collect();
    order.sort_by(|&a, &b| rep.cells[b].net.partial_cmp(&rep.cells[a].net).unwrap_or(std::cmp::Ordering::Equal));

    println!("signal           {signal:?}");
    println!("configs (trials) {}", rep.n_trials);
    match rep.pbo {
        Some(p) => println!("sweep PBO        {p:.3}  ({})", if p < 0.5 { "below 0.5 - winners tend to survive OOS" } else { "0.5+ - overfit territory, no promotions" }),
        None => println!("sweep PBO        n/a (need more buckets)"),
    }
    println!();
    println!("{:>4} {:>12} {:>7} {:>6} {:>7} {:>6}  {:>8}  knobs", "id", "net", "trips", "win%", "sharpe", "dsr", "verdict");
    for &i in order.iter().take(25) {
        let c = &rep.cells[i];
        println!(
            "{:>4} {:>12.2} {:>7} {:>5.1}% {:>7.3} {:>6.3}  {:>8}  w={} thr={} hold={} tp={} sl={} lim={}",
            c.id, c.net, c.round_trips, c.win_rate * 100.0, c.sharpe, c.dsr,
            format!("{:?}", c.verdict),
            c.config.ofi_window, c.config.threshold, c.config.hold, c.config.tp_bps, c.config.sl_bps, c.config.use_limit
        );
    }
    let promote = rep.cells.iter().filter(|c| c.verdict == Verdict::Promote).count();
    let park = rep.cells.iter().filter(|c| c.verdict == Verdict::Park).count();
    let retire = rep.cells.iter().filter(|c| c.verdict == Verdict::Retire).count();
    println!();
    println!("verdicts: {promote} promote / {park} park / {retire} retire");
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