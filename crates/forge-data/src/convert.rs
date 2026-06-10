//! cryptohftdata parquet -> `*.forge` converter (feature `convert`).
//!
//! Reads the three normalized parquet streams produced by
//! `tools/chd-to-parquet.py` (trade / bookDelta / hlquote), merges them into a
//! single event stream, assigns `local_ts = exch_ts + feed_latency` with an
//! explicit configurable latency, sorts by a total deterministic key, and
//! writes a `*.forge` file via [`ForgeWriter`]. Fail-fast on NaN/Inf, negative
//! venue timestamps, or overflow (Requirement 7 + 9).

use std::fs::File;
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use arrow::array::{Array, BooleanArray, Float64Array, Int64Array, StringArray};
use arrow::record_batch::RecordBatch;
use forge_core::{side_to_u8, Event, EventKind, ForgeError, Price, Qty, Side, UnixNanos};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

use crate::writer::{DataError, StreamMeta};
use crate::ForgeWriter;

/// Which streams to include in a conversion.
#[derive(Clone, Copy, Debug)]
pub struct Streams {
    /// Binance trade prints.
    pub trade: bool,
    /// Binance incremental order-book deltas.
    pub book_delta: bool,
    /// Hyperliquid top-of-book quotes (emitted as paired bid/ask events).
    pub hlquote: bool,
}

impl Default for Streams {
    fn default() -> Self {
        Self { trade: true, book_delta: true, hlquote: true }
    }
}

/// Configuration for a single conversion run.
#[derive(Clone, Debug)]
pub struct ConvertConfig {
    /// Root of the normalized tick tree (e.g. `/root/chd/data/ticks`).
    pub root: PathBuf,
    /// Binance symbol key (trade + bookDelta), e.g. `BTCUSDT`.
    pub symbol: String,
    /// Hyperliquid coin key (hlquote), e.g. `BTC`.
    pub coin: String,
    /// Date partition, e.g. `2025-12-01`.
    pub date: String,
    /// Two-digit hour partitions to include, e.g. `["00","01"]`.
    pub hours: Vec<String>,
    /// Feed latency in nanoseconds added to each `exch_ts` to form `local_ts`.
    pub feed_latency_ns: u64,
    /// Output `*.forge` path.
    pub out: PathBuf,
    /// Which streams to merge.
    pub streams: Streams,
}

fn parse_err(msg: impl Into<String>) -> DataError {
    DataError::Parse(msg.into())
}

/// Read every record batch from a parquet file.
fn read_batches(path: &Path) -> Result<Vec<RecordBatch>, DataError> {
    let file = File::open(path)?;
    let builder = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|e| parse_err(format!("open parquet {}: {e}", path.display())))?;
    let reader = builder
        .build()
        .map_err(|e| parse_err(format!("build reader {}: {e}", path.display())))?;
    let mut out = Vec::new();
    for batch in reader {
        out.push(batch.map_err(|e| parse_err(format!("read batch {}: {e}", path.display())))?);
    }
    Ok(out)
}

fn col_i64<'a>(b: &'a RecordBatch, name: &str) -> Result<&'a Int64Array, DataError> {
    b.column_by_name(name)
        .and_then(|c| c.as_any().downcast_ref::<Int64Array>())
        .ok_or_else(|| parse_err(format!("missing or non-i64 column `{name}`")))
}

fn col_f64<'a>(b: &'a RecordBatch, name: &str) -> Result<&'a Float64Array, DataError> {
    b.column_by_name(name)
        .and_then(|c| c.as_any().downcast_ref::<Float64Array>())
        .ok_or_else(|| parse_err(format!("missing or non-f64 column `{name}`")))
}

fn col_bool<'a>(b: &'a RecordBatch, name: &str) -> Result<&'a BooleanArray, DataError> {
    b.column_by_name(name)
        .and_then(|c| c.as_any().downcast_ref::<BooleanArray>())
        .ok_or_else(|| parse_err(format!("missing or non-bool column `{name}`")))
}

