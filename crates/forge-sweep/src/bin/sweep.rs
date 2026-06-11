//! `forge-sweep` - sweep a bot (OFI momentum or the wall/imbalance bot) over
//! *.forge windows; print the promote/park/retire scorecard with %-edge, max
//! drawdown, and a leverage lens. Run detached in tmux (tools/sweep.sh).
//!
//!   forge-sweep w1.forge w2.forge --strategy ofi \
//!     --windows 20,50,100 --thresholds 1,1.5,2 --holds 5s,30s --cooldowns 2s \
//!     --tps 0,8 --sls 0,8 --limits false --leverage 20
//!   forge-sweep w1.forge --strategy wall --topn 3,5,10 --thresholds 0.2,0.4 \
//!     --reversions false,true --holds 5s,30s --cooldowns 2s --tps 0,8 --sls 0,8

use std::fmt::Debug;
use std::process::ExitCode;

use forge_core::{Event, Qty};
use forge_data::ForgeReader;
use forge_sim::FeeSchedule;
use forge_strategy::{AbsorptionBot, BasisBot, CvdBot, ObiBot, OfiMomentum, RegimeFilter, Signal, WallFlowBot};
use forge_sweep::{
    expand, expand_absorption, expand_basis, expand_cvd, expand_imbalance, expand_wallflow, run_sweep,
    AbsorptionGridSpec, BasisGridSpec, CvdGridSpec, GridSpec, ImbalanceGridSpec, SweepReport, Thresholds,
    Verdict, WallFlowGridSpec,
};

