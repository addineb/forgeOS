//! The basis/lag sim: replay a merged multi-venue [`LagEvent`] stream on a
//! virtual clock, maintain the EXECUTION book (HL) and the REFERENCE price
//! (Binance), apply an outbound ORDER LATENCY, and execute either TAKER market
//! orders (cross the spread, walk the book = slippage) or MAKER limit orders
//! (rest in the book, fill only when HL's own tape trades through them after
//! clearing the queue ahead = adverse selection). Records per-round-trip P&L.
//! Reuses the proven `forge-sim` fill/account math and `forge-book` order book.

use std::cmp::{Ordering, Reverse};
use std::collections::BinaryHeap;

use forge_book::OrderBook;
use forge_core::{Event, EventKind, Price, Qty, Side, UnixNanos};
use forge_sim::{maker_fill, money_to_f64, price_market, price_to_limit, Account, FeeSchedule};

use crate::feed::{LagEvent, LagKind, Role};

/// Read-only context handed to a strategy on each event (no-lookahead).
pub struct LagCtx<'a> {
    /// Virtual clock (ns) = `event.local_ts`.
    pub now: u64,
    /// The execution venue book (HL) folded up to `now`.
    pub exec_book: &'a OrderBook,
    /// Latest reference price (Binance trade), 0.0 until the first ref trade.
    pub ref_px: f64,
    /// Latest HL funding rate (per-hour rate), 0.0 until the first funding tick.
    pub funding: f64,
    /// Latest cross-asset LEAD price (e.g. BTC), 0.0 until the first lead trade.
    pub lead_px: f64,
    /// Our signed position (raw qty): + long, - short.
    pub position_qty: i64,
}

/// How an order executes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LagOrderKind {
    /// Cross the spread now (taker).
    Market,
    /// Rest at `price`; fill only when the HL tape trades through it (maker).
    Limit,
}

/// An order on the execution venue. `side` is OUR side (`Bid` = buy).
#[derive(Clone, Copy, Debug)]
pub struct LagOrder {
    /// Buy (bid) or sell (ask).
    pub side: Side,
    /// Execution style.
    pub kind: LagOrderKind,
    /// Limit price (unused for `Market`).
    pub price: Price,
    /// Quantity.
    pub qty: Qty,
}
impl LagOrder {
    /// A market (taker) order.
    #[must_use]
    pub fn market(side: Side, qty: Qty) -> Self {
        Self { side, kind: LagOrderKind::Market, price: Price::ZERO, qty }
    }
    /// A limit (maker) order resting at `price`.
    #[must_use]
    pub fn limit(side: Side, price: Price, qty: Qty) -> Self {
        Self { side, kind: LagOrderKind::Limit, price, qty }
    }
}

