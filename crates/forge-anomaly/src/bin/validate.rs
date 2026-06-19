//! Validate binary: runs AnomalyEngine on real CSV data.
//!
//! Usage:
//!   validate --input data.csv                        # full file
//!   validate --input data.csv --date 2026-06-05      # single date
//!   validate --input data.csv --date 2026-06-05 --output results.txt
//!   validate --synthetic                             # synthetic test data

use std::fs::File;
use std::io::{self, Write};
use std::path::PathBuf;

use clap::Parser;
use forge_anomaly::{
    load_volume_bars, AnomalyEngine, AnomalyKind, AnomalySignal, CausalDirection,
    CausalEngine, CausalEngineOutput, CausalSignal, EngineConfig, EngineMode, SignalDirection,
    SignalType, VolumeBar,
};

// ── CLI ───────────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "forge-anomaly validate",
    about = "Run the anomaly engine on depthscope CSV data and print signal/pattern stats.",
    long_about = "Replays volume-bar CSV data through AnomalyEngine and reports:\n\
                  - Signal count, rate, direction, and confidence distribution\n\
                  - Pattern-based vs Mahalanobis-only signal breakdown\n\
                  - Per-anomaly-kind event totals\n\
                  \n\
                  Without --input, generates synthetic data for a quick smoke test."
)]
struct Cli {
    /// Path to depthscope CSV file (BTCUSDT_YYYY-MM-DD_vb10.csv format).
    #[arg(long, short, value_hint = clap::ValueHint::FilePath)]
    input: Option<PathBuf>,

    /// Filter bars to a single UTC date: YYYY-MM-DD.
    #[arg(long, short, value_name = "DATE")]
    date: Option<String>,

    /// Write output to this file instead of stdout.
    #[arg(long, short, value_hint = clap::ValueHint::FilePath)]
    output: Option<PathBuf>,

    /// Print every anomaly event and signal detail (noisy).
    #[arg(long, short)]
    verbose: bool,

    /// Run on synthetic test data instead of a CSV file.
    #[arg(long)]
    synthetic: bool,

    /// Engine implementation. `legacy` (default) runs the Mahalanobis + z-score + FDR
    /// pipeline. `causal` runs the new template-based CausalEngine (first template:
    /// absorption_reversal). Both modes share the same output schema.
    #[arg(long, value_enum, default_value_t = EngineModeArg::Legacy)]
    engine: EngineModeArg,
}

/// Clap-friendly enum mirroring `forge_anomaly::EngineMode`. Kept separate so
/// clap's derive generates a clean `--help` listing without exposing the
/// internal type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum EngineModeArg {
    Legacy,
    Causal,
}

impl From<EngineModeArg> for EngineMode {
    fn from(a: EngineModeArg) -> Self {
        match a {
            EngineModeArg::Legacy => EngineMode::Legacy,
            EngineModeArg::Causal => EngineMode::Causal,
        }
    }
}

