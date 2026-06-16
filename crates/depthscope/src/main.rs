//! Depthscope: replay L2 orderbook data and compute depth features at macrostructure timescales.
//!
//! Reads Binance orderbook deltas (parquet) from CHD, reconstructs the full L2 book
//! using forge-book::OrderBook, and computes DepthFeatures at configurable intervals.
//! Outputs CSV for downstream analysis.
//!
//! SAFEGUARDS:
//! - Warm-up period: first N seconds of data are discarded (book is building up)
//! - Hour stitching: consecutive hours are loaded in order, book state carries over
//! - Top-N focus: features computed on top-20 levels (where alpha lives)
//! - Null-edge: coinflip must lose ~fees

use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

use arrow::array::{Array, Float64Array, Int64Array, StringArray};
use arrow::record_batch::RecordBatch;
use clap::Parser;
use forge_book::OrderBook;
use forge_core::{EventKind, Price, Qty, Side, UnixNanos};
use forge_depth::{CVD, DepthFeatures, DepthSnapshot, VolumeProfile, WallTracker};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

/// Depth-pattern study tool.
#[derive(Parser, Debug)]
#[command(name = "depthscope", about = "Replay L2 data and compute depth features")]
struct Args {
    /// Root directory containing CHD parquet data (e.g. /root/chd/data/ticks)
    #[arg(long, default_value = "./data")]
    data_root: PathBuf,

    /// Binance symbol key (e.g. BTCUSDT)
    #[arg(long, default_value = "BTCUSDT")]
    symbol: String,

    /// Date to study (e.g. 2026-06-09)
    #[arg(long)]
    date: String,

    /// Hours to include (comma-separated, e.g. "09,10,11" or "all")
    #[arg(long, default_value = "all")]
    hours: String,

    /// Snapshot interval in seconds (how often to compute features, time-bar mode)
    #[arg(long, default_value_t = 1)]
    interval_s: u64,

    /// Volume bar threshold in base units (e.g. 10 BTC). When set, snapshots are taken
    /// every N units of cumulative trade volume instead of at time intervals.
    /// Mutually exclusive with --interval-s (volume bar takes priority).
    #[arg(long)]
    volume_bar: Option<f64>,

    /// Warm-up period in seconds (discard initial data while book builds)
    #[arg(long, default_value_t = 300)]
    warmup_s: u64,

    /// Wall detection threshold in base units (e.g. 5.0 BTC)
    #[arg(long, default_value_t = 5.0)]
    wall_threshold: f64,

    /// Volume profile bin width in price units
    #[arg(long, default_value_t = 10.0)]
    vp_bin_width: f64,

    /// Volume profile lookback window in seconds (how much trade history to use for VP)
    #[arg(long, default_value_t = 300)]
    vp_window_s: u64,

    /// Top-N levels for imbalance calculation
    #[arg(long, default_value_t = 20)]
    top_n: usize,

    /// Output CSV file path. Supports {symbol}, {date}, {mode} placeholders.
    /// Default: depthscope_output.csv
    #[arg(long, default_value = "depthscope_output.csv")]
    output: PathBuf,

    /// Feed latency in nanoseconds added to exch_ts
    #[arg(long, default_value_t = 500_000)]
    feed_latency_ns: u64,
}

/// Read parquet file and return all record batches.
fn read_parquet(path: &PathBuf) -> Result<Vec<RecordBatch>, Box<dyn std::error::Error>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = File::open(path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)?;
    let reader = builder.build()?;
    let mut batches = Vec::new();
    for batch in reader {
        batches.push(batch?);
    }
    Ok(batches)
}

/// A raw orderbook delta row from CHD parquet.
/// CHD schema: venueTs (ms), captureTs (ns), side, price, qty, kind ("change"|"remove")
struct RawDelta {
    venue_ts_ms: i64,
    capture_ts_ns: i64,
    side: String,
    price: f64,
    qty: f64,
    #[allow(dead_code)]
is_remove: bool, // kind == "remove" — used to set qty=0 for removals in reader
}