fn parse_f64s(s: &str) -> Result<Vec<f64>, String> {
    s.split(',').map(|x| x.trim().parse().map_err(|e| format!("bad number `{x}`: {e}"))).collect()
}
fn parse_usizes(s: &str) -> Result<Vec<usize>, String> {
    s.split(',').map(|x| x.trim().parse().map_err(|e| format!("bad int `{x}`: {e}"))).collect()
}
/// Parse a duration like `30s`, `5m`, `2h`, `250ms`, `500us`, `1000ns` (bare = ns) into nanoseconds.
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
fn parse_durs(s: &str) -> Result<Vec<u64>, String> {
    s.split(',').map(parse_dur).collect()
}
/// Format nanoseconds back to a compact human duration for the scorecard.
fn fmt_dur(ns: u64) -> String {
    if ns >= 3_600_000_000_000 && ns.is_multiple_of(3_600_000_000_000) {
        format!("{}h", ns / 3_600_000_000_000)
    } else if ns >= 60_000_000_000 && ns.is_multiple_of(60_000_000_000) {
        format!("{}m", ns / 60_000_000_000)
    } else if ns >= 1_000_000_000 && ns.is_multiple_of(1_000_000_000) {
        format!("{}s", ns / 1_000_000_000)
    } else if ns >= 1_000_000 && ns.is_multiple_of(1_000_000) {
        format!("{}ms", ns / 1_000_000)
    } else {
        format!("{ns}ns")
    }
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
fn parse_regimes(s: &str) -> Result<Vec<RegimeFilter>, String> {
    s.split(',')
        .map(|x| match x.trim().to_lowercase().as_str() {
            "any" => Ok(RegimeFilter::Any),
            "trend" | "trending" => Ok(RegimeFilter::OnlyTrending),
            "side" | "sideways" | "chop" => Ok(RegimeFilter::OnlySideways),
            "neutral" | "neut" => Ok(RegimeFilter::OnlyNeutral),
            other => Err(format!("bad regime `{other}` (any|trend|side|neutral)")),
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
    println!("regime split     trendN/sideN/neutN = net P&L earned while Trending/Sideways/Neutral (the specialization lens)");
    println!("{:>4} {:>11} {:>7} {:>6} {:>9} {:>11} {:>9} {:>9} {:>9} {:>6} {:>8}  knobs", "id", "net", "trips", "win%", "ret%/trip", "maxDD", "trendN", "sideN", "neutN", "dsr", "verdict");
    for &i in order.iter().take(top) {
        let c = &rep.cells[i];
        println!(
            "{:>4} {:>11.2} {:>7} {:>5.1}% {:>9.4} {:>11.2} {:>9.2} {:>9.2} {:>9.2} {:>6.3} {:>8}  {}",
            c.id, c.net, c.round_trips, c.win_rate * 100.0, c.avg_pct, c.max_dd,
            c.net_by_regime[0], c.net_by_regime[1], c.net_by_regime[2], c.dsr,
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

/// Persist a sweep to disk: a full per-config scorecard CSV for this run, plus
/// one appended summary row in `ledger.csv` (the tracked verdict history).
fn write_outputs<C>(
    rep: &SweepReport<C>,
    dir: &str,
    tag: &str,
    knobs: impl Fn(&C) -> String,
) -> std::io::Result<()> {
    use std::io::Write;
    std::fs::create_dir_all(dir)?;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let scorecard = format!("{dir}/{tag}-{ts}.csv");
    let mut f = std::fs::File::create(&scorecard)?;
    writeln!(f, "id,net,trips,win_pct,ret_pct_per_trip,max_dd,trend_net,side_net,neut_net,sharpe,dsr,verdict,knobs")?;
    for c in &rep.cells {
        writeln!(
            f,
            "{},{:.6},{},{:.4},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:.6},{:?},{}",
            c.id, c.net, c.round_trips, c.win_rate * 100.0, c.avg_pct, c.max_dd,
            c.net_by_regime[0], c.net_by_regime[1], c.net_by_regime[2], c.sharpe, c.dsr,
            c.verdict, knobs(&c.config)
        )?;
    }

    let promote = rep.cells.iter().filter(|c| c.verdict == Verdict::Promote).count();
    let park = rep.cells.iter().filter(|c| c.verdict == Verdict::Park).count();
    let retire = rep.cells.iter().filter(|c| c.verdict == Verdict::Retire).count();
    let best = rep.best_net_id.and_then(|id| rep.cells.iter().find(|c| c.id == id));
    let (best_net, best_knobs) = best.map_or((0.0, String::new()), |c| (c.net, knobs(&c.config)));
    let pbo = rep.pbo.unwrap_or(-1.0);

    let ledger = format!("{dir}/ledger.csv");
    let fresh = !std::path::Path::new(&ledger).exists();
    let mut lf = std::fs::OpenOptions::new().create(true).append(true).open(&ledger)?;
    if fresh {
        writeln!(lf, "unix_ts,tag,n_trials,pbo,promote,park,retire,best_net,best_knobs,scorecard")?;
    }
    writeln!(
        lf,
        "{ts},{tag},{},{pbo:.4},{promote},{park},{retire},{best_net:.6},{best_knobs},{scorecard}",
        rep.n_trials
    )?;
    eprintln!("wrote {scorecard} (+ ledger row in {ledger})");
    Ok(())
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
    let mut out_dir: Option<String> = None;

    // Pre-scan strategy so threshold axes match the signal's scale: OFI's
    // normalised signal can exceed 1, but imbalance/CVD are bounded to [-1, 1],
    // so they need sub-1 thresholds or the entry knob never bites (0 trips).
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let bounded = matches!(
        argv.iter().position(|a| a == "--strategy").and_then(|p| argv.get(p + 1)).map(String::as_str),
        Some("wall") | Some("cvd") | Some("wallflow")
    );
    let preset_name = argv
        .iter()
        .position(|a| a == "--preset")
        .and_then(|p| argv.get(p + 1))
        .cloned()
        .unwrap_or_else(|| "custom".to_string());

    // ofi axes
    let mut windows_ax = vec![20usize, 50, 100];
    let mut thr_ax = if bounded { vec![0.2, 0.4, 0.6] } else { vec![1.0, 1.5, 2.5] };
    // wall axes
    let mut topn_ax = vec![3usize, 5, 10];
    let mut rev_ax = vec![false, true];
    // shared axes
    let mut hold_ax: Vec<u64> = vec![5_000_000_000, 30_000_000_000];
    let mut cd_ax: Vec<u64> = vec![1_000_000_000];
    let mut tp_ax = vec![0.0, 8.0];
    let mut sl_ax = vec![0.0, 8.0];
    let mut lim_ax = vec![false];
    let mut reg_ax = vec![RegimeFilter::Any];
    let mut wallmin_ax = vec![2.0_f64, 5.0, 10.0];

    // Preset sets base axes before explicit flags override (argv pre-scanned above).
    if let Some(pos) = argv.iter().position(|a| a == "--preset") {
        match argv.get(pos + 1).map(String::as_str) {
            Some("fast") => {
                windows_ax = vec![20, 50, 100];
                topn_ax = vec![3, 5, 10];
                thr_ax = if bounded { vec![0.1, 0.2, 0.3, 0.4, 0.5] } else { vec![1.0, 1.5, 2.0, 2.5] };
                rev_ax = vec![false, true];
                hold_ax = vec![1_000_000_000, 5_000_000_000, 15_000_000_000]; // 1s,5s,15s
                cd_ax = vec![500_000_000, 2_000_000_000];                      // 0.5s,2s
                tp_ax = vec![0.0, 5.0, 8.0];
                sl_ax = vec![0.0, 5.0, 8.0];
            }
            Some("slow") => {
                windows_ax = vec![100, 200, 400];
                topn_ax = vec![5, 10, 20];
                thr_ax = if bounded { vec![0.2, 0.35, 0.5] } else { vec![2.0, 3.0, 4.0] };
                rev_ax = vec![false, true];
                hold_ax = vec![300_000_000_000, 900_000_000_000, 1_800_000_000_000]; // 5m,15m,30m
                cd_ax = vec![30_000_000_000, 120_000_000_000];                        // 30s,2m
                tp_ax = vec![0.0, 15.0, 30.0];
                sl_ax = vec![0.0, 15.0, 30.0];
            }
            other => return Err(format!("--preset must be fast|slow, got {other:?}")),
        }
    }

    let mut args = argv.into_iter();
    while let Some(a) = args.next() {
        let mut val = || args.next().ok_or_else(|| format!("missing value after {a}"));
        match a.as_str() {
            "--preset" => {
                let _ = val()?; // already handled in the pre-scan
            }
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
            "--holds" => hold_ax = parse_durs(&val()?)?,
            "--cooldowns" => cd_ax = parse_durs(&val()?)?,
            "--tps" => tp_ax = parse_f64s(&val()?)?,
            "--sls" => sl_ax = parse_f64s(&val()?)?,
            "--limits" => lim_ax = parse_bools(&val()?)?,
            "--regimes" => reg_ax = parse_regimes(&val()?)?,
            "--wallmins" => wallmin_ax = parse_f64s(&val()?)?,
            "--out" => out_dir = Some(val()?),
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
                ofi_window: windows_ax, threshold: thr_ax, qty: q, hold_ns: hold_ax, cooldown_ns: cd_ax,
                tp_bps: tp_ax, sl_bps: sl_ax, use_limit: lim_ax, signal, seed: 1, fill_timeout_ns: 200_000_000, regime_filter: reg_ax,
            });
            eprintln!("ofi grid: {} configs x {} window(s)", grid.len(), windows.len());
            let rep = run_sweep(&windows, &grid, OfiMomentum::new, sample_ns, fees, latency_ns, 20, th);
            let knobs = |c: &forge_strategy::MomentumConfig| {
                format!("w={} thr={} hold={} cd={} tp={} sl={} lim={} reg={:?}", c.ofi_window, c.threshold, fmt_dur(c.hold_ns), fmt_dur(c.cooldown_ns), c.tp_bps, c.sl_bps, c.use_limit, c.regime_filter)
            };
            print_report(&rep, signal, leverage, top, knobs);
            if let Some(dir) = &out_dir {
                write_outputs(&rep, dir, &format!("ofi-{preset_name}"), knobs).map_err(|e| format!("write outputs: {e}"))?;
            }
        }
        "wall" => {
            let grid = expand_imbalance(&ImbalanceGridSpec {
                top_n: topn_ax, threshold: thr_ax, reversion: rev_ax, qty: q, hold_ns: hold_ax, cooldown_ns: cd_ax,
                tp_bps: tp_ax, sl_bps: sl_ax, use_limit: lim_ax, signal, seed: 1, fill_timeout_ns: 200_000_000, regime_filter: reg_ax,
            });
            eprintln!("wall grid: {} configs x {} window(s)", grid.len(), windows.len());
            let rep = run_sweep(&windows, &grid, ObiBot::new, sample_ns, fees, latency_ns, 20, th);
            let knobs = |c: &forge_strategy::ImbalanceConfig| {
                format!("topN={} thr={} rev={} hold={} cd={} tp={} sl={} lim={} reg={:?}", c.top_n, c.threshold, c.reversion, fmt_dur(c.hold_ns), fmt_dur(c.cooldown_ns), c.tp_bps, c.sl_bps, c.use_limit, c.regime_filter)
            };
            print_report(&rep, signal, leverage, top, knobs);
            if let Some(dir) = &out_dir {
                write_outputs(&rep, dir, &format!("wall-{preset_name}"), knobs).map_err(|e| format!("write outputs: {e}"))?;
            }
        }
        "cvd" => {
            let grid = expand_cvd(&CvdGridSpec {
                window: windows_ax, threshold: thr_ax, reversion: rev_ax, qty: q, hold_ns: hold_ax, cooldown_ns: cd_ax,
                tp_bps: tp_ax, sl_bps: sl_ax, use_limit: lim_ax, signal, seed: 1, fill_timeout_ns: 200_000_000, regime_filter: reg_ax,
            });
            eprintln!("cvd grid: {} configs x {} window(s)", grid.len(), windows.len());
            let rep = run_sweep(&windows, &grid, CvdBot::new, sample_ns, fees, latency_ns, 20, th);
            let knobs = |c: &forge_strategy::CvdConfig| {
                format!("win={} thr={} rev={} hold={} cd={} tp={} sl={} lim={} reg={:?}", c.window, c.threshold, c.reversion, fmt_dur(c.hold_ns), fmt_dur(c.cooldown_ns), c.tp_bps, c.sl_bps, c.use_limit, c.regime_filter)
            };
            print_report(&rep, signal, leverage, top, knobs);
            if let Some(dir) = &out_dir {
                write_outputs(&rep, dir, &format!("cvd-{preset_name}"), knobs).map_err(|e| format!("write outputs: {e}"))?;
            }
        }
        "absorption" => {
            let grid = expand_absorption(&AbsorptionGridSpec {
                window: windows_ax, min_vol: wallmin_ax, reversion: rev_ax, qty: q,
                hold_ns: hold_ax, cooldown_ns: cd_ax, tp_bps: tp_ax, sl_bps: sl_ax,
                use_limit: lim_ax, signal, seed: 1, fill_timeout_ns: 200_000_000, regime_filter: reg_ax,
            });
            eprintln!("absorption grid: {} configs x {} window(s)", grid.len(), windows.len());
            let rep = run_sweep(&windows, &grid, AbsorptionBot::new, sample_ns, fees, latency_ns, 20, th);
            let knobs = |c: &forge_strategy::AbsorptionConfig| {
                format!("win={} minv={} rev={} hold={} cd={} tp={} sl={} lim={} reg={:?}", c.window, c.min_vol, c.reversion, fmt_dur(c.hold_ns), fmt_dur(c.cooldown_ns), c.tp_bps, c.sl_bps, c.use_limit, c.regime_filter)
            };
            print_report(&rep, signal, leverage, top, knobs);
            if let Some(dir) = &out_dir {
                write_outputs(&rep, dir, &format!("absorption-{preset_name}"), knobs).map_err(|e| format!("write outputs: {e}"))?;
            }
        }
        "wallflow" => {
            let grid = expand_wallflow(&WallFlowGridSpec {
                wall_min: wallmin_ax, window: windows_ax, cancel_ratio_min: thr_ax, reversion: rev_ax,
                qty: q, hold_ns: hold_ax, cooldown_ns: cd_ax, tp_bps: tp_ax, sl_bps: sl_ax,
                use_limit: lim_ax, signal, seed: 1, fill_timeout_ns: 200_000_000, regime_filter: reg_ax,
            });
            eprintln!("wallflow grid: {} configs x {} window(s)", grid.len(), windows.len());
            let rep = run_sweep(&windows, &grid, WallFlowBot::new, sample_ns, fees, latency_ns, 20, th);
            let knobs = |c: &forge_strategy::WallFlowConfig| {
                format!("wmin={} win={} cr={} rev={} hold={} cd={} tp={} sl={} lim={} reg={:?}", c.wall_min, c.window, c.cancel_ratio_min, c.reversion, fmt_dur(c.hold_ns), fmt_dur(c.cooldown_ns), c.tp_bps, c.sl_bps, c.use_limit, c.regime_filter)
            };
            print_report(&rep, signal, leverage, top, knobs);
            if let Some(dir) = &out_dir {
                write_outputs(&rep, dir, &format!("wallflow-{preset_name}"), knobs).map_err(|e| format!("write outputs: {e}"))?;
            }
        }
        "basis" => {
            let grid = expand_basis(&BasisGridSpec {
                top_n: topn_ax, threshold_bps: thr_ax, window: windows_ax, reversion: rev_ax,
                sample_ns: 500_000_000, qty: q, hold_ns: hold_ax, cooldown_ns: cd_ax,
                tp_bps: tp_ax, sl_bps: sl_ax, use_limit: lim_ax, signal, seed: 1,
                // fill timeout MUST exceed order latency, else the shell re-fires
                // exits while the first is in flight -> runaway position.
                fill_timeout_ns: latency_ns.saturating_add(500_000_000).max(200_000_000), regime_filter: reg_ax,
            });
            eprintln!("basis grid: {} configs x {} window(s)", grid.len(), windows.len());
            let rep = run_sweep(&windows, &grid, BasisBot::new, sample_ns, fees, latency_ns, 20, th);
            let knobs = |c: &forge_strategy::BasisConfig| {
                format!("topN={} thr={}bps win={} rev={} hold={} cd={} tp={} sl={} reg={:?}", c.top_n, c.threshold_bps, c.window, c.reversion, fmt_dur(c.hold_ns), fmt_dur(c.cooldown_ns), c.tp_bps, c.sl_bps, c.regime_filter)
            };
            print_report(&rep, signal, leverage, top, knobs);
            if let Some(dir) = &out_dir {
                write_outputs(&rep, dir, &format!("basis-{preset_name}"), knobs).map_err(|e| format!("write outputs: {e}"))?;
            }
        }
        other => return Err(format!("unknown --strategy `{other}` (use ofi|wall|cvd|wallflow|absorption|basis)")),
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