// ── main ──────────────────────────────────────────────────────────────────────

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // Resolve output writer: file or stdout.
    let mut out: Box<dyn Write> = if let Some(ref path) = cli.output {
        Box::new(File::create(path)?)
    } else {
        Box::new(io::stdout())
    };

    // Load bars.
    let (bars, cfg, desc) = if cli.synthetic || cli.input.is_none() {
        writeln!(out, "# synthetic data (relaxed thresholds)")?;
        let b = synthetic_bars(300);
        let c = EngineConfig {
            mahalanobis_threshold: 2.5,
            min_confidence: 0.45,
            ..EngineConfig::default()
        };
        (b, c, "synthetic".to_string())
    } else {
        let path = cli.input.as_ref().unwrap();
        let all = load_volume_bars(path)?;
        let filtered = if let Some(ref date) = cli.date {
            filter_by_date(all, date)?
        } else {
            all
        };
        let d = cli.date.as_deref().unwrap_or("all dates");
        eprintln!(
            "  loaded {} bars from {} (date: {})",
            filtered.len(),
            path.display(),
            d
        );
        (
            filtered,
            EngineConfig::default(),
            format!("{}", path.display()),
        )
    };

    if bars.is_empty() {
        writeln!(out, "# no bars to process")?;
        return Ok(());
    }

    // Track the date range actually seen (used by both engine modes).
    let mut seen_earliest_ts: u64 = u64::MAX;
    let mut seen_latest_ts: u64 = 0;
    for bar in &bars {
        seen_earliest_ts = seen_earliest_ts.min(bar.ts);
        seen_latest_ts = seen_latest_ts.max(bar.ts);
    }

    // ── Dispatch by engine mode ──
    let engine_mode: EngineMode = cli.engine.into();
    if engine_mode == EngineMode::Causal {
        // Causal mode ignores legacy-only config fields; uses causal defaults.
        let mut causal_cfg = EngineConfig::default();
        causal_cfg.engine_mode = EngineMode::Causal;
        causal_cfg.lookback_bars = causal_cfg.causal.lookback_bars;
        return run_causal_mode(
            &mut out,
            &bars,
            &causal_cfg,
            &desc,
            seen_earliest_ts,
            seen_latest_ts,
        );
    }

    // Run engine (legacy).
    let mut engine = AnomalyEngine::new(cfg.clone());
    let mut all_signals: Vec<AnomalySignal> = Vec::new();
    let mut total_maha = 0.0;
    let mut maha_count = 0u64;
    let mut anomaly_counts: std::collections::HashMap<AnomalyKind, u64> =
        std::collections::HashMap::new();

    for (_bar_idx, bar) in bars.iter().enumerate() {
        let output = engine.on_bar(bar);

        if output.mahalanobis_dist > 0.0 {
            total_maha += output.mahalanobis_dist;
            maha_count += 1;
        }
        for ev in &output.anomalies {
            *anomaly_counts.entry(ev.kind).or_insert(0) += 1;
        }

        if cli.verbose {
            for ev in &output.anomalies {
                writeln!(
                    out,
                    "  ANOMALY bar={:<6} kind={:<20} dir={:<8} z={:+.2} raw={:.3} conf={:.3}",
                    ev.bar_index,
                    format!("{:?}", ev.kind),
                    ev.direction,
                    ev.z_score,
                    ev.raw_value,
                    ev.confidence
                )?;
            }
            if output.mahalanobis_dist >= cfg.mahalanobis_threshold {
                writeln!(
                    out,
                    "  MAHA    bar={:<6} dist={:.2}",
                    bar.bar_index, output.mahalanobis_dist
                )?;
            }
        }

        if let Some(sig) = output.signal {
            if cli.verbose {
                let has_pat = sig
                    .events
                    .iter()
                    .any(|e| e.kind == AnomalyKind::PatternRepeat);
                let tag = if has_pat { "[PATTERN]" } else { "" };
                writeln!(out,
                    "  SIGNAL  bar={:<6} ts={} {:?}/{:?} conf={:.3} maha={:.2} move={:.1}bps hold={} {}",
                    sig.bar_index, format_ts(sig.ts), sig.signal_type, sig.direction,
                    sig.confidence, sig.mahalanobis_dist, sig.expected_move_bps, sig.hold_bars, tag)?;
            }
            all_signals.push(sig);
        }
    }

    let has_pattern = |sig: &AnomalySignal| -> bool {
        sig.events
            .iter()
            .any(|e| e.kind == AnomalyKind::PatternRepeat)
    };
    let pattern_signals: Vec<_> = all_signals.iter().filter(|s| has_pattern(s)).collect();
    let maha_only: Vec<_> = all_signals.iter().filter(|s| !has_pattern(s)).collect();

    // ── Header ──
    writeln!(out)?;
    writeln!(
        out,
        "═══════════════════════════════════════════════════════════════"
    )?;
    writeln!(out, "  forge-anomaly validate  |  source: {}", desc)?;
    if cli.date.is_some() {
        writeln!(
            out,
            "  date: {}  |  bars: {}",
            cli.date.as_deref().unwrap(),
            bars.len()
        )?;
    } else {
        let early = if seen_earliest_ts < u64::MAX {
            format_ts_date_only(seen_earliest_ts)
        } else {
            "?".into()
        };
        let late = if seen_latest_ts > 0 {
            format_ts_date_only(seen_latest_ts)
        } else {
            "?".into()
        };
        writeln!(
            out,
            "  dates: {} → {}  |  bars: {}",
            early,
            late,
            bars.len()
        )?;
    }
    writeln!(out, "  config: lookback={}  maha_thresh={:.1}  maha_max={:.0}  min_conf={:.2}  fee={:.0}bps  fdr_alpha={:.2}",
        cfg.lookback_bars, cfg.mahalanobis_threshold, cfg.mahalanobis_max, cfg.min_confidence, cfg.fee_bps, cfg.fdr_alpha)?;
    writeln!(
        out,
        "  pattern: min_count={}  lookback={}  seq_len_auto  cooldown_auto",
        cfg.min_pattern_count, cfg.pattern_lookback_bars
    )?;
    writeln!(
        out,
        "═══════════════════════════════════════════════════════════════"
    )?;
    writeln!(out)?;

    // ── Signals ──
    if all_signals.is_empty() {
        writeln!(out, "  NO SIGNALS generated.")?;
        let avg_m = if maha_count > 0 {
            total_maha / maha_count as f64
        } else {
            0.0
        };
        writeln!(
            out,
            "  avg maha_dist (non-zero) = {:.2}  (threshold = {:.1})",
            avg_m, cfg.mahalanobis_threshold
        )?;
    } else {
        writeln!(
            out,
            "── Signal Table ─────────────────────────────────────────────"
        )?;
        writeln!(
            out,
            "  {:>3} {:>6} {:>19} {:>12} {:>8} {:>6.3} {:>6.1} {:>6.1} {:>5} {}",
            "#", "bar", "ts", "type", "dir", "conf", "maha", "move", "hold", "pattern"
        )?;
        writeln!(
            out,
            "  {} {} {} {} {} {} {} {} {} {}",
            "---",
            "------",
            "-------------------",
            "------------",
            "--------",
            "-------",
            "-------",
            "-------",
            "-----",
            "-------"
        )?;
        for (idx, sig) in all_signals.iter().enumerate() {
            let pat = if has_pattern(sig) { "✓" } else { "" };
            writeln!(
                out,
                "  {:>3} {:>6} {:>19} {:>12} {:>8} {:>6.3} {:>6.1} {:>6.1} {:>5} {:>7}",
                idx,
                sig.bar_index,
                format_ts(sig.ts),
                sig.signal_type,
                sig.direction,
                sig.confidence,
                sig.mahalanobis_dist,
                sig.expected_move_bps,
                sig.hold_bars,
                pat
            )?;
        }
        writeln!(out)?;
    }

    // ── Statistics ──
    writeln!(
        out,
        "── Summary Statistics ───────────────────────────────────────"
    )?;
    writeln!(out, "  bars processed:             {:>6}", bars.len())?;
    writeln!(
        out,
        "  total signals:              {:>6}  ({:.1}% of bars)",
        all_signals.len(),
        all_signals.len() as f64 / bars.len() as f64 * 100.0
    )?;
    writeln!(
        out,
        "    mahalanobis only:         {:>6}  ({:.1}% of signals)",
        maha_only.len(),
        if all_signals.is_empty() {
            0.0
        } else {
            maha_only.len() as f64 / all_signals.len() as f64 * 100.0
        }
    )?;
    writeln!(
        out,
        "    pattern-based:            {:>6}  ({:.1}% of signals)  ← KEY METRIC",
        pattern_signals.len(),
        if all_signals.is_empty() {
            0.0
        } else {
            pattern_signals.len() as f64 / all_signals.len() as f64 * 100.0
        }
    )?;
    writeln!(out)?;

    if !all_signals.is_empty() {
        let n = all_signals.len() as f64;
        let avg_c = all_signals.iter().map(|s| s.confidence).sum::<f64>() / n;
        let avg_mv = all_signals.iter().map(|s| s.expected_move_bps).sum::<f64>() / n;
        let avg_h = all_signals.iter().map(|s| s.hold_bars as f64).sum::<f64>() / n;

        let min_c = all_signals
            .iter()
            .map(|s| s.confidence)
            .fold(f64::INFINITY, f64::min);
        let max_c = all_signals
            .iter()
            .map(|s| s.confidence)
            .fold(0.0_f64, f64::max);

        writeln!(
            out,
            "  avg confidence:             {:.3}  (range {:.3} – {:.3})",
            avg_c, min_c, max_c
        )?;
        writeln!(out, "  avg expected move:          {:.1} bps", avg_mv)?;
        writeln!(out, "  avg hold bars:              {:.0}", avg_h)?;

        let avg_m = if maha_count > 0 {
            total_maha / maha_count as f64
        } else {
            0.0
        };
        writeln!(out, "  avg maha_dist (non-zero):   {:.2}", avg_m)?;

        let rev = all_signals
            .iter()
            .filter(|s| matches!(s.signal_type, SignalType::Reversal))
            .count();
        let mom = all_signals
            .iter()
            .filter(|s| matches!(s.signal_type, SignalType::MomentumContinuation))
            .count();
        let lng = all_signals
            .iter()
            .filter(|s| matches!(s.direction, SignalDirection::Long))
            .count();
        let sht = all_signals
            .iter()
            .filter(|s| matches!(s.direction, SignalDirection::Short))
            .count();
        writeln!(
            out,
            "  reversal: {:>2}   momentum: {:>2}   long: {:>2}   short: {:>2}",
            rev, mom, lng, sht
        )?;

        // Pattern signal details.
        if !pattern_signals.is_empty() {
            let avg_pat_c = pattern_signals.iter().map(|s| s.confidence).sum::<f64>()
                / pattern_signals.len() as f64;
            let avg_pat_cnt = pattern_signals
                .iter()
                .map(|s| s.pattern_count as f64)
                .sum::<f64>()
                / pattern_signals.len() as f64;
            writeln!(out, "  pattern signals avg conf:   {:.3}", avg_pat_c)?;
            writeln!(out, "  pattern signals avg rep:    {:.1}", avg_pat_cnt)?;
        }
    }

    // ── Anomaly Kind Totals ──
    writeln!(out)?;
    writeln!(
        out,
        "── Anomaly Kind Totals ──────────────────────────────────────"
    )?;
    let mut sorted: Vec<_> = anomaly_counts.into_iter().collect();
    sorted.sort_by_key(|(_, c)| -(*c as i64));
    for (kind, count) in &sorted {
        let pct = if bars.is_empty() {
            0.0
        } else {
            *count as f64 / bars.len() as f64 * 100.0
        };
        let bar = if pct > 20.0 {
            "████████"
        } else if pct > 5.0 {
            "████"
        } else if pct > 0.5 {
            "█"
        } else {
            ""
        };
        writeln!(
            out,
            "  {:>20}  {:>6}  ({:>5.1}%)  {}",
            format!("{:?}", kind),
            count,
            pct,
            bar
        )?;
    }

    writeln!(out)?;
    writeln!(out, "done.")?;
    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn filter_by_date(mut bars: Vec<VolumeBar>, date_str: &str) -> io::Result<Vec<VolumeBar>> {
    let (start_ns, end_ns) = date_to_ns_range(date_str);
    bars.sort_by_key(|b| b.ts);
    let mut prev_cum = 0.0_f64;
    let mut out: Vec<VolumeBar> = Vec::new();
    for bar in bars {
        if bar.ts >= start_ns && bar.ts < end_ns {
            out.push(bar);
        }
    }
    for (i, bar) in out.iter_mut().enumerate() {
        bar.bar_index = i as u64;
        bar.bar_vol = if i == 0 {
            bar.cum_vol
        } else {
            (bar.cum_vol - prev_cum).max(0.0)
        };
        prev_cum = bar.cum_vol;
    }
    Ok(out)
}

