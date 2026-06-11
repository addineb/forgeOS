//! The basis/lag sim: replay a merged multi-venue [`LagEvent`] stream on a
//! virtual clock, maintain the EXECUTION book (HL) and the REFERENCE price
//! (Binance), apply an outbound ORDER LATENCY (orders submitted at T reach the
//! matcher at `T + order_latency` and are priced against the exec book AS IT
//! STANDS THEN = adverse selection), take liquidity on the exec book (real
//! spread + slippage), and record per-round-trip P&L. Reuses the proven
//! `forge-sim` fill/account math and the `forge-book` order book.

use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;

use forge_book::OrderBook;
use forge_core::{Event, EventKind, Qty, Side, UnixNanos};
use forge_sim::{money_to_f64, price_market, Account, FeeSchedule};

use crate::feed::{LagEvent, LagKind, Role};

/// Read-only context handed to a strategy on each event. Exposes only data at
/// or before `now` (no-lookahead by construction).
pub struct LagCtx<'a> {
    /// Virtual clock (ns) = `event.local_ts`.
    pub now: u64,
    /// The execution venue book (HL) folded up to `now`.
    pub exec_book: &'a OrderBook,
    /// Latest reference price (Binance trade), 0.0 until the first ref trade.
    pub ref_px: f64,
    /// Our signed position (raw qty): + long, - short.
    pub position_qty: i64,
}

/// A market (taker) order on the execution venue. `side` is OUR side
/// (`Bid` = buy, `Ask` = sell).
#[derive(Clone, Copy, Debug)]
pub struct LagOrder {
    /// Buy (bid) or sell (ask).
    pub side: Side,
    /// Quantity.
    pub qty: Qty,
}

/// A basis/lag strategy: pure reaction to `(exec_book, ref_px, position, now)`.
pub trait LagStrategy {
    /// Called once per event after it is applied; push taker orders into `out`.
    fn on_event(&mut self, ctx: &LagCtx, out: &mut Vec<LagOrder>);
}

/// Engine configuration.
#[derive(Clone, Copy, Debug)]
pub struct LagConfig {
    /// Outbound order latency (ns): submit at T, reach matcher at `T + this`.
    pub order_latency_ns: u64,
    /// Max exec-book levels kept per side (0 = unlimited).
    pub exec_book_levels: usize,
    /// Fee schedule applied to fills.
    pub fees: FeeSchedule,
}

impl Default for LagConfig {
    fn default() -> Self {
        Self { order_latency_ns: 0, exec_book_levels: 20, fees: FeeSchedule::legacy() }
    }
}

#[derive(Clone, Copy)]
struct Pending {
    arrival: u64,
    seq: u64,
    side_u8: u8, // 1 = Bid, 2 = Ask
    qty_raw: i64,
}
impl PartialEq for Pending {
    fn eq(&self, o: &Self) -> bool {
        self.arrival == o.arrival && self.seq == o.seq
    }
}
impl Eq for Pending {}
impl Ord for Pending {
    fn cmp(&self, o: &Self) -> Ordering {
        self.arrival.cmp(&o.arrival).then(self.seq.cmp(&o.seq))
    }
}
impl PartialOrd for Pending {
    fn partial_cmp(&self, o: &Self) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

/// Summary of a completed lag replay.
#[derive(Clone, Debug)]
pub struct LagReport {
    /// Events processed.
    pub events: u64,
    /// Orders the strategy submitted.
    pub orders_submitted: u64,
    /// Orders filled (taker).
    pub orders_filled: u64,
    /// Orders that found no liquidity.
    pub orders_rejected: u64,
    /// Completed round trips.
    pub round_trips: u64,
    /// Net P&L (quote units, f64).
    pub net_pnl: f64,
    /// Final signed position (raw qty).
    pub final_position: i64,
    /// Per-round-trip return in PERCENT of entry notional (the honest sample).
    pub trip_returns_pct: Vec<f64>,
}

/// The lag replay engine. One engine = one deterministic single-threaded sim.
pub struct LagEngine<S: LagStrategy> {
    book: OrderBook,
    ref_px: f64,
    acct: Account,
    strat: S,
    fees: FeeSchedule,
    order_latency_ns: u64,
    now: u64,
    started: bool,
    pending: BinaryHeap<Reverse<Pending>>,
    seq: u64,
    buf: Vec<LagOrder>,
    orders_submitted: u64,
    orders_filled: u64,
    orders_rejected: u64,
    events: u64,
}

impl<S: LagStrategy> LagEngine<S> {
    /// Build an engine with the given strategy and config.
    pub fn new(strat: S, cfg: LagConfig) -> Self {
        Self {
            book: OrderBook::with_max_levels(cfg.exec_book_levels),
            ref_px: 0.0,
            acct: Account::new(),
            strat,
            fees: cfg.fees,
            order_latency_ns: cfg.order_latency_ns,
            now: 0,
            started: false,
            pending: BinaryHeap::new(),
            seq: 0,
            buf: Vec::new(),
            orders_submitted: 0,
            orders_filled: 0,
            orders_rejected: 0,
            events: 0,
        }
    }