/// A basis/lag strategy: pure reaction to `(exec_book, ref_px, position, now)`.
pub trait LagStrategy {
    /// Called once per event after it is applied; push orders into `out`.
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
    order: LagOrder,
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

#[derive(Clone, Copy)]
struct Resting {
    side: Side,
    price: i64,
    qty_remaining: i64,
    queue_ahead: i64,
}

/// Summary of a completed lag replay.
#[derive(Clone, Debug)]
pub struct LagReport {
    /// Events processed.
    pub events: u64,
    /// Orders the strategy submitted.
    pub orders_submitted: u64,
    /// Orders filled (taker + maker).
    pub orders_filled: u64,
    /// Of the fills, how many were maker (queue) fills.
    pub maker_fills: u64,
    /// Limit orders that rested passively.
    pub orders_rested: u64,
    /// Orders that found no liquidity.
    pub orders_rejected: u64,
    /// Completed round trips.
    pub round_trips: u64,
    /// Net P&L (quote units, f64).
    pub net_pnl: f64,
    /// Final signed position (raw qty).
    pub final_position: i64,
    /// Per-round-trip (close_ts_ns, return_pct of entry notional).
    pub trip_returns: Vec<(u64, f64)>,
    /// Entry notional per round trip (quote units), paired with trip_returns.
    pub trip_notionals: Vec<f64>,
}

/// The lag replay engine. One engine = one deterministic single-threaded sim.
pub struct LagEngine<S: LagStrategy> {
    book: OrderBook,
    ref_px: f64,
    ref_last: Vec<f64>,
    funding: f64,
    lead_px: f64,
    acct: Account,
    strat: S,
    fees: FeeSchedule,
    order_latency_ns: u64,
    lat_samples: Vec<u64>,
    lat_rng: u64,
    now: u64,
    started: bool,
    pending: BinaryHeap<Reverse<Pending>>,
    resting: Vec<Resting>,
    seq: u64,
    buf: Vec<LagOrder>,
    orders_submitted: u64,
    orders_filled: u64,
    maker_fills: u64,
    orders_rested: u64,
    orders_rejected: u64,
    events: u64,
}

impl<S: LagStrategy> LagEngine<S> {
    /// Build an engine with the given strategy and config.
    pub fn new(strat: S, cfg: LagConfig) -> Self {
        Self {
            book: OrderBook::with_max_levels(cfg.exec_book_levels),
            ref_px: 0.0,
            ref_last: Vec::new(),
            funding: 0.0,
            lead_px: 0.0,
            acct: Account::new(),
            strat,
            fees: cfg.fees,
            order_latency_ns: cfg.order_latency_ns,
            lat_samples: Vec::new(),
            lat_rng: 0x9E37_79B9_7F4A_7C15,
            now: 0,
            started: false,
            pending: BinaryHeap::new(),
            resting: Vec::new(),
            seq: 0,
            buf: Vec::new(),
            orders_submitted: 0,
            orders_filled: 0,
            maker_fills: 0,
            orders_rested: 0,
            orders_rejected: 0,
            events: 0,
        }
    }

    /// Supply an empirical per-order latency distribution (ns). When non-empty, each
    /// order samples its submit->execute delay from these instead of the fixed latency.
    pub fn set_latency_samples(&mut self, s: Vec<u64>) {
        self.lat_samples = s;
    }

    fn next_latency(&mut self) -> u64 {
        if self.lat_samples.is_empty() {
            return self.order_latency_ns;
        }
        let mut x = self.lat_rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.lat_rng = x;
        let idx = (x % self.lat_samples.len() as u64) as usize;
        self.lat_samples[idx]
    }

    fn fill_taker(&mut self, side: Side, fill: &forge_sim::Fill) {
        self.acct.apply_taker(side, fill, &self.fees, self.now);
        self.orders_filled += 1;
    }

    fn place_limit(&mut self, order: LagOrder) {
        let limit = order.price.raw();
        let best_ask = self.book.best_ask().map(|(p, _)| p.raw());
        let best_bid = self.book.best_bid().map(|(p, _)| p.raw());
        let marketable = match order.side {
            Side::Bid => best_ask.is_some_and(|a| limit >= a),
            Side::Ask => best_bid.is_some_and(|b| limit <= b),
        };
        if marketable {
            match price_to_limit(&self.book, order.side, order.qty, Some(limit)) {
                Some(fill) => self.fill_taker(order.side, &fill),
                None => self.orders_rejected += 1,
            }
            return;
        }
        let queue_ahead = self.book.qty_at(order.side, order.price).map_or(0, |q| q.raw());
        self.resting.push(Resting { side: order.side, price: limit, qty_remaining: order.qty.raw(), queue_ahead });
        self.orders_rested += 1;
    }

    fn execute(&mut self, order: LagOrder) {
        match order.kind {
            LagOrderKind::Market => match price_market(&self.book, order.side, order.qty) {
                Some(fill) => self.fill_taker(order.side, &fill),
                None => self.orders_rejected += 1,
            },
            LagOrderKind::Limit => self.place_limit(order),
        }
    }