fn col_str<'a>(b: &'a RecordBatch, name: &str) -> Result<&'a StringArray, DataError> {
    b.column_by_name(name)
        .and_then(|c| c.as_any().downcast_ref::<StringArray>())
        .ok_or_else(|| parse_err(format!("missing or non-utf8 column `{name}`")))
}

fn check_non_null(arr: &dyn Array, i: usize, name: &str) -> Result<(), DataError> {
    if arr.is_null(i) {
        return Err(parse_err(format!("null in column `{name}` at row {i}")));
    }
    Ok(())
}

/// The chd normalized feed stamps timestamps in MILLISECONDS; forge-core is
/// nanosecond-native, so convert at ingest. Fail-fast on negative or overflow.
fn exch_from_ms(ms: i64) -> Result<UnixNanos, DataError> {
    if ms < 0 {
        return Err(DataError::Forge(ForgeError::Negative { field: "exch_ts_ms", value: ms }));
    }
    let ns = ms
        .checked_mul(1_000_000)
        .ok_or(DataError::Forge(ForgeError::Overflow { op: "ms_to_ns" }))?;
    UnixNanos::from_i64(ns).map_err(DataError::Forge)
}

fn local_from(exch: UnixNanos, feed_latency_ns: u64) -> Result<UnixNanos, DataError> {
    exch.checked_add(feed_latency_ns).map_err(DataError::Forge)
}

fn push_trades(path: &Path, lat: u64, out: &mut Vec<Event>) -> Result<(), DataError> {
    for b in read_batches(path)? {
        let ts = col_i64(&b, "ts")?;
        let price = col_f64(&b, "price")?;
        let qty = col_f64(&b, "qty")?;
        let ibm = col_bool(&b, "isBuyerMaker")?;
        for i in 0..b.num_rows() {
            check_non_null(ts, i, "ts")?;
            check_non_null(price, i, "price")?;
            check_non_null(qty, i, "qty")?;
            check_non_null(ibm, i, "isBuyerMaker")?;
            let exch = exch_from_ms(ts.value(i))?;
            let local = local_from(exch, lat)?;
            // is_buyer_maker => the taker (aggressor) is the seller => Ask.
            let side = if ibm.value(i) { Side::Ask } else { Side::Bid };
            let ev = Event::new(
                EventKind::Trade,
                exch,
                local,
                Some(side),
                Price::from_f64(price.value(i)).map_err(DataError::Forge)?,
                Qty::from_f64(qty.value(i)).map_err(DataError::Forge)?,
                0,
            )
            .map_err(DataError::Forge)?;
            out.push(ev);
        }
    }
    Ok(())
}

fn push_book_delta(path: &Path, lat: u64, out: &mut Vec<Event>) -> Result<(), DataError> {
    for b in read_batches(path)? {
        let vts = col_i64(&b, "venueTs")?;
        let side = col_str(&b, "side")?;
        let price = col_f64(&b, "price")?;
        let qty = col_f64(&b, "qty")?;
        for i in 0..b.num_rows() {
            check_non_null(vts, i, "venueTs")?;
            check_non_null(side, i, "side")?;
            check_non_null(price, i, "price")?;
            check_non_null(qty, i, "qty")?;
            let exch = exch_from_ms(vts.value(i))?;
            let local = local_from(exch, lat)?;
            let s = match side.value(i) {
                "bid" | "buy" | "b" => Side::Bid,
                "ask" | "sell" | "a" => Side::Ask,
                other => return Err(parse_err(format!("unknown book side `{other}`"))),
            };
            // qty == 0 encodes a level removal; Qty accepts zero.
            let ev = Event::new(
                EventKind::BookDelta,
                exch,
                local,
                Some(s),
                Price::from_f64(price.value(i)).map_err(DataError::Forge)?,
                Qty::from_f64(qty.value(i)).map_err(DataError::Forge)?,
                0,
            )
            .map_err(DataError::Forge)?;
            out.push(ev);
        }
    }
    Ok(())
}