    fn execute(&mut self, p: Pending) {
        let side = if p.side_u8 == 1 { Side::Bid } else { Side::Ask };
        let qty = Qty::from_raw(p.qty_raw);
        match price_market(&self.book, side, qty) {
            Some(fill) => {
                self.acct.apply_taker(side, &fill, &self.fees, self.now);
                self.orders_filled += 1;
            }
            None => self.orders_rejected += 1,
        }
    }

    fn drain_pending(&mut self, now: u64) {
        while let Some(top) = self.pending.peek() {
            if top.0.arrival <= now {
                let Reverse(p) = self.pending.pop().expect("peek implies pop");
                self.execute(p);
            } else {
                break;
            }
        }
    }

    /// Process one event.
    ///
    /// # Errors
    /// A non-monotonic `local_ts`, an order-arrival overflow, or a book error.
    pub fn step(&mut self, ev: &LagEvent) -> Result<(), String> {
        let t = ev.local_ts;
        if self.started && t < self.now {
            return Err(format!("non-monotonic local_ts: prev {} got {t}", self.now));
        }
        self.now = t;
        self.started = true;

        self.drain_pending(t);

        match (ev.role, ev.kind) {
            (Role::Exec, LagKind::BookDelta) => {
                let e = Event::new(
                    EventKind::BookDelta,
                    UnixNanos::new(ev.exch_ts),
                    UnixNanos::new(ev.local_ts),
                    ev.side,
                    ev.price,
                    ev.qty,
                    0,
                )
                .map_err(|e| format!("{e}"))?;
                self.book.apply(&e).map_err(|e| format!("{e}"))?;
            }
            (Role::Reference, LagKind::Trade) => {
                self.ref_px = ev.price.to_f64();
            }
            _ => {}
        }

        self.buf.clear();
        {
            let ctx = LagCtx {
                now: t,
                exec_book: &self.book,
                ref_px: self.ref_px,
                position_qty: self.acct.net_qty(),
            };
            self.strat.on_event(&ctx, &mut self.buf);
        }

        for idx in 0..self.buf.len() {
            let o = self.buf[idx];
            let arrival = t
                .checked_add(self.order_latency_ns)
                .ok_or("order arrival overflow")?;
            let side_u8 = if o.side == Side::Bid { 1 } else { 2 };
            self.pending.push(Reverse(Pending { arrival, seq: self.seq, side_u8, qty_raw: o.qty.raw() }));
            self.seq += 1;
            self.orders_submitted += 1;
        }

        self.events += 1;
        Ok(())
    }

    /// Run over a sequence of events.
    ///
    /// # Errors
    /// Propagates any [`LagEngine::step`] error.
    pub fn run<'a, I: IntoIterator<Item = &'a LagEvent>>(&mut self, evs: I) -> Result<(), String> {
        for ev in evs {
            self.step(ev)?;
        }
        Ok(())
    }

    /// Drain in-flight orders and produce the report.
    #[must_use]
    pub fn finish(mut self) -> LagReport {
        self.drain_pending(u64::MAX);
        let tp = self.acct.trip_pnls();
        let tn = self.acct.trip_notionals();
        let mut trip_returns_pct = Vec::with_capacity(tp.len());
        for (pnl, notl) in tp.iter().zip(tn.iter()) {
            if *notl > 0 {
                trip_returns_pct.push(money_to_f64(*pnl) / money_to_f64(*notl) * 100.0);
            }
        }
        LagReport {
            events: self.events,
            orders_submitted: self.orders_submitted,
            orders_filled: self.orders_filled,
            orders_rejected: self.orders_rejected,
            round_trips: self.acct.round_trips(),
            net_pnl: money_to_f64(self.acct.net_pnl()),
            final_position: self.acct.net_qty(),
            trip_returns_pct,
        }
    }
}