    fn drain_pending(&mut self, now: u64) {
        while let Some(top) = self.pending.peek() {
            if top.0.arrival <= now {
                let Reverse(p) = self.pending.pop().expect("peek implies pop");
                self.execute(p.order);
            } else {
                break;
            }
        }
    }

    /// An HL trade prints: fill resting makers the tape ran through (queue first).
    fn process_trade(&mut self, aggr: Side, tprice: i64, tqty: i64) {
        if tqty <= 0 {
            return;
        }
        let mut i = 0;
        while i < self.resting.len() {
            let mut o = self.resting[i];
            let crosses = match (o.side, aggr) {
                (Side::Bid, Side::Ask) => tprice <= o.price,
                (Side::Ask, Side::Bid) => tprice >= o.price,
                _ => false,
            };
            if !crosses {
                i += 1;
                continue;
            }
            if o.queue_ahead >= tqty {
                o.queue_ahead -= tqty;
                self.resting[i] = o;
                i += 1;
                continue;
            }
            let avail = tqty - o.queue_ahead;
            o.queue_ahead = 0;
            let fill = avail.min(o.qty_remaining);
            if fill > 0 {
                let f = maker_fill(Price::from_raw(o.price), Qty::from_raw(fill));
                self.acct.apply_maker(o.side, &f, &self.fees, self.now);
                self.orders_filled += 1;
                self.maker_fills += 1;
                o.qty_remaining -= fill;
            }
            if o.qty_remaining == 0 {
                self.resting.remove(i);
            } else {
                self.resting[i] = o;
                i += 1;
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
            (Role::Exec, LagKind::Trade) => {
                if let Some(aggr) = ev.side {
                    self.process_trade(aggr, ev.price.raw(), ev.qty.raw());
                }
            }
            (Role::Reference, LagKind::Trade) => {
                let i = ev.src as usize;
                if self.ref_last.len() <= i {
                    self.ref_last.resize(i + 1, 0.0);
                }
                self.ref_last[i] = ev.price.to_f64();
                let (sum, cnt) = self.ref_last.iter().filter(|&&x| x > 0.0).fold((0.0, 0u32), |(s, c), &x| (s + x, c + 1));
                self.ref_px = if cnt > 0 { sum / f64::from(cnt) } else { 0.0 };
            }
            (Role::Reference, LagKind::BookDelta) => {}
            (Role::Lead, LagKind::Trade) => self.lead_px = ev.price.to_f64(),
            (Role::Lead, LagKind::BookDelta) => {}
            (_, LagKind::Funding) => self.funding = ev.aux,
        }

        self.buf.clear();
        {
            let ctx = LagCtx {
                now: t,
                exec_book: &self.book,
                ref_px: self.ref_px,
                funding: self.funding,
                lead_px: self.lead_px,
                position_qty: self.acct.net_qty(),
            };
            self.strat.on_event(&ctx, &mut self.buf);
        }

        for idx in 0..self.buf.len() {
            let order = self.buf[idx];
            let lat = self.next_latency();
            let arrival = t.checked_add(lat).ok_or("order arrival overflow")?;
            self.pending.push(Reverse(Pending { arrival, seq: self.seq, order }));
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
        let tc = self.acct.trip_close_ts();
        let mut trip_returns = Vec::with_capacity(tp.len());
        let mut trip_notionals = Vec::with_capacity(tp.len());
        for i in 0..tp.len() {
            if tn[i] > 0 {
                trip_returns.push((tc[i], money_to_f64(tp[i]) / money_to_f64(tn[i]) * 100.0));
                trip_notionals.push(money_to_f64(tn[i]));
            }
        }
        LagReport {
            events: self.events,
            orders_submitted: self.orders_submitted,
            orders_filled: self.orders_filled,
            maker_fills: self.maker_fills,
            orders_rested: self.orders_rested,
            orders_rejected: self.orders_rejected,
            round_trips: self.acct.round_trips(),
            net_pnl: money_to_f64(self.acct.net_pnl()),
            final_position: self.acct.net_qty(),
            trip_returns,
            trip_notionals,
        }
    }
}