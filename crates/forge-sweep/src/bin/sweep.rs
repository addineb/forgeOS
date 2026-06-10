//! `forge-sweep` - sweep a bot (OFI momentum or the wall/imbalance bot) over
//! *.forge windows; print the promote/park/retire scorecard with %-edge, max
//! drawdown, and a leverage lens. Run detached in tmux (tools/sweep.sh).
//!
//!   forge-sweep w1.forge w2.forge --strategy ofi \
//!     --windows 20,50,100 --thresholds 1,1.5,2 --holds 100,300 --cooldowns 50 \
//!     --tps 0,8 --sls 0,8 --limits false --leverage 20
//!   forge-sweep w1.forge --strategy wall --topn 3,5,10 --thresholds 0.2,0.4 \
//!     --reversions false,true --holds 100,300 --cooldowns 50 --tps 0,8 --sls 0,8

use std::fmt::Debug;
use std::process::ExitCode;

use forge_core::{Event, Qty};
use forge_data::ForgeReader;
use forge_sim::FeeSchedule;
use forge_strategy::{ObiBot, OfiMomentum, Signal};
use forge_sweep::{
    expand, expand_imbalance, run_sweep, GridSpec, ImbalanceGridSpec, SweepReport, Thresholds, Verdict,
};

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
            "false" | "0" | "market" | "follow" => Ok(false),
            "true" | "1" | "limit" | "fade" => Ok(true),
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

fn print_report<C: Debug>(rep: &SweepReport<C>, signal: Signal, leverage: f64, top: usize, knobs: impl Fn(&C) -> String) {
    let mut order: Vec<usize> = (0..rep.cells.len()).collect();
    order.sort_by(|&a, &b| rep.cells[b].net.partial_cmp(&rep.cells[a].net).unwrap_or(std::cmp::Ordering::Equal));

    println!("signal           {signal:?}");
    println!("configs (trials) {}", rep.n_trials);
    println!("leverage lens    {leverage}x  (edge in % is leverage-free; ROE = ret% * lev)");
    match rep.pbo {
        Some(p) => println!("sweep PBO        {p:.3}  ({})", if p < 0.5 { "winners tend to survive OOS" } else { "0.5+ overfit - no promotions" }),
        None => println!("sweep PBO        n/a"),
    }
    println!();
    println!("{:>4} {:>11} {:>7} {:>6} {:>9} {:>11} {:>6} {:>8}  knobs", "id", "net", "trips", "win%", "ret%/trip", "maxDD", "dsr", "verdict");
    for &i in order.iter().take(top) {
        let c = &rep.cells[i];
        println!(
            "{:>4} {:>11.2} {:>7} {:>5.1}% {:>9.4} {:>11.2} {:>6.3} {:>8}  {}",
            c.id, c.net, c.round_trips, c.win_rate * 100.0, c.avg_pct, c.max_dd, c.dsr,
            format!("{:?}", c.verdict), knobs(&c.config)
        );
    }
    let promote = rep.cells.iter().filter(|c| c.verdict == Verdict::Promote).count();
    let park = rep.cells.iter().filter(|c| c.verdict == Verdict::Park).count();
    let retire = rep.cells.iter().filter(|c| c.verdict == Verdict::Retire).count();
    println!();
    if let Some(&best) = order.first() {
        let c = &rep.cells[best];
        println!("best by net (id {}): edge {:.4}% per trip -> at {}x = {:.4}% on margin/trip", c.id, c.avg_pct, leverage, c.avg_pct * leverage);
        println!("  (a negative % edge cannot be fixed by leverage - it scales gains AND losses)");
    }
    println!("verdicts: {promote} promote / {park} park / {retire} retire");
}