fn push_hlquote(path: &Path, lat: u64, out: &mut Vec<Event>) -> Result<(), DataError> {
    for b in read_batches(path)? {
        let ts = col_i64(&b, "ts")?;
        let bid = col_f64(&b, "bid")?;
        let ask = col_f64(&b, "ask")?;
        for i in 0..b.num_rows() {
            check_non_null(ts, i, "ts")?;
            check_non_null(bid, i, "bid")?;
            check_non_null(ask, i, "ask")?;
            let exch = exch_from_ms(ts.value(i))?;
            let local = local_from(exch, lat)?;
            // A BBO quote is emitted as a paired bid/ask Quote event (qty 0,
            // since BBO-only carries no size). mid is derivable downstream.
            out.push(
                Event::new(
                    EventKind::Quote,
                    exch,
                    local,
                    Some(Side::Bid),
                    Price::from_f64(bid.value(i)).map_err(DataError::Forge)?,
                    Qty::ZERO,
                    0,
                )
                .map_err(DataError::Forge)?,
            );
            out.push(
                Event::new(
                    EventKind::Quote,
                    exch,
                    local,
                    Some(Side::Ask),
                    Price::from_f64(ask.value(i)).map_err(DataError::Forge)?,
                    Qty::ZERO,
                    0,
                )
                .map_err(DataError::Forge)?,
            );
        }
    }
    Ok(())
}

fn stream_dir(cfg: &ConvertConfig, key: &str, stream: &str) -> PathBuf {
    cfg.root.join(key).join(stream).join(&cfg.date)
}

/// Run a conversion, returning the finalized stream metadata.
///
/// Missing hour files are skipped (partial windows are normal). The event set
/// is sorted by a total key `(local_ts, exch_ts, kind, side, price)` so output
/// is byte-identical for identical input and latency (Requirement 7.6).
///
/// # Errors
/// Any I/O, parquet decode, or [`forge_core::ForgeError`] integrity failure.
pub fn convert(cfg: &ConvertConfig) -> Result<StreamMeta, DataError> {
    let lat = cfg.feed_latency_ns;
    let mut events: Vec<Event> = Vec::new();

    for hh in &cfg.hours {
        if cfg.streams.trade {
            let p = stream_dir(cfg, &cfg.symbol, "trade").join(format!("{hh}.parquet"));
            if p.exists() {
                push_trades(&p, lat, &mut events)?;
            }
        }
        if cfg.streams.book_delta {
            let p = stream_dir(cfg, &cfg.symbol, "bookDelta").join(format!("{hh}.parquet"));
            if p.exists() {
                push_book_delta(&p, lat, &mut events)?;
            }
        }
        if cfg.streams.hlquote {
            let p = stream_dir(cfg, &cfg.coin, "hlquote").join(format!("{hh}.parquet"));
            if p.exists() {
                push_hlquote(&p, lat, &mut events)?;
            }
        }
    }

    events.sort_unstable_by(|a, b| {
        a.local_ts
            .cmp(&b.local_ts)
            .then(a.exch_ts.cmp(&b.exch_ts))
            .then(a.kind.as_u8().cmp(&b.kind.as_u8()))
            .then(side_to_u8(a.side).cmp(&side_to_u8(b.side)))
            .then(a.price.raw().cmp(&b.price.raw()))
    });

    if let Some(parent) = cfg.out.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = File::create(&cfg.out)?;
    let mut writer = ForgeWriter::new(BufWriter::new(file));
    for ev in &events {
        writer.write_event(ev)?;
    }
    writer.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ms_to_ns_converts() {
        // 2025-12-01 00:00:00 UTC in ms -> ns.
        let e = exch_from_ms(1_764_547_200_000).unwrap();
        assert_eq!(e.get(), 1_764_547_200_000_000_000);
    }

    #[test]
    fn ms_to_ns_rejects_negative() {
        assert!(exch_from_ms(-1).is_err());
    }

    #[test]
    fn ms_to_ns_rejects_overflow() {
        // i64::MAX ms * 1e6 overflows i64 -> fail fast, not wrap.
        assert!(exch_from_ms(i64::MAX).is_err());
    }
}