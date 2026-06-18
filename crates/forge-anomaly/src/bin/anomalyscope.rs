use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

use clap::Parser;
use forge_anomaly::{
    load_volume_bars, load_volume_bars_with_fwd, AnomalyEngine, AnomalyKind, DetectionMethod,
    EngineConfig, EvalRow, SignalDirection, evaluate,
};

#[derive(Parser, Debug)]
#[command(name = "anomalyscope", about = "Replay depthscope CSVs through forge-anomaly")]
struct Args {
    #[arg(long)]
    input: PathBuf,
    #[arg(long, default_value = "anomaly_signals.csv")]
    output: PathBuf,
    #[arg(long, default_value_t = 50)]
    lookback_bars: usize,
    #[arg(long, default_value_t = 4.0)]
    mahalanobis_threshold: f64,
    #[arg(long, default_value_t = 0.65)]
    isolation_threshold: f64,
    #[arg(long, default_value = "mahalanobis")]
    method: String,
    #[arg(long, default_value_t = 9.0)]
    fee_bps: f64,
    #[arg(long, default_value_t = false)]
    verbose: bool,
    #[arg(long, default_value_t = false)]
    backtest: bool,
    #[arg(long, default_value_t = 3.0)]
    expected_move_maha_coeff: f64,
    #[arg(long, default_value_t = 20.0)]
    expected_move_iso_coeff: f64,
    #[arg(long, default_value_t = 0.05)]
    fdr_alpha: f64,
    #[arg(long, default_value_t = 100)]
    null_edge_permutations: u32,
    #[arg(long, default_value_t = 0.25)]
    null_edge_margin: f64,
    #[arg(long, default_value_t = 8.0)]
    max_signals_per_100_bars: f64,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let method = parse_method(&args.method);

    eprintln!("anomalyscope: forge-anomaly v0.2");
    eprintln!("  input:  {}", args.input.display());
    eprintln!("  method: {:?}", method);

    if args.backtest {
        return run_backtest(&args, method);
    }

    let bars = load_volume_bars(&args.input)?;
    eprintln!("  loaded {} bars", bars.len());
    if bars.is_empty() {
        return Ok(());
    }

    let cfg = EngineConfig {
        lookback_bars: args.lookback_bars,
        mahalanobis_threshold: args.mahalanobis_threshold,
        isolation_threshold: args.isolation_threshold,
        method,
        fee_bps: args.fee_bps,
        expected_move_maha_coeff: args.expected_move_maha_coeff,
        expected_move_iso_coeff: args.expected_move_iso_coeff,
        fdr_alpha: args.fdr_alpha,
        null_edge_permutations: args.null_edge_permutations,
        null_edge_margin: args.null_edge_margin,
        max_signals_per_100_bars: args.max_signals_per_100_bars,
        ..EngineConfig::default()
    };

    let mut engine = AnomalyEngine::new(cfg);
    let outputs = engine.on_bars(&bars);

    let mut out = File::create(&args.output)?;
    let mut n_signals = 0usize;

    if args.verbose {
        writeln!(
            out,
            "bar_index,ts,row_type,kind,direction,signal_type,z_score,confidence,\
             maha_dist,iso_score,description,mid_price"
        )?;
    } else {
        writeln!(
            out,
            "bar_index,ts,direction,signal_type,confidence,maha_dist,iso_score,\
             expected_move_bps,hold_bars,pattern_count,null_edge,description,mid_price"
        )?;
    }

    for (i, output) in outputs.iter().enumerate() {
        let bar = &bars[i];
        if args.verbose {
            for ev in &output.anomalies {
                writeln!(
                    out,
                    "{},{},anomaly,{},{},,{:.4},{:.4},{:.4},{:.4},,{}",
                    ev.bar_index,
                    ev.ts,
                    kind_str(ev.kind),
                    dir_str(ev.direction),
                    ev.z_score,
                    ev.confidence,
                    output.mahalanobis_dist,
                    output.isolation_score,
                    bar.mid_price,
                )?;
            }
        }
        if let Some(sig) = &output.signal {
            n_signals += 1;
            if args.verbose {
                writeln!(
                    out,
                    "{},{},signal,,{},{},{:.4},{:.4},{:.4},\"{}\",{}",
                    sig.bar_index,
                    sig.ts,
                    dir_str(sig.direction),
                    sig.signal_type,
                    sig.confidence,
                    sig.mahalanobis_dist,
                    sig.isolation_score,
                    sig.description.replace('"', "'"),
                    bar.mid_price,
                )?;
            } else {
                writeln!(
                    out,
                    "{},{},{},{},{:.4},{:.4},{:.4},{:.2},{},{},{},\"{}\",{}",
                    sig.bar_index,
                    sig.ts,
                    dir_str(sig.direction),
                    sig.signal_type,
                    sig.confidence,
                    sig.mahalanobis_dist,
                    sig.isolation_score,
                    sig.expected_move_bps,
                    sig.hold_bars,
                    sig.pattern_count,
                    sig.passed_null_edge,
                    sig.description.replace('"', "'"),
                    bar.mid_price,
                )?;
            }
        }
    }

