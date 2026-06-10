//! [`SimEngine`]: virtual-clock event loop with an outbound order-latency
//! queue, taker AND maker (queue-position) fills, P&L accounting, and a
//! determinism hash.

use std::cmp::Ordering;
use std::cmp::Reverse;
use std::collections::BinaryHeap;

use forge_book::OrderBook;
use forge_core::{Event, EventKind, ForgeError, ForgeResult, Side};

use crate::account::Account;
use crate::fills::{maker_fill, price_market, price_to_limit, FeeSchedule, Money};
use crate::strategy::{Ctx, OrderIntent, OrderKind, Strategy};

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

#[inline]
fn fold_u64(h: &mut u64, x: u64) {
    for b in x.to_le_bytes() {
        *h ^= u64::from(b);
        *h = h.wrapping_mul(FNV_PRIME);
    }
}

/// An order in flight to the matcher, ordered by `(arrival, seq)`.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Pending {
    arrival: u64,
    seq: u64,
    intent: OrderIntent,
}

impl Ord for Pending {
    fn cmp(&self, other: &Self) -> Ordering {
        self.arrival.cmp(&other.arrival).then(self.seq.cmp(&other.seq))
    }
}
impl PartialOrd for Pending {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// A passive maker order resting in the book, tracked by queue position.
#[derive(Clone, Copy)]
struct Resting {
    side: Side,
    price: i64,
    qty_remaining: i64,
    /// Volume that must trade through our price before WE start filling.
    queue_ahead: i64,
}

/// Engine configuration.
#[derive(Clone, Copy, Debug)]
pub struct SimConfig {
    /// Nanoseconds from a strategy submitting an order to it reaching the
    /// matcher (outbound half of the two-clock latency model).
    pub order_latency_ns: u64,
    /// Max book levels kept per side (0 = unlimited).
    pub book_max_levels: usize,
    /// Fee schedule applied to fills.
    pub fees: FeeSchedule,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self { order_latency_ns: 0, book_max_levels: 20, fees: FeeSchedule::legacy() }
    }
}

/// Summary of a completed replay. All money is `i128` quote-scaled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SimReport {
    /// Events processed.
    pub events: u64,
    /// Virtual clock at the last event (ns).
    pub last_ts: u64,
    /// Orders the strategy submitted.
    pub orders_submitted: u64,
    /// Orders that reached the matcher after latency.
    pub orders_reached: u64,
    /// Fills executed (taker + maker).
    pub orders_filled: u64,
    /// Of the fills, how many were maker (queue) fills.
    pub maker_fills: u64,
    /// Limit orders that rested passively.
    pub orders_rested: u64,
    /// Market/marketable orders that found no liquidity.
    pub orders_rejected: u64,
    /// Limit orders still resting (unfilled) at the end.
    pub resting_open: u64,
    /// Realized gross P&L (before fees).
    pub realized: Money,
    /// Total fees paid (negative = net rebate).
    pub fees: Money,
    /// Net P&L = realized - fees.
    pub net_pnl: Money,
    /// Completed round trips.
    pub round_trips: u64,
    /// Final signed position (raw qty).
    pub final_position: i64,
    /// Determinism hash over the run.
    pub det_hash: u64,
    /// Final book state hash.
    pub book_hash: u64,
}

/// The replay engine. One engine = one deterministic, single-threaded sim.
pub struct SimEngine<S: Strategy> {
    book: OrderBook,
    account: Account,
    strat: S,
    fees: FeeSchedule,
    order_latency_ns: u64,
    now: u64,
    started: bool,
    pending: BinaryHeap<Reverse<Pending>>,
    resting: Vec<Resting>,
    seq: u64,
    orders_submitted: u64,
    orders_reached: u64,
    orders_filled: u64,
    maker_fills: u64,
    orders_rested: u64,
    orders_rejected: u64,
    events: u64,
    det_hash: u64,
    buf: Vec<OrderIntent>,
    equity_sample_ns: u64,
    next_sample: u64,
    sampling_started: bool,
    equity: Vec<(u64, i128)>,
}

impl<S: Strategy> SimEngine<S> {
    /// Build an engine with the given strategy and config.
    pub fn new(strat: S, cfg: SimConfig) -> Self {
        Self {
            book: OrderBook::with_max_levels(cfg.book_max_levels),
            account: Account::new(),
            strat,
            fees: cfg.fees,
            order_latency_ns: cfg.order_latency_ns,
            now: 0,
            started: false,
            pending: BinaryHeap::new(),
            resting: Vec::new(),
            seq: 0,
            orders_submitted: 0,
            orders_reached: 0,
            orders_filled: 0,
            maker_fills: 0,
            orders_rested: 0,
            orders_rejected: 0,
            events: 0,
            det_hash: FNV_OFFSET,
            buf: Vec::new(),
            equity_sample_ns: 0,
            next_sample: 0,
            sampling_started: false,
            equity: Vec::new(),
        }
    }

