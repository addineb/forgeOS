//! Phase 2 validation binary: runs AnomalyEngine on real CSV data with date filtering.
//!
//! Usage:
//!   cargo run --bin validate                                # synthetic data mode
//!   cargo run --bin validate -- --input stitched_vb10.csv   # full file
//!   cargo run --bin validate -- --input data.csv --date 2026-06-05

use std::io;
use std::path::PathBuf;

use forge_anomaly::{
    load_volume_bars, AnomalyEngine, AnomalyKind, AnomalySignal, EngineConfig, SignalDirection,
    SignalType, VolumeBar,
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    let (bars, cfg, desc) = if let Some(ref path) = cli.input {
        let all = load_volume_bars(path)?;
        let filtered = if let Some(ref date) = cli.date {
            filter_by_date(all, date)?
        } else {
            all
        };
        let d = cli.date.as_deref().unwrap_or("all dates");
        eprintln!("  loaded {} bars from {} (date: {})", filtered.len(), path.display(), d);
        (filtered, EngineConfig::default(), format!("{}", path.display()))
    } else {
        eprintln!("  using synthetic data (relaxed thresholds)");
        let b = synthetic_bars(300);
        let c = EngineConfig {
            mahalanobis_threshold: 2.5,
            min_confidence: 0.45,
            ..EngineConfig::default()
        };
        (b, c, "synthetic".to_string())
    };

    if bars.is_empty() {
        eprintln!("  no bars to process");
        return Ok(());
    }

    let mut engine = AnomalyEngine::new(cfg.clone());
    let mut all_signals: Vec<AnomalySignal> = Vec::new();
    let mut total_maha = 0.0;
    let mut maha_count = 0u64;
    let mut anomaly_counts: std::collections::HashMap<AnomalyKind, u64> = std::collections::HashMap::new();

    for bar in &bars {
        let output = engine.on_bar(bar);
        if output.mahalanobis_dist > 0.0 {
            total_maha += output.mahalanobis_dist;
            maha_count += 1;
        }
        for ev in &output.anomalies {
            *anomaly_counts.entry(ev.kind).or_insert(0) += 1;
        }
        if let Some(sig) = output.signal {
            all_signals.push(sig);
        }
    }

    let has_pattern = |sig: &AnomalySignal| -> bool {
        sig.events.iter().any(|e| e.kind == AnomalyKind::PatternRepeat)
    };
    let pattern_signals: Vec<_> = all_signals.iter().filter(|s| has_pattern(s)).collect();
    let maha_only: Vec<_> = all_signals.iter().filter(|s| !has_pattern(s)).collect();

    println!("=== Phase 2: Real Data Validation ===");
    println!("  source:    {}", desc);
    println!("  lookback={}  maha_thresh={}  min_conf={:.2}  fee={}bps",
        cfg.lookback_bars, cfg.mahalanobis_threshold, cfg.min_confidence, cfg.fee_bps);
    println!("  bars:      {}", bars.len());
    println!();

    if all_signals.is_empty() {
        println!("NO SIGNALS generated.");
        let avg = if maha_count > 0 { total_maha / maha_count as f64 } else { 0.0 };
        println!("  avg maha_dist = {:.3} (need ≥ {})", avg, cfg.mahalanobis_threshold);
    } else {
        println!("── Signals ({}) ───────────────────────────────────────", all_signals.len());
        for (idx, sig) in all_signals.iter().take(20).enumerate() {
            let ts_fmt = format_ts(sig.ts);
            let pat_label = if has_pattern(sig) { " [PATTERN]" } else { "" };
            println!(
                "  #{:<2} bar={:<5} ts={} {:?}/{:?} conf={:.3} maha={:.2}{}",
                idx, sig.bar_index, ts_fmt, sig.signal_type, sig.direction,
                sig.confidence, sig.mahalanobis_dist, pat_label,
            );
            println!(
                "       move={:.2}bps hold={} pat_count={} null={}",
                sig.expected_move_bps, sig.hold_bars, sig.pattern_count, sig.passed_null_edge,
            );
        }
        if all_signals.len() > 20 {
            println!("  ... {} more signals (truncated)", all_signals.len() - 20);
        }
    }

    println!();
    println!("── Statistics ───────────────────────────────────────────");
    println!("  bars processed:           {}", bars.len());
    println!("  total signals:            {}", all_signals.len());
    println!(
        "    mahalanobis only:       {} ({:.1}%)",
        maha_only.len(),
        if all_signals.is_empty() { 0.0 } else { maha_only.len() as f64 / all_signals.len() as f64 * 100.0 }
    );
    println!(
        "    pattern-based:          {} ({:.1}%)",
        pattern_signals.len(),
        if all_signals.is_empty() { 0.0 } else { pattern_signals.len() as f64 / all_signals.len() as f64 * 100.0 }
    );
    println!(
        "  signal rate:              {:.1}%",
        all_signals.len() as f64 / bars.len() as f64 * 100.0
    );
    let avg_m = if maha_count > 0 { total_maha / maha_count as f64 } else { 0.0 };
    println!("  avg maha_dist (non-zero): {:.3}", avg_m);

    if !all_signals.is_empty() {
        let n = all_signals.len();
        let avg_c = all_signals.iter().map(|s| s.confidence).sum::<f64>() / n as f64;
        let avg_mv = all_signals.iter().map(|s| s.expected_move_bps).sum::<f64>() / n as f64;
        println!("  avg confidence:           {:.3}", avg_c);
        println!("  avg expected move:        {:.2} bps", avg_mv);
        let rev = all_signals.iter().filter(|s| matches!(s.signal_type, SignalType::Reversal)).count();
        let mom = all_signals.iter().filter(|s| matches!(s.signal_type, SignalType::MomentumContinuation)).count();
        let lng = all_signals.iter().filter(|s| matches!(s.direction, SignalDirection::Long)).count();
        let sht = all_signals.iter().filter(|s| matches!(s.direction, SignalDirection::Short)).count();
        println!("  reversal: {}  momentum: {}  long: {}  short: {}", rev, mom, lng, sht);
    }

    println!();
    println!("── Anomaly Kind Totals ──────────────────────────────────");
    let mut sorted: Vec<_> = anomaly_counts.into_iter().collect();
    sorted.sort_by_key(|(_, c)| -( *c as i64));
    for (kind, count) in &sorted {
        println!("  {:?}: {}", kind, count);
    }

    Ok(())
}