    eprintln!("  signals: {} / {} bars", n_signals, bars.len());
    eprintln!("  wrote {}", args.output.display());
    Ok(())
}

fn run_backtest(args: &Args, method: DetectionMethod) -> Result<(), Box<dyn std::error::Error>> {
    let pairs = load_volume_bars_with_fwd(&args.input)?;
    eprintln!("  loaded {} bars (with fwd returns)", pairs.len());
    if pairs.is_empty() {
        return Ok(());
    }

    let bars: Vec<_> = pairs.iter().map(|(b, _)| *b).collect();

    let cfg = EngineConfig {
        lookback_bars: args.lookback_bars,
        mahalanobis_threshold: args.mahalanobis_threshold,
        isolation_threshold: args.isolation_threshold,
        method,
        fee_bps: args.fee_bps,
        expected_move_maha_coeff: args.expected_move_maha_coeff,
        expected_move_iso_coeff: args.expected_move_iso_coeff,
        fdr_alpha: args.fdr_alpha,
        null_edge_permutations: args.null_edge_permutations,
        null_edge_margin: args.null_edge_margin,
        max_signals_per_100_bars: args.max_signals_per_100_bars,
        ..EngineConfig::default()
    };

    let mut engine = AnomalyEngine::new(cfg);
    let outputs = engine.on_bars(&bars);

    let mut eval_rows: Vec<EvalRow> = Vec::new();

    for (i, output) in outputs.iter().enumerate() {
        if let Some(sig) = &output.signal {
            let (_, ref fwd) = pairs[i];
            eval_rows.push(EvalRow {
                bar_index: sig.bar_index,
                ts: sig.ts,
                direction: sig.direction,
                signal_type: sig.signal_type,
                confidence: sig.confidence,
                expected_move_bps: sig.expected_move_bps,
                hold_bars: sig.hold_bars,
                pattern_count: sig.pattern_count,
                maha_dist: sig.mahalanobis_dist,
                iso_score: sig.isolation_score,
                fwd: *fwd,
            });
        }
    }

    let stats = evaluate(&eval_rows);
    eprintln!("{}", stats);

    let (maha_c, iso_c) = forge_anomaly::calibrate_expected_move(&eval_rows);
    eprintln!("  calibration: suggested maha_coeff={:.3} iso_coeff={:.3}", maha_c, iso_c);

    let backtest_path = PathBuf::from(args.output.with_extension("").to_string_lossy().to_string() + "_backtest.csv");
    let mut out = File::create(&backtest_path)?;
    writeln!(
        out,
        "bar_index,ts,direction,signal_type,confidence,fwd_ret_15m_bps,fwd_ret_1h_bps,fwd_ret_4h_bps,signed_ret_1h_bps,expected_move_bps"
    )?;
    for r in &eval_rows {
        let signed_1h = match r.direction {
            SignalDirection::Long => r.fwd.fwd_ret_1h_bps,
            SignalDirection::Short => -r.fwd.fwd_ret_1h_bps,
            SignalDirection::Neutral => 0.0,
        };
        writeln!(
            out,
            "{},{},{},{},{:.4},{:.4},{:.4},{:.4},{:.4},{:.4}",
            r.bar_index,
            r.ts,
            dir_str(r.direction),
            r.signal_type,
            r.confidence,
            r.fwd.fwd_ret_15m_bps,
            r.fwd.fwd_ret_1h_bps,
            r.fwd.fwd_ret_4h_bps,
            signed_1h,
            r.expected_move_bps,
        )?;
    }
    eprintln!("  wrote {}", backtest_path.display());
    Ok(())
}

fn parse_method(s: &str) -> DetectionMethod {
    match s.to_lowercase().as_str() {
        "isolation" | "isolation_forest" | "if" => DetectionMethod::IsolationForest,
        "both" => DetectionMethod::Both,
        "either" => DetectionMethod::Either,
        _ => DetectionMethod::Mahalanobis,
    }
}

fn kind_str(k: AnomalyKind) -> &'static str {
    match k {
        AnomalyKind::Ofi => "ofi",
        AnomalyKind::Cvd => "cvd",
        AnomalyKind::DepthImbalance => "depth_imbalance",
        AnomalyKind::Absorption => "absorption",
        AnomalyKind::LiquidityVacuum => "liquidity_vacuum",
        AnomalyKind::VolDeltaDivergence => "vol_delta_divergence",
        AnomalyKind::AggressorImbalance => "aggressor_imbalance",
        AnomalyKind::LargePrint => "large_print",
        AnomalyKind::TradeIntensity => "trade_intensity",
        AnomalyKind::PatternRepeat => "pattern_repeat",
    }
}

fn dir_str(d: SignalDirection) -> &'static str {
    match d {
        SignalDirection::Long => "long",
        SignalDirection::Short => "short",
        SignalDirection::Neutral => "neutral",
    }
}