    /// Sample the equity curve every interval_ns of virtual time (0 = off).
    pub fn enable_equity_sampling(&mut self, interval_ns: u64) {
        self.equity_sample_ns = interval_ns;
    }

    /// The sampled equity curve as (ts, net_pnl_including_unrealized).
    #[must_use]
    pub fn equity_curve(&self) -> &[(u64, i128)] {
        &self.equity
    }

    /// Process one event.
    ///
    /// # Errors
    /// [`ForgeError::NonMonotonicTs`] if `local_ts` goes backwards;
    /// [`ForgeError::Overflow`] on order-arrival overflow; or a book error.
    pub fn step(&mut self, ev: &Event) -> ForgeResult<()> {
        let t = ev.local_ts.get();
        if self.started && t < self.now {
            return Err(ForgeError::NonMonotonicTs { prev: self.now, got: t });
        }
        self.now = t;
        self.started = true;

        // Orders that have arrived by `now` are placed/filled against the book
        // as it stands before this event (causal: data <= arrival only).
        self.drain_pending(t);

        // The book reflects only events up to and including `now`.
        self.book.apply(ev)?;

        // A trade prints through resting maker orders (queue-position fills).
        if ev.kind == EventKind::Trade {
            self.process_trade(ev);
        }

        // Strategy reacts; ctx exposes only data with local_ts <= now.
        self.buf.clear();
        {
            let ctx = Ctx {
                now: ev.local_ts,
                event: ev,
                book: &self.book,
                position_qty: self.account.net_qty(),
            };
            self.strat.on_event(&ctx, &mut self.buf);
        }

        fold_u64(&mut self.det_hash, t);

        for idx in 0..self.buf.len() {
            let intent = self.buf[idx];
            let arrival = t
                .checked_add(self.order_latency_ns)
                .ok_or(ForgeError::Overflow { op: "order arrival" })?;
            self.pending.push(Reverse(Pending { arrival, seq: self.seq, intent }));
            self.seq += 1;
            self.orders_submitted += 1;
            fold_u64(&mut self.det_hash, u64::from(intent.side.as_u8()));
            fold_u64(&mut self.det_hash, intent.qty.raw() as u64);
            fold_u64(&mut self.det_hash, arrival);
        }

        if self.equity_sample_ns > 0 {
            if !self.sampling_started {
                self.next_sample = t;
                self.sampling_started = true;
            }
            if t >= self.next_sample {
                let mark = self.book.mid().map(forge_core::Price::raw);
                let unreal = mark.map_or(0, |m| self.account.unrealized(m));
                self.equity.push((t, self.account.net_pnl() + unreal));
                while self.next_sample <= t {
                    self.next_sample += self.equity_sample_ns;
                }
            }
        }

        self.events += 1;
        Ok(())
    }

    fn drain_pending(&mut self, now: u64) {
        while let Some(top) = self.pending.peek() {
            if top.0.arrival <= now {
                let Reverse(p) = self.pending.pop().expect("peek implies pop");
                fold_u64(&mut self.det_hash, p.arrival);
                self.execute(p.intent);
            } else {
                break;
            }
        }
    }

    fn execute(&mut self, intent: OrderIntent) {
        self.orders_reached += 1;
        match intent.kind {
            OrderKind::Market => match price_market(&self.book, intent.side, intent.qty) {
                Some(fill) => self.fill_taker(intent.side, &fill),
                None => self.orders_rejected += 1,
            },
            OrderKind::Limit => self.place_limit(intent),
        }
    }

    fn fill_taker(&mut self, side: Side, fill: &crate::fills::Fill) {
        self.account.apply_taker(side, fill, &self.fees);
        self.orders_filled += 1;
        fold_u64(&mut self.det_hash, fill.avg_price.raw() as u64);
        fold_u64(&mut self.det_hash, fill.filled.raw() as u64);
        fold_u64(&mut self.det_hash, u64::from(side.as_u8()));
    }

    fn place_limit(&mut self, intent: OrderIntent) {
        let limit = intent.price.raw();
        let best_ask = self.book.best_ask().map(|(p, _)| p.raw());
        let best_bid = self.book.best_bid().map(|(p, _)| p.raw());
        let marketable = match intent.side {
            Side::Bid => best_ask.is_some_and(|a| limit >= a),
            Side::Ask => best_bid.is_some_and(|b| limit <= b),
        };
        if marketable {
            // a marketable limit crosses now as a taker, capped at its price
            match price_to_limit(&self.book, intent.side, intent.qty, Some(limit)) {
                Some(fill) => self.fill_taker(intent.side, &fill),
                None => self.orders_rejected += 1,
            }
            return;
        }
        let queue_ahead = self.book.qty_at(intent.side, intent.price).map_or(0, |q| q.raw());
        self.resting.push(Resting {
            side: intent.side,
            price: limit,
            qty_remaining: intent.qty.raw(),
            queue_ahead,
        });
        self.orders_rested += 1;
    }

