//! `forge-sweep` - sweep OFI-momentum configs over *.forge windows and print
//! the promote/park/retire scorecard with bps-per-trade (leverage-free edge),
//! max drawdown, and an optional leverage lens. Run detached in tmux for big
//! grids (tools/sweep.sh).
//!
//! Grid axes are comma lists, e.g.:
//!   forge-sweep w1.forge w2.forge \
//!     --windows 20,50,100,200 --thresholds 0.5,1,1.5,2,2.5,3 \
//!     --holds 50,100,300,600 --cooldowns 20,50 --tps 0,5,8 --sls 0,5,8 \
//!     --limits false,true --leverage 20

use std::process::ExitCode;

use forge_core::{Event, Qty};
use forge_data::ForgeReader;
use forge_strategy::Signal;
use forge_sweep::{expand, run_sweep, GridSpec, Thresholds, Verdict};

fn parse_f64s(s: &str) -> Result<Vec<f64>, String> {
    s.split(',').map(|x| x.trim().parse().map_err(|e| format!("bad number `{x}`: {e}"))).collect()
}
fn parse_usizes(s: &str) -> Result<Vec<usize>, String> {
    s.split(',').map(|x| x.trim().parse().map_err(|e| format!("bad int `{x}`: {e}"))).collect()
}
fn parse_u32s(s: &str) -> Result<Vec<u32>, String> {
    s.split(',').map(|x| x.trim().parse().map_err(|e| format!("bad int `{x}`: {e}"))).collect()
}
fn parse_bools(s: &str) -> Result<Vec<bool>, String> {
    s.split(',')
        .map(|x| match x.trim() {
            "false" | "0" | "market" => Ok(false),
            "true" | "1" | "limit" => Ok(true),
            other => Err(format!("bad bool `{other}`")),
        })
        .collect()
}

fn load_window(path: &str) -> Result<Vec<Event>, String> {
    let reader = ForgeReader::open(path).map_err(|e| format!("open {path}: {e}"))?;
    let mut evs = Vec::with_capacity(reader.len());
    for rec in reader.records() {
        evs.push(rec.to_event().map_err(|e| format!("decode {path}: {e}"))?);
    }
    Ok(evs)
}

#[allow(clippy::too_many_lines)]
fn run() -> Result<(), String> {
    let mut paths: Vec<String> = Vec::new();
    let mut signal = Signal::Real;
    let mut sample_ns: u64 = 60_000_000_000;
    let mut latency_ns: u64 = 2_000_000;
    let mut leverage: f64 = 1.0;
    let mut top: usize = 25;
    let mut qty = 0.01_f64;

    let mut windows_ax = vec![20usize, 50, 100];
    let mut thr_ax = vec![1.0, 1.5, 2.5];
    let mut hold_ax = vec![100u32, 300];
    let mut cd_ax = vec![50u32];
    let mut tp_ax = vec![0.0, 8.0];
    let mut sl_ax = vec![0.0, 8.0];
    let mut lim_ax = vec![false];

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = || args.next().ok_or_else(|| format!("missing value after {a}"));
        match a.as_str() {
            "--shuffle" => signal = Signal::Shuffled,
            "--sample-ns" => sample_ns = val()?.parse().map_err(|e| format!("sample-ns: {e}"))?,
            "--latency-ns" => latency_ns = val()?.parse().map_err(|e| format!("latency-ns: {e}"))?,
            "--leverage" => leverage = val()?.parse().map_err(|e| format!("leverage: {e}"))?,
            "--top" => top = val()?.parse().map_err(|e| format!("top: {e}"))?,
            "--qty" => qty = val()?.parse().map_err(|e| format!("qty: {e}"))?,
            "--windows" => windows_ax = parse_usizes(&val()?)?,
            "--thresholds" => thr_ax = parse_f64s(&val()?)?,
            "--holds" => hold_ax = parse_u32s(&val()?)?,
            "--cooldowns" => cd_ax = parse_u32s(&val()?)?,
            "--tps" => tp_ax = parse_f64s(&val()?)?,
            "--sls" => sl_ax = parse_f64s(&val()?)?,
            "--limits" => lim_ax = parse_bools(&val()?)?,
            s if s.starts_with("--") => return Err(format!("unknown flag {s}")),
            s => paths.push(s.to_string()),
        }
    }
    if paths.is_empty() {
        return Err("usage: forge-sweep <window.forge> [more...] [grid flags] (see header)".into());
    }

    eprintln!("loading {} window(s)...", paths.len());
    let mut windows = Vec::new();
    for p in &paths {
        let w = load_window(p)?;
        eprintln!("  {p}: {} events", w.len());
        windows.push(w);
    }

    let spec = GridSpec {
        ofi_window: windows_ax,
        threshold: thr_ax,
        qty: Qty::from_f64(qty).map_err(|e| format!("qty: {e}"))?,
        hold: hold_ax,
        cooldown: cd_ax,
        tp_bps: tp_ax,
        sl_bps: sl_ax,
        use_limit: lim_ax,
        signal,
        seed: 1,
        fill_timeout_ns: 200_000_000,
    };
    let grid = expand(&spec);
    eprintln!("grid: {} configs x {} window(s) = {} runs", grid.len(), windows.len(), grid.len() * windows.len());

    let rep = run_sweep(&windows, &grid, sample_ns, forge_sim::FeeSchedule::legacy(), latency_ns, 20, Thresholds::default());

    let mut order: Vec<usize> = (0..rep.cells.len()).collect();
    order.sort_by(|&a, &b| rep.cells[b].net.partial_cmp(&rep.cells[a].net).unwrap_or(std::cmp::Ordering::Equal));

    println!("signal           {signal:?}");
    println!("configs (trials) {}", rep.n_trials);
    println!("leverage lens    {leverage}x  (edge in % is leverage-free; ROE = ret% * lev)");
    match rep.pbo {
        Some(p) => println!("sweep PBO        {p:.3}  ({})", if p < 0.5 { "winners tend to survive OOS" } else { "0.5+ overfit territory - no promotions" }),
        None => println!("sweep PBO        n/a"),
    }
    println!();
    println!("{:>4} {:>11} {:>7} {:>6} {:>9} {:>11} {:>6} {:>8}  knobs", "id", "net", "trips", "win%", "ret%/trip", "maxDD", "dsr", "verdict");
    for &i in order.iter().take(top) {
        let c = &rep.cells[i];
        println!(
            "{:>4} {:>11.2} {:>7} {:>5.1}% {:>9.4} {:>11.2} {:>6.3} {:>8}  w={} thr={} hold={} cd={} tp={} sl={} lim={}",
            c.id, c.net, c.round_trips, c.win_rate * 100.0, c.avg_pct, c.max_dd, c.dsr,
            format!("{:?}", c.verdict),
            c.config.ofi_window, c.config.threshold, c.config.hold, c.config.cooldown, c.config.tp_bps, c.config.sl_bps, c.config.use_limit
        );
    }
    let promote = rep.cells.iter().filter(|c| c.verdict == Verdict::Promote).count();
    let park = rep.cells.iter().filter(|c| c.verdict == Verdict::Park).count();
    let retire = rep.cells.iter().filter(|c| c.verdict == Verdict::Retire).count();
    println!();
    if let Some(best) = order.first() {
        let c = &rep.cells[*best];
        let roe_per_trip = c.avg_pct * leverage;
        println!(
            "best by net (id {}): edge {:.4}% per trip  ->  at {}x = {:.4}% return on margin per trip",
            c.id, c.avg_pct, leverage, roe_per_trip
        );
        println!("  (a negative % edge cannot be fixed by leverage - it scales gains AND losses)");
    }
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