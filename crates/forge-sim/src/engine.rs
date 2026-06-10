//! [`SimEngine`]: the virtual-clock event loop with an outbound order-latency
//! queue and a determinism hash.

use std::cmp::Reverse;
use std::collections::BinaryHeap;

use forge_book::OrderBook;
use forge_core::{Event, ForgeError, ForgeResult};

use crate::strategy::{Ctx, OrderIntent, Strategy};

const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
const FNV_PRIME: u64 = 0x0000_0100_0000_01b3;

#[inline]
fn fold_u64(h: &mut u64, x: u64) {
    for b in x.to_le_bytes() {
        *h ^= u64::from(b);
        *h = h.wrapping_mul(FNV_PRIME);
    }
}

/// Engine configuration.
#[derive(Clone, Copy, Debug)]
pub struct SimConfig {
    /// Nanoseconds from a strategy submitting an order to it reaching the
    /// matching engine (the outbound half of the two-clock latency model).
    pub order_latency_ns: u64,
    /// Max book levels kept per side (0 = unlimited); see `forge_book`.
    pub book_max_levels: usize,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self { order_latency_ns: 0, book_max_levels: 20 }
    }
}

/// Summary of a completed replay.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SimReport {
    /// Events processed.
    pub events: u64,
    /// Virtual clock at the last event (ns).
    pub last_ts: u64,
    /// Orders the strategy submitted.
    pub orders_submitted: u64,
    /// Orders that reached the (future) matching engine after latency.
    pub orders_reached: u64,
    /// Determinism hash over the run (events, intents, arrivals).
    pub det_hash: u64,
    /// Final book state hash.
    pub book_hash: u64,
}

/// The replay engine. One engine = one deterministic, single-threaded sim.
pub struct SimEngine<S: Strategy> {
    book: OrderBook,
    strat: S,
    cfg: SimConfig,
    now: u64,
    started: bool,
    /// Min-heap of `(arrival_ts, seq)` for orders in flight to the matcher.
    pending: BinaryHeap<Reverse<(u64, u64)>>,
    seq: u64,
    orders_submitted: u64,
    orders_reached: u64,
    events: u64,
    det_hash: u64,
    buf: Vec<OrderIntent>,
}

impl<S: Strategy> SimEngine<S> {
    /// Build an engine with the given strategy and config.
    pub fn new(strat: S, cfg: SimConfig) -> Self {
        Self {
            book: OrderBook::with_max_levels(cfg.book_max_levels),
            strat,
            cfg,
            now: 0,
            started: false,
            pending: BinaryHeap::new(),
            seq: 0,
            orders_submitted: 0,
            orders_reached: 0,
            events: 0,
            det_hash: FNV_OFFSET,
            buf: Vec::new(),
        }
    }

    /// Process one event: advance the clock, deliver arrived orders, fold the
    /// event into the book, run the strategy, and queue any new orders with
    /// outbound latency.
    ///
    /// # Errors
    /// [`ForgeError::NonMonotonicTs`] if `local_ts` goes backwards;
    /// [`ForgeError::Overflow`] if the order arrival time overflows; or any
    /// book error.
    pub fn step(&mut self, ev: &Event) -> ForgeResult<()> {
        let t = ev.local_ts.get();
        if self.started && t < self.now {
            return Err(ForgeError::NonMonotonicTs { prev: self.now, got: t });
        }
        self.now = t;
        self.started = true;

        // Outbound orders that have arrived by `now` reach the matcher.
        self.drain_pending(t);

        // The book reflects only events up to and including `now`.
        self.book.apply(ev)?;

        // Strategy reacts; ctx exposes only data with local_ts <= now.
        self.buf.clear();
        {
            let ctx = Ctx { now: ev.local_ts, event: ev, book: &self.book };
            self.strat.on_event(&ctx, &mut self.buf);
        }

        fold_u64(&mut self.det_hash, t);

        for idx in 0..self.buf.len() {
            let intent = self.buf[idx];
            let arrival = t
                .checked_add(self.cfg.order_latency_ns)
                .ok_or(ForgeError::Overflow { op: "order arrival" })?;
            self.pending.push(Reverse((arrival, self.seq)));
            self.seq += 1;
            self.orders_submitted += 1;
            fold_u64(&mut self.det_hash, u64::from(intent.side.as_u8()));
            fold_u64(&mut self.det_hash, intent.price.raw() as u64);
            fold_u64(&mut self.det_hash, intent.qty.raw() as u64);
            fold_u64(&mut self.det_hash, arrival);
        }

        self.events += 1;
        Ok(())
    }