    /// A trade prints: clear queues and fill resting makers the tape ran through.
    fn process_trade(&mut self, trade: &Event) {
        let Some(aggr) = trade.side else { return };
        let tprice = trade.price.raw();
        let tqty = trade.qty.raw();
        if tqty <= 0 {
            return;
        }
        let mut i = 0;
        while i < self.resting.len() {
            let mut o = self.resting[i];
            // our resting buy is filled by aggressive sells at/below our price;
            // our resting sell by aggressive buys at/above our price.
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
                let f = maker_fill(forge_core::Price::from_raw(o.price), forge_core::Qty::from_raw(fill));
                self.account.apply_maker(o.side, &f, &self.fees);
                self.orders_filled += 1;
                self.maker_fills += 1;
                fold_u64(&mut self.det_hash, o.price as u64);
                fold_u64(&mut self.det_hash, fill as u64);
                fold_u64(&mut self.det_hash, u64::from(o.side.as_u8()));
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

    /// Run over a sequence of events.
    ///
    /// # Errors
    /// Propagates any [`SimEngine::step`] error.
    pub fn run<'a, I: IntoIterator<Item = &'a Event>>(&mut self, events: I) -> ForgeResult<()> {
        for ev in events {
            self.step(ev)?;
        }
        Ok(())
    }

    /// Drain orders still in flight and produce the final report.
    #[must_use]
    pub fn finish(mut self) -> SimReport {
        self.drain_pending(u64::MAX);
        SimReport {
            events: self.events,
            last_ts: self.now,
            orders_submitted: self.orders_submitted,
            orders_reached: self.orders_reached,
            orders_filled: self.orders_filled,
            maker_fills: self.maker_fills,
            orders_rested: self.orders_rested,
            orders_rejected: self.orders_rejected,
            resting_open: self.resting.len() as u64,
            realized: self.account.realized(),
            fees: self.account.fees(),
            net_pnl: self.account.net_pnl(),
            round_trips: self.account.round_trips(),
            final_position: self.account.net_qty(),
            det_hash: self.det_hash,
            book_hash: self.book.state_hash(),
        }
    }

    /// The reconstructed book.
    #[must_use]
    pub fn book(&self) -> &OrderBook {
        &self.book
    }

    /// The account (position + P&L).
    #[must_use]
    pub fn account(&self) -> &Account {
        &self.account
    }

    /// Current virtual clock (ns).
    #[must_use]
    pub fn now(&self) -> u64 {
        self.now
    }

    /// Orders submitted so far.
    #[must_use]
    pub fn orders_submitted(&self) -> u64 {
        self.orders_submitted
    }

    /// Orders that have reached the matcher so far.
    #[must_use]
    pub fn orders_reached(&self) -> u64 {
        self.orders_reached
    }

    /// Maker fills so far.
    #[must_use]
    pub fn maker_fills(&self) -> u64 {
        self.maker_fills
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy::{NoopStrategy, OrderIntent, Strategy};
    use forge_core::{Event, EventKind, Price, Qty, Side, UnixNanos};
    use std::cell::Cell;
    use std::rc::Rc;

    fn bookdelta(side: Side, px: i64, qty: i64, ts: u64) -> Event {
        Event::new(
            EventKind::BookDelta,
            UnixNanos::new(ts),
            UnixNanos::new(ts),
            Some(side),
            Price::from_raw(px),
            Qty::from_raw(qty),
            0,
        )
        .unwrap()
    }

    fn trade(aggr: Side, px: i64, qty: i64, ts: u64) -> Event {
        Event::new(
            EventKind::Trade,
            UnixNanos::new(ts),
            UnixNanos::new(ts),
            Some(aggr),
            Price::from_raw(px),
            Qty::from_raw(qty),
            0,
        )
        .unwrap()
    }

    struct Spy {
        violations: Rc<Cell<u32>>,
    }
    impl Strategy for Spy {
        fn on_event(&mut self, ctx: &Ctx, _out: &mut Vec<OrderIntent>) {
            if ctx.book.last_ts() > ctx.now.get() || ctx.event.local_ts != ctx.now {
                self.violations.set(self.violations.get() + 1);
            }
        }
    }

    struct OnceStrategy {
        fired: bool,
    }
    impl Strategy for OnceStrategy {
        fn on_event(&mut self, _ctx: &Ctx, out: &mut Vec<OrderIntent>) {
            if !self.fired {
                self.fired = true;
                out.push(OrderIntent::market(Side::Bid, Qty::from_raw(100_000_000)));
            }
        }
    }

    /// Emits one buy LIMIT at a fixed price on the first event.
    struct MakerOnce {
        fired: bool,
        price: i64,
        qty: i64,
    }
    impl Strategy for MakerOnce {
        fn on_event(&mut self, _ctx: &Ctx, out: &mut Vec<OrderIntent>) {
            if !self.fired {
                self.fired = true;
                out.push(OrderIntent::limit(Side::Bid, Price::from_raw(self.price), Qty::from_raw(self.qty)));
            }
        }
    }

    fn stream() -> Vec<Event> {
        vec![
            bookdelta(Side::Bid, 100, 5, 10),
            bookdelta(Side::Ask, 101, 4, 11),
            bookdelta(Side::Bid, 100, 7, 12),
            bookdelta(Side::Ask, 102, 3, 13),
        ]
    }

    #[test]
    fn no_lookahead_holds() {
        let violations = Rc::new(Cell::new(0u32));
        let mut eng = SimEngine::new(Spy { violations: violations.clone() }, SimConfig::default());
        eng.run(stream().iter()).unwrap();
        let _ = eng.finish();
        assert_eq!(violations.get(), 0);
    }

    #[test]
    fn non_monotonic_event_fails_fast() {
        let mut eng = SimEngine::new(NoopStrategy, SimConfig::default());
        eng.step(&bookdelta(Side::Bid, 100, 5, 100)).unwrap();
        assert!(eng.step(&bookdelta(Side::Bid, 100, 5, 99)).is_err());
    }

    #[test]
    fn determinism_two_runs_match() {
        let evs = stream();
        let mut a = SimEngine::new(NoopStrategy, SimConfig::default());
        let mut b = SimEngine::new(NoopStrategy, SimConfig::default());
        a.run(evs.iter()).unwrap();
        b.run(evs.iter()).unwrap();
        assert_eq!(a.finish(), b.finish());
    }

    #[test]
    fn order_latency_delays_arrival() {
        let cfg = SimConfig { order_latency_ns: 5, ..SimConfig::default() };
        let mut eng = SimEngine::new(OnceStrategy { fired: false }, cfg);
        eng.step(&bookdelta(Side::Bid, 100, 5, 10)).unwrap();
        eng.step(&bookdelta(Side::Ask, 101, 4, 11)).unwrap();
        assert_eq!(eng.orders_submitted(), 1);
        assert_eq!(eng.orders_reached(), 0);
        let r = eng.finish();
        assert_eq!(r.orders_reached, 1);
    }

    #[test]
    fn maker_fills_only_after_queue_cleared() {
        // best bid 100 with size 5 resting ahead of us; we post a buy limit at
        // 100 for qty 1. Aggressive sells must clear the 5 ahead before we fill.
        let cfg = SimConfig { order_latency_ns: 0, book_max_levels: 20, fees: FeeSchedule::zero() };
        let mut eng = SimEngine::new(MakerOnce { fired: false, price: 100, qty: 1 }, cfg);
        eng.step(&bookdelta(Side::Bid, 100, 5, 10)).unwrap(); // emits the limit
        // order is placed at next drain; feed a small sell that only eats queue
        eng.step(&trade(Side::Ask, 100, 3, 11)).unwrap();
        assert_eq!(eng.maker_fills(), 0, "queue (5) not yet cleared by 3");
        // another sell of 4: clears remaining queue (2) then fills our 1
        eng.step(&trade(Side::Ask, 100, 4, 12)).unwrap();
        assert_eq!(eng.maker_fills(), 1, "queue cleared -> we fill");
        assert_eq!(eng.account().net_qty(), 1, "we are now long 1 via the maker fill");
    }

    #[test]
    fn marketable_limit_crosses_as_taker() {
        // buy limit at 105 while best ask is 101 -> marketable -> taker fill now
        let cfg = SimConfig { order_latency_ns: 0, book_max_levels: 20, fees: FeeSchedule::zero() };
        let mut eng = SimEngine::new(
            MakerOnce { fired: false, price: 105, qty: 1 },
            cfg,
        );
        eng.step(&bookdelta(Side::Ask, 101, 5, 10)).unwrap(); // emits limit buy @105
        eng.step(&bookdelta(Side::Bid, 99, 5, 11)).unwrap(); // drain places/fills it
        let r = eng.finish();
        assert_eq!(r.maker_fills, 0);
        assert_eq!(r.orders_filled, 1, "marketable limit filled as taker");
        assert_eq!(r.final_position, 1);
    }
}