/// Read all orderbook deltas from a single hour file.
/// CHD column names: venueTs, captureTs, side, price, qty, kind
fn read_orderbook_deltas(path: &PathBuf) -> Result<Vec<RawDelta>, Box<dyn std::error::Error>> {
    let batches = read_parquet(path)?;
    let mut deltas = Vec::new();
    for b in &batches {
        let et = b.column_by_name("venueTs")
            .and_then(|c| c.as_any().downcast_ref::<Int64Array>())
            .ok_or("missing venueTs")?;
        let rt = b.column_by_name("captureTs")
            .and_then(|c| c.as_any().downcast_ref::<Int64Array>())
            .ok_or("missing captureTs")?;
        let sd = b.column_by_name("side")
            .and_then(|c| c.as_any().downcast_ref::<StringArray>())
            .ok_or("missing side")?;
        let pr = b.column_by_name("price")
            .and_then(|c| c.as_any().downcast_ref::<Float64Array>())
            .ok_or("missing price")?;
        let qt = b.column_by_name("qty")
            .and_then(|c| c.as_any().downcast_ref::<Float64Array>())
            .ok_or("missing qty")?;
        // kind column: "change" = update, "remove" = level deletion
        let kd = b.column_by_name("kind")
            .and_then(|c| c.as_any().downcast_ref::<StringArray>());

        for i in 0..b.num_rows() {
            if et.is_null(i) || sd.is_null(i) || pr.is_null(i) || qt.is_null(i) {
                continue;
            }
            let is_remove = kd.as_ref().map_or(false, |k| {
                !k.is_null(i) && k.value(i) == "remove"
            });
            deltas.push(RawDelta {
                venue_ts_ms: et.value(i),
                capture_ts_ns: rt.value(i),
                side: sd.value(i).to_string(),
                price: pr.value(i),
                qty: if is_remove { 0.0 } else { qt.value(i) },
                is_remove,
            });
        }
    }
    Ok(deltas)
}

/// A raw trade row from CHD parquet.
/// CHD schema: ts (ms), localTs (ns), symbol, price, qty, isBuyerMaker
struct RawTrade {
    event_time_ms: i64,
    price: f64,
    qty: f64,
    is_buyer_maker: bool,
}