    fn drain_pending(&mut self, now: u64) {
        while let Some(&Reverse((arrival, _))) = self.pending.peek() {
            if arrival <= now {
                self.pending.pop();
                self.orders_reached += 1;
                fold_u64(&mut self.det_hash, arrival);
            } else {
                break;
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
            det_hash: self.det_hash,
            book_hash: self.book.state_hash(),
        }
    }

    /// The reconstructed book.
    #[must_use]
    pub fn book(&self) -> &OrderBook {
        &self.book
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy::{NoopStrategy, Strategy};
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

    /// Asserts no-lookahead at every call by bumping a shared counter on any
    /// violation (book ahead of clock, or event ts != clock).
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

    /// Emits exactly one order on the first event.
    struct OnceStrategy {
        fired: bool,
    }
    impl Strategy for OnceStrategy {
        fn on_event(&mut self, _ctx: &Ctx, out: &mut Vec<OrderIntent>) {
            if !self.fired {
                self.fired = true;
                out.push(OrderIntent {
                    side: Side::Bid,
                    price: Price::from_raw(100),
                    qty: Qty::from_raw(1),
                });
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
        assert_eq!(violations.get(), 0, "strategy must never see data ahead of the clock");
    }

    #[test]
    fn non_monotonic_event_fails_fast() {
        let mut eng = SimEngine::new(NoopStrategy, SimConfig::default());
        eng.step(&bookdelta(Side::Bid, 100, 5, 100)).unwrap();
        // an earlier local_ts must be rejected
        assert!(eng.step(&bookdelta(Side::Bid, 100, 5, 99)).is_err());
    }

    #[test]
    fn determinism_two_runs_match() {
        let evs = stream();
        let mut a = SimEngine::new(NoopStrategy, SimConfig::default());
        let mut b = SimEngine::new(NoopStrategy, SimConfig::default());
        a.run(evs.iter()).unwrap();
        b.run(evs.iter()).unwrap();
        let ra = a.finish();
        let rb = b.finish();
        assert_eq!(ra, rb);
    }

    #[test]
    fn order_latency_delays_arrival() {
        // order submitted at t=10, latency 5 -> arrives at 15. Events only reach
        // t=11, so during the run nothing arrives; finish() drains the rest.
        let cfg = SimConfig { order_latency_ns: 5, book_max_levels: 20 };
        let mut eng = SimEngine::new(OnceStrategy { fired: false }, cfg);
        eng.step(&bookdelta(Side::Bid, 100, 5, 10)).unwrap();
        eng.step(&bookdelta(Side::Ask, 101, 4, 11)).unwrap();
        assert_eq!(eng.orders_submitted(), 1);
        assert_eq!(eng.orders_reached(), 0, "order must not arrive before t+latency");
        let r = eng.finish();
        assert_eq!(r.orders_submitted, 1);
        assert_eq!(r.orders_reached, 1);
    }

    #[test]
    fn order_arrives_when_clock_passes_latency() {
        let cfg = SimConfig { order_latency_ns: 5, book_max_levels: 20 };
        let mut eng = SimEngine::new(OnceStrategy { fired: false }, cfg);
        eng.step(&bookdelta(Side::Bid, 100, 5, 10)).unwrap(); // submit, arrival 15
        eng.step(&bookdelta(Side::Ask, 101, 4, 16)).unwrap(); // clock 16 >= 15
        assert_eq!(eng.orders_reached(), 1, "order should arrive once clock passes 15");
    }
}