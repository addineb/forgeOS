//! Multi-venue data feed: load the EXECUTION venue's full-depth book deltas
//! (Hyperliquid, via the converter `hlbook` stage) plus one or more REFERENCE
//! venues' trades (Binance etc.), stamp PER-VENUE feed latency to form
//! `local_ts = exch_ts + feed_latency`, and merge into one time-ordered stream.
//!
//! Timestamp contract (VERIFIED across venues, not assumed): the chd parquet
//! `ts`/`venueTs` columns are MILLISECONDS; we widen to nanoseconds here, the
//! engine's native unit. Fail-fast on negative/overflow.

use std::fs::File;
use std::path::{Path, PathBuf};

use arrow::array::{Array, BooleanArray, Float64Array, Int64Array, StringArray};
use arrow::record_batch::RecordBatch;
use forge_core::{Price, Qty, Side};
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

/// Which leg an event belongs to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    /// The venue we trade (Hyperliquid).
    Exec,
    /// A reference / fair-value venue (Binance, OKX, ...).
    Reference,
}

impl Role {
    #[inline]
    fn ord(self) -> u8 {
        match self {
            Role::Exec => 0,
            Role::Reference => 1,
        }
    }
}

/// Event payload kind.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LagKind {
    /// An incremental order-book level update (qty 0 = removal).
    BookDelta,
    /// A trade print.
    Trade,
    /// A funding-rate update (rate carried in aux).
    Funding,
}

/// One normalized multi-venue event. Timestamps are Unix NANOSECONDS.
#[derive(Clone, Copy, Debug)]
pub struct LagEvent {
    /// Which leg.
    pub role: Role,
    /// Payload kind.
    pub kind: LagKind,
    /// Venue timestamp (ns).
    pub exch_ts: u64,
    /// Earliest-visible timestamp = `exch_ts + feed_latency` (ns).
    pub local_ts: u64,
    /// Side where applicable.
    pub side: Option<Side>,
    /// Fixed-point price.
    pub price: Price,
    /// Fixed-point quantity (0 on a book-delta removal).
    pub qty: Qty,
    /// Reference-venue index (0-based) for Reference trades; 0 for exec events.
    pub src: u8,
    /// Auxiliary scalar (funding rate on Funding events; 0.0 otherwise).
    pub aux: f64,
}

/// A window-load request.
#[derive(Clone, Debug)]
pub struct FeedConfig {
    /// Root of the fresh tick tree, e.g. `/root/chd/fresh/ticks`.
    pub root: PathBuf,
    /// Execution coin key (hlbook dir), e.g. `BTC`.
    pub coin: String,
    /// Reference symbol key (trade dir), e.g. `BTCUSDT`.
    pub ref_symbols: Vec<String>,
    /// Date partition.
    pub date: String,
    /// Hours to load (`["00",...]`).
    pub hours: Vec<String>,
    /// Feed latency (ns) added to exec events.
    pub exec_latency_ns: u64,
    /// Feed latency (ns) added to reference events.
    pub ref_latency_ns: u64,
}

fn read_batches(path: &Path) -> Result<Vec<RecordBatch>, String> {
    let file = File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let rdr = ParquetRecordBatchReaderBuilder::try_new(file)
        .map_err(|e| format!("parquet {}: {e}", path.display()))?
        .build()
        .map_err(|e| format!("reader {}: {e}", path.display()))?;
    let mut out = Vec::new();
    for b in rdr {
        out.push(b.map_err(|e| format!("batch {}: {e}", path.display()))?);
    }
    Ok(out)
}

fn col_i64<'a>(b: &'a RecordBatch, n: &str) -> Result<&'a Int64Array, String> {
    b.column_by_name(n)
        .and_then(|c| c.as_any().downcast_ref::<Int64Array>())
        .ok_or_else(|| format!("missing i64 col `{n}`"))
}
fn col_f64<'a>(b: &'a RecordBatch, n: &str) -> Result<&'a Float64Array, String> {
    b.column_by_name(n)
        .and_then(|c| c.as_any().downcast_ref::<Float64Array>())
        .ok_or_else(|| format!("missing f64 col `{n}`"))
}
fn col_str<'a>(b: &'a RecordBatch, n: &str) -> Result<&'a StringArray, String> {
    b.column_by_name(n)
        .and_then(|c| c.as_any().downcast_ref::<StringArray>())
        .ok_or_else(|| format!("missing str col `{n}`"))
}
fn col_bool<'a>(b: &'a RecordBatch, n: &str) -> Result<&'a BooleanArray, String> {
    b.column_by_name(n)
        .and_then(|c| c.as_any().downcast_ref::<BooleanArray>())
        .ok_or_else(|| format!("missing bool col `{n}`"))
}

/// ms -> ns, fail-fast on negative / overflow.
fn ms_to_ns(ms: i64) -> Result<u64, String> {
    if ms < 0 {
        return Err(format!("negative ts {ms}"));
    }
    i64::checked_mul(ms, 1_000_000)
        .map(|ns| ns as u64)
        .ok_or_else(|| "ts overflow ms->ns".to_string())
}

