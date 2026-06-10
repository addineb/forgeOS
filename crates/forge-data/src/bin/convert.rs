//! `forge-convert` - CLI for the cryptohftdata parquet -> `*.forge` converter.
//!
//! Example:
//!   forge-convert --root /root/chd/data/ticks --symbol BTCUSDT --coin BTC \
//!       --date 2025-12-01 --hours 00,01 --feed-latency-ns 2000000 \
//!       --out /root/forgeOS/data/btc-20251201-00_01.forge

use std::path::PathBuf;
use std::process::ExitCode;

use forge_data::convert::{convert, ConvertConfig, Streams};
use forge_data::ForgeReader;

fn usage() -> String {
    "\
forge-convert - parquet -> *.forge converter

required:
  --root <dir>             root of the tick tree (e.g. /root/chd/data/ticks)
  --date <YYYY-MM-DD>      date partition
  --feed-latency-ns <u64>  feed latency added to exch_ts to form local_ts
  --out <file>             output *.forge path

optional:
  --symbol <S>             Binance symbol key (default BTCUSDT)
  --coin <C>               Hyperliquid coin key (default BTC)
  --hours <list|all>       comma list of HH or `all` (default all)
  --streams <list>         subset of trade,bookDelta,hlquote (default all)
  --verify                 read the output back and check count/monotonicity/checksum
"
    .to_string()
}

struct Cli {
    cfg: ConvertConfig,
    verify: bool,
}

fn parse() -> Result<Cli, String> {
    let mut root: Option<PathBuf> = None;
    let mut symbol = "BTCUSDT".to_string();
    let mut coin = "BTC".to_string();
    let mut date: Option<String> = None;
    let mut hours_arg = "all".to_string();
    let mut feed_latency_ns: Option<u64> = None;
    let mut out: Option<PathBuf> = None;
    let mut streams_arg: Option<String> = None;
    let mut verify = false;

    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut next = || args.next().ok_or_else(|| format!("missing value after {a}"));
        match a.as_str() {
            "--root" => root = Some(PathBuf::from(next()?)),
            "--symbol" => symbol = next()?,
            "--coin" => coin = next()?,
            "--date" => date = Some(next()?),
            "--hours" => hours_arg = next()?,
            "--feed-latency-ns" => {
                feed_latency_ns = Some(next()?.parse().map_err(|e| format!("bad --feed-latency-ns: {e}"))?);
            }
            "--out" => out = Some(PathBuf::from(next()?)),
            "--streams" => streams_arg = Some(next()?),
            "--verify" => verify = true,
            "-h" | "--help" => return Err(usage()),
            other => return Err(format!("unknown argument `{other}`\n\n{}", usage())),
        }
    }

    let root = root.ok_or("missing --root")?;
    let date = date.ok_or("missing --date")?;
    let feed_latency_ns = feed_latency_ns.ok_or("missing --feed-latency-ns")?;
    let out = out.ok_or("missing --out")?;

    let hours: Vec<String> = if hours_arg == "all" {
        (0..24).map(|h| format!("{h:02}")).collect()
    } else {
        hours_arg.split(',').map(|s| s.trim().to_string()).collect()
    };

    let streams = match streams_arg {
        None => Streams::default(),
        Some(list) => {
            let mut s = Streams { trade: false, book_delta: false, hlquote: false };
            for part in list.split(',') {
                match part.trim() {
                    "trade" => s.trade = true,
                    "bookDelta" => s.book_delta = true,
                    "hlquote" => s.hlquote = true,
                    other => return Err(format!("unknown stream `{other}`")),
                }
            }
            s
        }
    };

    Ok(Cli {
        cfg: ConvertConfig { root, symbol, coin, date, hours, feed_latency_ns, out, streams },
        verify,
    })
}

fn run() -> Result<(), String> {
    let Cli { cfg, verify } = parse()?;
    let meta = convert(&cfg).map_err(|e| format!("convert failed: {e}"))?;
    println!(
        "wrote {} : {} events, {} bytes, checksum {:#018x}",
        cfg.out.display(),
        meta.count,
        meta.bytes,
        meta.checksum
    );
    if verify {
        let reader = ForgeReader::open(&cfg.out).map_err(|e| format!("verify open: {e}"))?;
        if reader.len() as u64 != meta.count {
            return Err(format!(
                "verify count mismatch: file {} vs writer {}",
                reader.len(),
                meta.count
            ));
        }
        let mut last = 0u64;
        for rec in reader.records() {
            if rec.local_ts < last {
                return Err("verify failed: local_ts not monotonic".to_string());
            }
            last = rec.local_ts;
        }
        if reader.checksum() != meta.checksum {
            return Err("verify failed: checksum mismatch".to_string());
        }
        println!("verify OK: {} events, monotonic, checksum match", reader.len());
    }
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