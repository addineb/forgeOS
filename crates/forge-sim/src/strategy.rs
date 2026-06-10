//! The strategy seam: the read-only [`Ctx`] handed to a strategy each event,
//! the [`OrderIntent`] it may emit, and the [`Strategy`] trait itself.

use forge_book::OrderBook;
use forge_core::{Event, Price, Qty, Side, UnixNanos};

/// Read-only context passed to a strategy on each event.
///
/// By construction it exposes only information available at `now`: the event
/// that just arrived, the book folded up to and including it, and our current
/// position. There is no API to read a future event - no-lookahead is enforced
/// structurally.
pub struct Ctx<'a> {
    /// The virtual clock = `event.local_ts`.
    pub now: UnixNanos,
    /// The event that just arrived at `now`.
    pub event: &'a Event,
    /// The order book reflecting all events with `local_ts <= now`.
    pub book: &'a OrderBook,
    /// Our signed position size (raw qty): positive long, negative short.
    pub position_qty: i64,
}

/// How an order executes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OrderKind {
    /// Cross the spread now: buy lifts the ask, sell hits the bid (taker).
    Market,
}

/// An order a strategy wants to send. `side` is OUR side: `Bid` = buy, `Ask` =
/// sell. For `Market`, `price` is unused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrderIntent {
    /// Buy (bid) or sell (ask).
    pub side: Side,
    /// Execution style.
    pub kind: OrderKind,
    /// Limit price (unused for `Market`; reserved for future limit orders).
    pub price: Price,
    /// Quantity (fixed-point).
    pub qty: Qty,
}

impl OrderIntent {
    /// A market (taker) order.
    #[must_use]
    pub fn market(side: Side, qty: Qty) -> Self {
        Self { side, kind: OrderKind::Market, price: Price::ZERO, qty }
    }
}

/// A trading strategy: pure reaction to `(event, book, position, now)`.
pub trait Strategy {
    /// Called once per event, after the event is applied to the book. Push any
    /// orders into `out`; the engine clears `out` before each call.
    fn on_event(&mut self, ctx: &Ctx, out: &mut Vec<OrderIntent>);
}

/// A strategy that never trades - the baseline for replay + determinism checks.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoopStrategy;

impl Strategy for NoopStrategy {
    #[inline]
    fn on_event(&mut self, _ctx: &Ctx, _out: &mut Vec<OrderIntent>) {}
}