fn push_hlbook(path: &Path, lat: u64, out: &mut Vec<LagEvent>) -> Result<(), String> {
    for b in read_batches(path)? {
        let vts = col_i64(&b, "venueTs")?;
        let side = col_str(&b, "side")?;
        let price = col_f64(&b, "price")?;
        let qty = col_f64(&b, "qty")?;
        for i in 0..b.num_rows() {
            let exch = ms_to_ns(vts.value(i))?;
            let s = match side.value(i) {
                "bid" | "buy" | "b" => Side::Bid,
                "ask" | "sell" | "a" => Side::Ask,
                other => return Err(format!("bad side `{other}`")),
            };
            out.push(LagEvent {
                role: Role::Exec,
                kind: LagKind::BookDelta,
                exch_ts: exch,
                local_ts: exch.saturating_add(lat),
                side: Some(s),
                price: Price::from_f64(price.value(i)).map_err(|e| format!("{e}"))?,
                qty: Qty::from_f64(qty.value(i)).map_err(|e| format!("{e}"))?,
                src: 0,
                aux: 0.0,
            });
        }
    }
    Ok(())
}

fn push_trades(path: &Path, lat: u64, role: Role, src: u8, out: &mut Vec<LagEvent>) -> Result<(), String> {
    for b in read_batches(path)? {
        let ts = col_i64(&b, "ts")?;
        let price = col_f64(&b, "price")?;
        let qty = col_f64(&b, "qty")?;
        let ibm = col_bool(&b, "isBuyerMaker")?;
        for i in 0..b.num_rows() {
            let exch = ms_to_ns(ts.value(i))?;
            // buyer-maker => taker is the seller => aggressor Ask.
            let s = if ibm.value(i) { Side::Ask } else { Side::Bid };
            out.push(LagEvent {
                role,
                kind: LagKind::Trade,
                exch_ts: exch,
                local_ts: exch.saturating_add(lat),
                side: Some(s),
                price: Price::from_f64(price.value(i)).map_err(|e| format!("{e}"))?,
                qty: Qty::from_f64(qty.value(i)).map_err(|e| format!("{e}"))?,
                src,
                aux: 0.0,
            });
        }
    }
    Ok(())
}

fn push_funding(path: &Path, lat: u64, out: &mut Vec<LagEvent>) -> Result<(), String> {
    for b in read_batches(path)? {
        let ts = col_i64(&b, "ts")?;
        let fr = col_f64(&b, "funding")?;
        for i in 0..b.num_rows() {
            let exch = ms_to_ns(ts.value(i))?;
            out.push(LagEvent {
                role: Role::Reference,
                kind: LagKind::Funding,
                exch_ts: exch,
                local_ts: exch.saturating_add(lat),
                side: None,
                price: Price::from_raw(0),
                qty: Qty::from_raw(0),
                src: 0,
                aux: fr.value(i),
            });
        }
    }
    Ok(())
}

/// Load + merge one day window into a deterministic, time-ordered stream.
///
/// # Errors
/// Any I/O, parquet decode, or fixed-point conversion failure.
pub fn load_window(cfg: &FeedConfig) -> Result<Vec<LagEvent>, String> {
    let mut evs: Vec<LagEvent> = Vec::new();
    for hh in &cfg.hours {
        let ex = cfg
            .root
            .join(&cfg.coin)
            .join("hlbook")
            .join(&cfg.date)
            .join(format!("{hh}.parquet"));
        if ex.exists() {
            push_hlbook(&ex, cfg.exec_latency_ns, &mut evs)?;
        }
        for (i, sym) in cfg.ref_symbols.iter().enumerate() {
            let rf = cfg.root.join(sym).join("trade").join(&cfg.date).join(format!("{hh}.parquet"));
            if rf.exists() {
                push_trades(&rf, cfg.ref_latency_ns, Role::Reference, i as u8, &mut evs)?;
            }
        }
        // EXEC venue (HL) trades -> needed to fill resting maker orders (queue model).
        let ex_tr = cfg.root.join(&cfg.coin).join("trade").join(&cfg.date).join(format!("{hh}.parquet"));
        if ex_tr.exists() {
            push_trades(&ex_tr, cfg.exec_latency_ns, Role::Exec, 0, &mut evs)?;
        }
        // EXEC venue (HL) funding rate -> crowded-positioning conditioning.
        let fnd = cfg.root.join(&cfg.coin).join("funding").join(&cfg.date).join(format!("{hh}.parquet"));
        if fnd.exists() {
            push_funding(&fnd, cfg.exec_latency_ns, &mut evs)?;
        }
    }
    evs.sort_by(|a, b| {
        a.local_ts
            .cmp(&b.local_ts)
            .then(a.role.ord().cmp(&b.role.ord()))
            .then((a.kind as u8).cmp(&(b.kind as u8)))
            .then(a.src.cmp(&b.src))
            .then(a.price.raw().cmp(&b.price.raw()))
    });
    Ok(evs)
}