/// ── CLI ──────────────────────────────────────────────────────────────────────

struct Cli {
    input: Option<PathBuf>,
    date: Option<String>,
}

impl Cli {
    fn parse() -> Self {
        let args: Vec<String> = std::env::args().collect();
        let mut input = None;
        let mut date = None;
        let mut i = 1;
        while i < args.len() {
            match args[i].as_str() {
                "--input" => {
                    if i + 1 < args.len() { input = Some(PathBuf::from(&args[i + 1])); i += 1; }
                }
                "--date" => {
                    if i + 1 < args.len() { date = Some(args[i + 1].clone()); i += 1; }
                }
                _ => {}
            }
            i += 1;
        }
        Self { input, date }
    }
}

/// ── Date filtering ───────────────────────────────────────────────────────────

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
    // Simple: parse YYYY-MM-DD and convert to ns since epoch estimate.
    // The CSV timestamps are ns; for BTCUSDT we assume they're UTC.
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
    let yoe = y.rem_euclid(400) as i64;
    let doy = (153 * m + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let epoch_days = 719468; // 1970-01-01 in days since 0000-03-01
    era * 146097 + doe - epoch_days
}

/// ── Timestamp formatting ─────────────────────────────────────────────────────

fn format_ts(ns: u64) -> String {
    let secs = ns / 1_000_000_000;
    let days = secs / 86400;
    let time = secs % 86400;
    let h = time / 3600;
    let m = (time % 3600) / 60;
    let s = time % 60;
    let epoch_days = days as i64;
    // Convert epoch days back to date
    let (y, mo, d) = epoch_to_date(epoch_days);
    format!("{:04}-{:02}-{:02} {:02}:{:02}:{:02}", y, mo, d, h, m, s)
}

