//! `lag-feedcheck` - load one multi-venue window and print sanity stats so the
//! data feed is VERIFIED before any sim is built on it.
//!
//!   lag-feedcheck --root /root/chd/fresh/ticks --coin BTC --symbol BTCUSDT \
//!     --date 2026-06-09 --hours 08,09,10,11,12,13,14,15

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use forgelag::{load_window, FeedConfig, LagKind, Role};

fn run() -> Result<(), String> {
    let mut root = PathBuf::from("/root/chd/fresh/ticks");
    let mut coin = "BTC".to_string();
    let mut symbol = "BTCUSDT".to_string();
    let mut date = String::new();
    let mut hours_arg = "all".to_string();
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = || args.next().ok_or_else(|| format!("missing value after {a}"));
        match a.as_str() {
            "--root" => root = PathBuf::from(val()?),
            "--coin" => coin = val()?,
            "--symbol" => symbol = val()?,
            "--date" => date = val()?,
            "--hours" => hours_arg = val()?,
            other => return Err(format!("unknown arg {other}")),
        }
    }
    if date.is_empty() {
        return Err("missing --date".into());
    }
    let hours: Vec<String> = if hours_arg == "all" {
        (0..24).map(|h| format!("{h:02}")).collect()
    } else {
        hours_arg.split(',').map(|s| s.trim().to_string()).collect()
    };

    let cfg = FeedConfig {
        root,
        coin,
        ref_symbols: vec![symbol],
        date: date.clone(),
        hours,
        exec_latency_ns: 0,
        ref_latency_ns: 0,
    };
    let evs = load_window(&cfg)?;
    if evs.is_empty() {
        return Err("no events loaded (check paths)".into());
    }

    let mut n_exec = 0u64;
    let mut n_ref = 0u64;
    let mut bids: BTreeMap<i64, i64> = BTreeMap::new();
    let mut asks: BTreeMap<i64, i64> = BTreeMap::new();
    let mut basis_samples: Vec<f64> = Vec::new();

    for e in &evs {
        match (e.role, e.kind) {
            (Role::Exec, LagKind::BookDelta) => {
                n_exec += 1;
                let p = e.price.raw();
                let q = e.qty.raw();
                let book = if matches!(e.side, Some(forge_core::Side::Bid)) { &mut bids } else { &mut asks };
                if q == 0 {
                    book.remove(&p);
                } else {
                    book.insert(p, q);
                }
            }
            (Role::Reference, LagKind::Trade) => {
                n_ref += 1;
                let lr = e.price.to_f64();
                if let (Some((&bb, _)), Some((&ba, _))) = (bids.iter().next_back(), asks.iter().next()) {
                    if ba > bb && lr > 0.0 {
                        let mid = (bb + ba) as f64 / 2.0 / forge_core::SCALE_F64;
                        basis_samples.push((mid - lr) / lr * 1e4);
                    }
                }
            }
            _ => {}
        }
    }

    let span_ns = evs.last().unwrap().local_ts - evs.first().unwrap().local_ts;
    let span_h = span_ns as f64 / 3_600_000_000_000.0;
    println!("date            {date}");
    println!("events          {} total  (exec book-deltas {n_exec}, ref trades {n_ref})", evs.len());
    println!("span            {span_h:.2} h");
    println!("ref trades/s    {:.2}", n_ref as f64 / (span_ns as f64 / 1e9).max(1.0));
    if basis_samples.is_empty() {
        println!("basis           NONE (book never two-sided when a trade printed)");
    } else {
        let n = basis_samples.len();
        let mut s = basis_samples.clone();
        s.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let med = s[n / 2];
        let p10 = s[n / 10];
        let p90 = s[n * 9 / 10];
        let mean = basis_samples.iter().sum::<f64>() / n as f64;
        println!("basis(HL-ref)   n={n} mean={mean:.2}bps  p10={p10:.2} med={med:.2} p90={p90:.2}");
    }
    println!("FEED OK");
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