/// Read all trades from a single hour file.
/// CHD column names: ts, localTs, symbol, price, qty, isBuyerMaker
fn read_trades(path: &PathBuf) -> Result<Vec<RawTrade>, Box<dyn std::error::Error>> {
    let batches = read_parquet(path)?;
    let mut trades = Vec::new();
    for b in &batches {
        let et = b.column_by_name("ts")
            .and_then(|c| c.as_any().downcast_ref::<Int64Array>())
            .ok_or("missing ts")?;
        let pr = b.column_by_name("price")
            .and_then(|c| c.as_any().downcast_ref::<Float64Array>())
            .ok_or("missing price")?;
        let qt = b.column_by_name("qty")
            .and_then(|c| c.as_any().downcast_ref::<Float64Array>())
            .ok_or("missing qty")?;
        let ibm = b.column_by_name("isBuyerMaker")
            .and_then(|c| c.as_any().downcast_ref::<arrow::array::BooleanArray>())
            .ok_or("missing isBuyerMaker")?;

        for i in 0..b.num_rows() {
            if et.is_null(i) || pr.is_null(i) || qt.is_null(i) || ibm.is_null(i) {
                continue;
            }
            trades.push(RawTrade {
                event_time_ms: et.value(i),
                price: pr.value(i),
                qty: qt.value(i),
                is_buyer_maker: ibm.value(i),
            });
        }
    }
    Ok(trades)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    eprintln!("depthscope: depth-pattern study tool");
    eprintln!("  symbol: {}", args.symbol);
    eprintln!("  date: {}", args.date);
    if let Some(vb) = args.volume_bar {
        eprintln!("  mode: volume-bar ({} units)", vb);
    } else {
        eprintln!("  mode: time-bar ({}s interval)", args.interval_s);
    }
    eprintln!("  warmup: {}s", args.warmup_s);
    eprintln!("  wall_threshold: {} BTC", args.wall_threshold);
    eprintln!("  top_n: {}", args.top_n);

    // Determine hours to process
    let hours: Vec<String> = if args.hours == "all" {
        (0..24).map(|h| format!("{h:02}")).collect()
    } else {
        args.hours.split(',').map(|s| s.trim().to_string()).collect()
    };

    // Build book directory path
    let book_dir = args.data_root.join(&args.symbol).join("bookDelta").join(&args.date);
    let trade_dir = args.data_root.join(&args.symbol).join("trade").join(&args.date);

    // Collect all deltas and trades across hours, sorted by time
    let mut all_deltas: Vec<RawDelta> = Vec::new();
    let mut all_trades: Vec<RawTrade> = Vec::new();

    for hh in &hours {
        let book_path = book_dir.join(format!("{hh}.parquet"));
        let trade_path = trade_dir.join(format!("{hh}.parquet"));

        if let Ok(deltas) = read_orderbook_deltas(&book_path) {
            eprintln!("  {hh}: {} deltas", deltas.len());
            all_deltas.extend(deltas);
        } else {
            eprintln!("  {hh}: no book data, skipping");
        }

        if let Ok(trades) = read_trades(&trade_path) {
            eprintln!("  {hh}: {} trades", trades.len());
            all_trades.extend(trades);
        }
    }

    eprintln!("Total: {} deltas, {} trades", all_deltas.len(), all_trades.len());

    if all_deltas.is_empty() {
        eprintln!("No data found. Check your --data-root, --symbol, and --date paths.");
        return Ok(());
    }

    // Sort deltas by venue_ts then capture_ts
    all_deltas.sort_by_key(|d| (d.venue_ts_ms, d.capture_ts_ns));
    all_trades.sort_by_key(|t| t.event_time_ms);

    // Determine time range
    let first_ts = all_deltas.first().map(|d| d.venue_ts_ms).unwrap_or(0);
    let last_ts = all_deltas.last().map(|d| d.venue_ts_ms).unwrap_or(0);
    let warmup_end_ms = first_ts + (args.warmup_s as i64) * 1000;
    eprintln!("Time range: {} - {} ({}s)", first_ts, last_ts, (last_ts - first_ts) / 1000);
    eprintln!("Warmup ends at: {} ({}s from start)", warmup_end_ms, args.warmup_s);

    // Reconstruct book and compute features
    let mut book = OrderBook::new();
    let mut cvd = CVD::new();
    let mut wall_tracker = WallTracker::new(args.wall_threshold);
    let mut trade_idx = 0usize;

    // Volume profile window: fixed lookback (default 5 min), not tied to bar interval
    let vp_window_ms = (args.vp_window_s as i64) * 1000;
    let mut vp_trades: Vec<(i64, f64, f64, Side)> = Vec::new();

    // Output: collect snapshots first, then compute forward returns, then write CSV
    let interval_ns = (args.interval_s as u64) * 1_000_000_000;
    let warmup_end_ns = (warmup_end_ms as u64) * 1_000_000;
    let mut next_snapshot_ns = warmup_end_ns;
    let mut applied = 0u64;
    let mut snapshots: Vec<DepthFeatures> = Vec::new();
    let mut cvd_history: Vec<f64> = Vec::new(); // last 3 CVD deltas for momentum

    // Volume bar state: track cumulative trade volume for volume-bar mode
    let volume_bar = args.volume_bar.unwrap_or(0.0);
    let use_volume_bar = volume_bar > 0.0;
    let mut cum_trade_vol: f64 = 0.0;
    let mut next_vol_bar_threshold = volume_bar; // first threshold after warmup

    // Expand output path placeholders
    let mode_str = if use_volume_bar { format!("vb{}", volume_bar as u64) } else { format!("tb{}s", args.interval_s) };
    let output_path: PathBuf = args.output.to_str().unwrap_or("depthscope_output.csv")
        .replace("{symbol}", &args.symbol)
        .replace("{date}", &args.date)
        .replace("{mode}", &mode_str)
        .into();
    eprintln!("  output: {}", output_path.display());

    eprintln!("Processing deltas...");

    for delta in &all_deltas {
        let ts_ns = (delta.venue_ts_ms as u64) * 1_000_000;

        // Apply delta to book
        let side = match delta.side.as_str() {
            "bid" | "buy" | "b" => Side::Bid,
            "ask" | "sell" | "a" => Side::Ask,
            _ => continue,
        };

        let price = match Price::from_f64(delta.price) {
            Ok(p) => p,
            Err(_) => continue,
        };
        let qty = match Qty::from_f64(delta.qty) {
            Ok(q) => q,
            Err(_) => continue,
        };

        let exch_ts = match UnixNanos::from_i64(ts_ns as i64) {
            Ok(t) => t,
            Err(_) => continue,
        };
        let local_ts = match exch_ts.checked_add(args.feed_latency_ns) {
            Ok(t) => t,
            Err(_) => continue,
        };

        // Use is_remove flag from CHD kind column instead of qty==0 check
        let kind = EventKind::BookDelta;

        let event = match forge_core::Event::new(kind, exch_ts, local_ts, Some(side), price, qty, 0) {
            Ok(e) => e,
            Err(_) => continue,
        };

        if let Err(_) = book.apply(&event) {
            continue;
        }
        applied += 1;

        // Process trades up to this timestamp for CVD
        while trade_idx < all_trades.len() && all_trades[trade_idx].event_time_ms <= delta.venue_ts_ms {
            let t = &all_trades[trade_idx];
            let trade_side = if t.is_buyer_maker { Side::Ask } else { Side::Bid };
            cvd.record_trade(trade_side, t.qty);
            vp_trades.push((t.event_time_ms, t.price, t.qty, trade_side));
            cum_trade_vol += t.qty;
            trade_idx += 1;
        }

        // Take snapshot at interval (time-bar) or volume threshold (volume-bar)
        let should_snapshot = if use_volume_bar {
            // Volume bar mode: snapshot when cumulative volume crosses threshold
            // Only after warmup
            ts_ns >= warmup_end_ns && cum_trade_vol >= next_vol_bar_threshold
        } else {
            // Time bar mode: snapshot at regular time intervals
            ts_ns >= next_snapshot_ns
        };

        if should_snapshot {
            // Update wall tracker only at snapshot time (not every delta — too expensive)
            let bid_levels: Vec<(f64, f64)> = book.bids_iter().map(|(p, q)| (p.to_f64(), q.to_f64())).collect();
            let ask_levels: Vec<(f64, f64)> = book.asks_iter().map(|(p, q)| (p.to_f64(), q.to_f64())).collect();
            wall_tracker.update(&bid_levels, &ask_levels, ts_ns);

            let snapshot = DepthSnapshot::from_book(&book, ts_ns, args.top_n);

            // Build volume profile from recent trades (prune old ones first)
            let vp_cutoff_ms = (ts_ns as i64) / 1_000_000 - vp_window_ms;
            vp_trades.retain(|(ts_ms, _, _, _)| *ts_ms >= vp_cutoff_ms);
            let vp_slice: Vec<(f64, f64, Side)> = vp_trades.iter()
                .map(|(_, p, q, s)| (*p, *q, *s))
                .collect();
            let vp = VolumeProfile::from_trades(&vp_slice, args.vp_bin_width);

            // CVD momentum: need previous 2 CVD deltas
            let prev_cvd = if cvd_history.len() >= 1 { Some(cvd_history[cvd_history.len() - 1]) } else { None };
            let prev2_cvd = if cvd_history.len() >= 2 { Some(cvd_history[cvd_history.len() - 2]) } else { None };

            let features = DepthFeatures::compute(&snapshot, &cvd, &vp, &wall_tracker, prev_cvd, prev2_cvd, cum_trade_vol);
            
            // Track CVD delta for momentum calculation
            cvd_history.push(cvd.delta());
            if cvd_history.len() > 100 { cvd_history.drain(0..1); } // keep bounded

            snapshots.push(features);

            // Advance to next snapshot threshold
            if use_volume_bar {
                // Advance past current volume, avoiding duplicate snapshots
                // if cum_vol has crossed multiple thresholds
                while next_vol_bar_threshold <= cum_trade_vol {
                    next_vol_bar_threshold += volume_bar;
                }
            } else {
                next_snapshot_ns += interval_ns;
            }

            if snapshots.len() % 1000 == 0 {
                eprintln!("  {} snapshots, {} deltas applied", snapshots.len(), applied);
            }
        }
    }

    eprintln!("Processed {} deltas, {} snapshots collected", applied, snapshots.len());

    // Compute forward returns and write CSV
    // Forward return horizons in seconds: 15min, 1hr, 4hr
    let fwd_horizons_ns: [(u64, &str); 3] = [
        (15 * 60 * 1_000_000_000, "fwd_ret_15m_bps"),
        (60 * 60 * 1_000_000_000, "fwd_ret_1h_bps"),
        (4 * 60 * 60 * 1_000_000_000, "fwd_ret_4h_bps"),
    ];

    let mut out_file = File::create(&output_path)?;
    // Extended header with forward returns
    write!(out_file, "{},fwd_ret_15m_bps,fwd_ret_1h_bps,fwd_ret_4h_bps\n", DepthFeatures::csv_header())?;

    for i in 0..snapshots.len() {
        let snap = &snapshots[i];
        let ts = snap.ts;
        let mid_now = snap.mid_price;

        // Compute forward returns for each horizon
        let mut fwd_rets = [0.0_f64; 3];
        for (hi, (horizon_ns, _label)) in fwd_horizons_ns.iter().enumerate() {
            let target_ts = ts + horizon_ns;
            // Find the snapshot closest to target_ts (binary search would be better but linear is fine for now)
            let mut best_idx = None;
            let mut best_diff = u64::MAX;
            for j in (i + 1)..snapshots.len() {
                let diff = if snapshots[j].ts >= target_ts {
                    snapshots[j].ts - target_ts
                } else {
                    target_ts - snapshots[j].ts
                };
                if diff < best_diff {
                    best_diff = diff;
                    best_idx = Some(j);
                }
                if snapshots[j].ts >= target_ts {
                    break; // past the target, no need to search further
                }
            }
            if let Some(j) = best_idx {
                let mid_then = snapshots[j].mid_price;
                if mid_now > 0.0 {
                    fwd_rets[hi] = (mid_then - mid_now) / mid_now * 10000.0;
                }
            }
        }

        write!(out_file, "{},{:.4},{:.4},{:.4}\n", snap.to_csv_row(), fwd_rets[0], fwd_rets[1], fwd_rets[2])?;
    }

    eprintln!("Done: {} deltas applied, {} snapshots written to {}", applied, snapshots.len(), output_path.display());
    Ok(())
}