fn epoch_to_date(days: i64) -> (i64, i64, i64) {
    let z = days + 719468;
    let era = if z >= 0 { z / 146097 } else { (z - 146096) / 146097 };
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

/// ── Synthetic data fallback ──────────────────────────────────────────────────

fn synthetic_bars(n: usize) -> Vec<VolumeBar> {
    let mut bars = Vec::with_capacity(n);
    let base_price = 50_000.0;
    let mut cum_vol = 0.0;
    for i in 0..n {
        let ts = (i as u64) * 30_000_000_000;
        let bar_vol = 10.0 + (i % 5) as f64 * 2.0;
        cum_vol += bar_vol;
        let (bid_vol, ask_vol, cvd) = match i {
            0..=79 => (100.0 + nz(i, 0) * 10.0, 100.0 + nz(i, 1) * 10.0, nz(i, 2) * 0.5),
            80..=94 => (30.0 + nz(i, 3) * 5.0, 250.0 + nz(i, 4) * 10.0, -15.0 + nz(i, 5) * 2.0),
            95..=119 => (100.0 + nz(i, 6) * 10.0, 100.0 + nz(i, 7) * 10.0, nz(i, 8) * 0.5),
            120..=134 => (200.0 + nz(i, 9) * 10.0, 80.0 + nz(i, 10) * 5.0, 5.0 + nz(i, 11)),
            _ => (100.0 + nz(i, 12) * 10.0, 100.0 + nz(i, 13) * 10.0, nz(i, 14) * 0.5),
        };
        bars.push(VolumeBar {
            ts, bar_index: i as u64, cum_vol, bar_vol,
            mid_price: base_price, best_bid: base_price - 2.0, best_ask: base_price + 2.0,
            spread_bps: 4.0 / base_price * 10000.0,
            full_imbalance: (ask_vol - bid_vol) / (ask_vol + bid_vol).max(1.0),
            top5_imbalance: (ask_vol - bid_vol) / (ask_vol + bid_vol).max(1.0),
            weighted_imbalance: 0.0,
            total_bid_vol: bid_vol * 3.0, total_ask_vol: ask_vol * 3.0,
            bid_vol_top5: bid_vol * 0.4, ask_vol_top5: ask_vol * 0.4,
            bid_vol_top10: bid_vol, ask_vol_top10: ask_vol,
            depth_breadth_bid: bid_vol * 0.05, depth_breadth_ask: ask_vol * 0.05,
            mean_bid_gap_bps: 1.0, mean_ask_gap_bps: 1.0,
            cvd_delta: cvd, cvd_ratio: 0.5, cvd_momentum: cvd * 0.3, cvd_acceleration: cvd * 0.1,
            trade_count: 200, buy_count: 100, sell_count: 100,
            aggressor_ratio: (100.0 + cvd) / 200.0,
            large_buy_count: 20, large_sell_count: 20,
            large_buy_vol: bid_vol * 0.3, large_sell_vol: ask_vol * 0.3,
            large_aggressor_ratio: 0.5, max_trade_size: 8.0,
            trade_intensity: 200.0 / bar_vol.max(1.0),
            liq_imbalance: nz(i, 21) * 0.5, funding_rate: 0.0, oi_pct_change: 0.0,
        });
    }
    bars
}

fn nz(idx: usize, seed: usize) -> f64 {
    let x = idx.wrapping_mul(6364136223846793005).wrapping_add(seed.wrapping_mul(1442695040888963407)) as u64;
    let y = x ^ (x >> 33);
    ((y.wrapping_mul(0xBF58476D1CE4E5B9) ^ (y >> 33)).wrapping_mul(0x94D049BB133111EB) as f64 / u64::MAX as f64) - 0.5
}