fn date_to_ns_range(date: &str) -> (u64, u64) {
    let parts: Vec<&str> = date.split('-').collect();
    if parts.len() != 3 {
        return (0, u64::MAX);
    }
    let y: i64 = parts[0].parse().unwrap_or(2026);
    let m: i64 = parts[1].parse().unwrap_or(6);
    let d: i64 = parts[2].parse().unwrap_or(1);
    let days = days_since_epoch(y, m, d);
    if days < 0 {
        return (0, u64::MAX);
    }
    let start_ns = (days as u64) * 86_400_000_000_000u64;
    let end_ns = start_ns + 86_400_000_000_000u64;
    (start_ns, end_ns)
}

fn days_since_epoch(y: i64, m: i64, d: i64) -> i64 {
    let m = m - 3;
    let y = if m < 0 { y - 1 } else { y };
    let m = if m < 0 { m + 12 } else { m };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let doy = (153 * m + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let epoch_days = 719468;
    era * 146097 + doe - epoch_days
}

fn format_ts(ns: u64) -> String {
    let secs = ns / 1_000_000_000;
    let days = secs / 86400;
    let time = secs % 86400;
    let h = time / 3600;
    let m = (time % 3600) / 60;
    let s = time % 60;
    let (y, mo, d) = epoch_to_date(days as i64);
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", y, mo, d, h, m, s)
}

fn format_ts_date_only(ns: u64) -> String {
    let secs = ns / 1_000_000_000;
    let days = secs / 86400;
    let (y, mo, d) = epoch_to_date(days as i64);
    format!("{:04}-{:02}-{:02}", y, mo, d)
}

fn epoch_to_date(days: i64) -> (i64, i64, i64) {
    let z = days + 719468;
    let era = if z >= 0 {
        z / 146097
    } else {
        (z - 146096) / 146097
    };
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

// ── Synthetic data ────────────────────────────────────────────────────────────

fn synthetic_bars(n: usize) -> Vec<VolumeBar> {
    let mut bars = Vec::with_capacity(n);
    let base_price = 50_000.0;
    let mut cum_vol = 0.0;
    for i in 0..n {
        let ts = (i as u64) * 30_000_000_000;
        let bar_vol = 10.0 + (i % 5) as f64 * 2.0;
        cum_vol += bar_vol;
        let (bid_vol, ask_vol, cvd) = match i {
            0..=79 => (
                100.0 + nz(i, 0) * 10.0,
                100.0 + nz(i, 1) * 10.0,
                nz(i, 2) * 0.5,
            ),
            80..=94 => (
                30.0 + nz(i, 3) * 5.0,
                250.0 + nz(i, 4) * 10.0,
                -15.0 + nz(i, 5) * 2.0,
            ),
            95..=119 => (
                100.0 + nz(i, 6) * 10.0,
                100.0 + nz(i, 7) * 10.0,
                nz(i, 8) * 0.5,
            ),
            120..=134 => (
                200.0 + nz(i, 9) * 10.0,
                80.0 + nz(i, 10) * 5.0,
                5.0 + nz(i, 11),
            ),
            _ => (
                100.0 + nz(i, 12) * 10.0,
                100.0 + nz(i, 13) * 10.0,
                nz(i, 14) * 0.5,
            ),
        };
        bars.push(VolumeBar {
            ts,
            bar_index: i as u64,
            cum_vol,
            bar_vol,
            mid_price: base_price,
            best_bid: base_price - 2.0,
            best_ask: base_price + 2.0,
            spread_bps: 4.0 / base_price * 10000.0,
            full_imbalance: (ask_vol - bid_vol) / (ask_vol + bid_vol).max(1.0),
            top5_imbalance: (ask_vol - bid_vol) / (ask_vol + bid_vol).max(1.0),
            weighted_imbalance: 0.0,
            total_bid_vol: bid_vol * 3.0,
            total_ask_vol: ask_vol * 3.0,
            bid_vol_top5: bid_vol * 0.4,
            ask_vol_top5: ask_vol * 0.4,
            bid_vol_top10: bid_vol,
            ask_vol_top10: ask_vol,
            depth_breadth_bid: bid_vol * 0.05,
            depth_breadth_ask: ask_vol * 0.05,
            mean_bid_gap_bps: 1.0,
            mean_ask_gap_bps: 1.0,
            cvd_delta: cvd,
            cvd_ratio: 0.5,
            cvd_momentum: cvd * 0.3,
            cvd_acceleration: cvd * 0.1,
            trade_count: 200,
            buy_count: 100,
            sell_count: 100,
            aggressor_ratio: (100.0 + cvd) / 200.0,
            large_buy_count: 20,
            large_sell_count: 20,
            large_buy_vol: bid_vol * 0.3,
            large_sell_vol: ask_vol * 0.3,
            large_aggressor_ratio: 0.5,
            max_trade_size: 8.0,
            trade_intensity: 200.0 / bar_vol.max(1.0),
            liq_imbalance: nz(i, 21) * 0.5,
            funding_rate: 0.0,
            oi_pct_change: 0.0,
        });
    }
    bars
}

fn nz(idx: usize, seed: usize) -> f64 {
    let x = idx
        .wrapping_mul(6364136223846793005)
        .wrapping_add(seed.wrapping_mul(1442695040888963407)) as u64;
    let y = x ^ (x >> 33);
    ((y.wrapping_mul(0xBF58476D1CE4E5B9) ^ (y >> 33)).wrapping_mul(0x94D049BB133111EB) as f64
        / u64::MAX as f64)
        - 0.5
}

// ── Causal mode ───────────────────────────────────────────────────────────────────

/// Map `CausalDirection` to the legacy `SignalDirection` enum so the same
/// downstream report columns render correctly.
fn causal_dir_to_signal(d: CausalDirection) -> SignalDirection {
    match d {
        CausalDirection::Long => SignalDirection::Long,
        CausalDirection::Short => SignalDirection::Short,
        CausalDirection::Neutral => SignalDirection::Neutral,
    }
}

/// Run the CausalEngine over `bars` and emit a legacy-schema report
/// (signal table + summary statistics) to `out`.
///
/// The schema matches the legacy path:
///   `# bar ts type dir conf maha move hold pattern`
/// with two additions:
///   `steps` (template completeness, e.g. 3/3) and `template` (template id).
fn run_causal_mode(
    out: &mut Box<dyn Write>,
    bars: &[VolumeBar],
    cfg: &EngineConfig,
    desc: &str,
    seen_earliest_ts: u64,
    seen_latest_ts: u64,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut engine = CausalEngine::new(cfg.clone());
    let mut all_signals: Vec<CausalSignal> = Vec::new();
    let mut anomaly_counts: std::collections::HashMap<String, u64> =
        std::collections::HashMap::new();

    for bar in bars {
        let output: CausalEngineOutput = engine.on_bar(bar);
        // Causal mode has no per-feature z-score events; we just count template
        // completions for visibility.  Anomaly kinds map to template steps.
        for sig in &output.signals {
            anomaly_counts
                .entry(sig.template_id.to_string())
                .or_insert(0);
            *anomaly_counts.get_mut(sig.template_id).unwrap() += 1;
        }
        all_signals.extend(output.signals);
    }

    // ── Header ──
    writeln!(out)?;
    writeln!(
        out,
        "═══════════════════════════════════════════════════════════════"
    )?;
    writeln!(
        out,
        "  forge-anomaly validate  |  source: {}  |  engine: CAUSAL",
        desc
    )?;
    let early = if seen_earliest_ts < u64::MAX {
        format_ts_date_only(seen_earliest_ts)
    } else {
        "?".into()
    };
    let late = if seen_latest_ts > 0 {
        format_ts_date_only(seen_latest_ts)
    } else {
        "?".into()
    };
    writeln!(
        out,
        "  dates: {} → {}  |  bars: {}",
        early,
        late,
        bars.len()
    )?;
    writeln!(
        out,
        "  causal: lookback={}  rate_limit={:.1}/100  template=absorption_reversal",
        cfg.causal.lookback_bars, cfg.causal.signal_rate_limit
    )?;
    writeln!(
        out,
        "  absorption_reversal: cvd_thr={:.2}*σ  abs_thr={:.2}  decel_ratio={:.2}  depth_pre≥{:.2}  max_bars={}  hold={}",
        cfg.causal.absorption_reversal.cvd_pressure_threshold,
        cfg.causal.absorption_reversal.absorption_hold_threshold,
        cfg.causal.absorption_reversal.deceleration_ratio,
        cfg.causal.absorption_reversal.depth_precondition_min,
        cfg.causal.absorption_reversal.max_step1_to_signal_bars,
        cfg.causal.absorption_reversal.hold_bars,
    )?;
    writeln!(
        out,
        "═══════════════════════════════════════════════════════════════"
    )?;
    writeln!(out)?;

    // ── Signal table ──
    if all_signals.is_empty() {
        writeln!(out, "  NO SIGNALS generated (causal mode).")?;
        writeln!(out, "  Template absorption_reversal did not complete a chain on this data.")?;
    } else {
        writeln!(
            out,
            "── Signal Table ─────────────────────────────────────────────"
        )?;
        writeln!(
            out,
            "  {:>3} {:>6} {:>19} {:>10} {:>8} {:>6.3} {:>6.1} {:>6.1} {:>5} {:>5} {}",
            "#", "bar", "ts", "template", "dir", "conf", "maha", "move", "hold", "steps", "pat"
        )?;
        writeln!(
            out,
            "  {} {} {} {} {} {} {} {} {} {} {}",
            "---",
            "------",
            "-------------------",
            "----------",
            "--------",
            "-------",
            "-------",
            "-------",
            "-----",
            "-----",
            "---"
        )?;
        for (idx, sig) in all_signals.iter().enumerate() {
            let dir_label = match sig.direction {
                CausalDirection::Long => "long",
                CausalDirection::Short => "short",
                CausalDirection::Neutral => "neutral",
            };
            let sig_dir = causal_dir_to_signal(sig.direction);
            // Causal mode has no PatternRepeat.  Always show empty for the
            // pattern column so A/B diffs align with legacy output.
            writeln!(
                out,
                "  {:>3} {:>6} {:>19} {:>10} {:>8} {:>6.3} {:>6.1} {:>6.1} {:>5} {:>5} {:>3}",
                idx,
                sig.bar_index,
                format_ts(sig.ts),
                sig.template_id,
                dir_label,
                sig.confidence,
                0.0, // no maha in causal mode
                sig.expected_move_bps,
                sig.hold_bars,
                format!("{}/{}", sig.steps_passed, sig.steps_total),
                "",
            )?;
            // Touch sig_dir to silence unused warning if compiler complains.
            let _ = sig_dir;
        }
        writeln!(out)?;
    }

    // ── Statistics ──
    writeln!(
        out,
        "── Summary Statistics ───────────────────────────────────────"
    )?;
    writeln!(out, "  bars processed:             {:>6}", bars.len())?;
    writeln!(
        out,
        "  total signals:              {:>6}  ({:.1}% of bars)",
        all_signals.len(),
        all_signals.len() as f64 / bars.len().max(1) as f64 * 100.0
    )?;
    // In causal mode there is no PatternRepeat / maha split.  We print
    // zeroed entries so A/B column counts match.
    writeln!(
        out,
        "    causal template:          {:>6}  ({:.1}% of signals)  ← KEY METRIC",
        all_signals.len(),
        if all_signals.is_empty() {
            0.0
        } else {
            100.0
        }
    )?;
    writeln!(
        out,
        "    pattern-based (legacy):   {:>6}  ({:.1}% of signals)",
        0,
        0.0
    )?;
    writeln!(out)?;

    if !all_signals.is_empty() {
        let n = all_signals.len() as f64;
        let avg_c = all_signals.iter().map(|s| s.confidence).sum::<f64>() / n;
        let avg_mv = all_signals.iter().map(|s| s.expected_move_bps).sum::<f64>() / n;
        let avg_h = all_signals.iter().map(|s| s.hold_bars as f64).sum::<f64>() / n;
        let min_c = all_signals
            .iter()
            .map(|s| s.confidence)
            .fold(f64::INFINITY, f64::min);
        let max_c = all_signals
            .iter()
            .map(|s| s.confidence)
            .fold(0.0_f64, f64::max);
        writeln!(
            out,
            "  avg confidence:             {:.3}  (range {:.3} – {:.3})",
            avg_c, min_c, max_c
        )?;
        writeln!(out, "  avg expected move:          {:.1} bps", avg_mv)?;
        writeln!(out, "  avg hold bars:              {:.0}", avg_h)?;
        writeln!(out, "  avg maha_dist (n/a):        —")?;

        let lng = all_signals
            .iter()
            .filter(|s| matches!(s.direction, CausalDirection::Long))
            .count();
        let sht = all_signals
            .iter()
            .filter(|s| matches!(s.direction, CausalDirection::Short))
            .count();
        writeln!(out, "  reversal:  {}  momentum: {}  long: {}  short: {}",
            all_signals.len(), 0, lng, sht)?;
    }

    writeln!(out)?;
    writeln!(out, "── Template Completeness ────────────────────────────────────")?;
    if all_signals.is_empty() {
        writeln!(out, "  (no completions)")?;
    } else {
        let mut sorted: Vec<&CausalSignal> = all_signals.iter().collect();
        sorted.sort_by(|a, b| b.confidence.partial_cmp(&a.confidence).unwrap_or(std::cmp::Ordering::Equal));
        for (i, s) in sorted.iter().take(10).enumerate() {
            let dir_label = match s.direction {
                CausalDirection::Long => "long",
                CausalDirection::Short => "short",
                CausalDirection::Neutral => "neutral",
            };
            writeln!(
                out,
                "  #{:>2} bar={:<6} dir={:<6} conf={:.3} steps={}/{} bars_in_story={}  {}",
                i, s.bar_index, dir_label, s.confidence,
                s.steps_passed, s.steps_total, s.bars_in_story,
                s.description,
            )?;
        }
    }

    Ok(())
}