#[allow(clippy::too_many_lines)]
fn run() -> Result<(), String> {
    let mut paths: Vec<String> = Vec::new();
    let mut strategy = "ofi".to_string();
    let mut signal = Signal::Real;
    let mut sample_ns: u64 = 60_000_000_000;
    let mut latency_ns: u64 = 2_000_000;
    let mut leverage: f64 = 1.0;
    let mut top: usize = 25;
    let mut qty = 0.01_f64;

    // ofi axes
    let mut windows_ax = vec![20usize, 50, 100];
    let mut thr_ax = vec![1.0, 1.5, 2.5];
    // wall axes
    let mut topn_ax = vec![3usize, 5, 10];
    let mut rev_ax = vec![false, true];
    // shared axes
    let mut hold_ax = vec![100u32, 300];
    let mut cd_ax = vec![50u32];
    let mut tp_ax = vec![0.0, 8.0];
    let mut sl_ax = vec![0.0, 8.0];
    let mut lim_ax = vec![false];

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = || args.next().ok_or_else(|| format!("missing value after {a}"));
        match a.as_str() {
            "--strategy" => strategy = val()?,
            "--shuffle" => signal = Signal::Shuffled,
            "--sample-ns" => sample_ns = val()?.parse().map_err(|e| format!("sample-ns: {e}"))?,
            "--latency-ns" => latency_ns = val()?.parse().map_err(|e| format!("latency-ns: {e}"))?,
            "--leverage" => leverage = val()?.parse().map_err(|e| format!("leverage: {e}"))?,
            "--top" => top = val()?.parse().map_err(|e| format!("top: {e}"))?,
            "--qty" => qty = val()?.parse().map_err(|e| format!("qty: {e}"))?,
            "--windows" => windows_ax = parse_usizes(&val()?)?,
            "--thresholds" => thr_ax = parse_f64s(&val()?)?,
            "--topn" => topn_ax = parse_usizes(&val()?)?,
            "--reversions" => rev_ax = parse_bools(&val()?)?,
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
        return Err("usage: forge-sweep <window.forge> [more...] --strategy ofi|wall [grid flags]".into());
    }

    eprintln!("loading {} window(s)...", paths.len());
    let mut windows = Vec::new();
    for p in &paths {
        let w = load_window(p)?;
        eprintln!("  {p}: {} events", w.len());
        windows.push(w);
    }
    let q = Qty::from_f64(qty).map_err(|e| format!("qty: {e}"))?;
    let fees = FeeSchedule::legacy();
    let th = Thresholds::default();

    match strategy.as_str() {
        "ofi" => {
            let grid = expand(&GridSpec {
                ofi_window: windows_ax, threshold: thr_ax, qty: q, hold: hold_ax, cooldown: cd_ax,
                tp_bps: tp_ax, sl_bps: sl_ax, use_limit: lim_ax, signal, seed: 1, fill_timeout_ns: 200_000_000,
            });
            eprintln!("ofi grid: {} configs x {} window(s)", grid.len(), windows.len());
            let rep = run_sweep(&windows, &grid, OfiMomentum::new, sample_ns, fees, latency_ns, 20, th);
            print_report(&rep, signal, leverage, top, |c| {
                format!("w={} thr={} hold={} cd={} tp={} sl={} lim={}", c.ofi_window, c.threshold, c.hold, c.cooldown, c.tp_bps, c.sl_bps, c.use_limit)
            });
        }
        "wall" => {
            let grid = expand_imbalance(&ImbalanceGridSpec {
                top_n: topn_ax, threshold: thr_ax, reversion: rev_ax, qty: q, hold: hold_ax, cooldown: cd_ax,
                tp_bps: tp_ax, sl_bps: sl_ax, use_limit: lim_ax, signal, seed: 1, fill_timeout_ns: 200_000_000,
            });
            eprintln!("wall grid: {} configs x {} window(s)", grid.len(), windows.len());
            let rep = run_sweep(&windows, &grid, ObiBot::new, sample_ns, fees, latency_ns, 20, th);
            print_report(&rep, signal, leverage, top, |c| {
                format!("topN={} thr={} rev={} hold={} cd={} tp={} sl={} lim={}", c.top_n, c.threshold, c.reversion, c.hold, c.cooldown, c.tp_bps, c.sl_bps, c.use_limit)
            });
        }
        other => return Err(format!("unknown --strategy `{other}` (use ofi|wall)")),
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