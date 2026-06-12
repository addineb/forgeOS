//! HONEST MAKER FILL MODEL - the anti-lie unit tests (spec lagshot-maker, Task 2.1).
//!
//! These prove the forgelag engine's resting-maker fills behave CONSERVATIVELY,
//! the single component most able to manufacture a fake edge:
//!   * a through-trade fills AT THE RESTING LIMIT price (never mid, never improved),
//!   * queue-ahead protects the order (FIFO) and is only ever decremented by trades,
//!   * trade volume beyond the queue but below the order size PARTIALLY fills, and
//!     the remainder keeps resting (fillable by a later through-trade).
//!
//! All scenarios use the PUBLIC engine API: a tiny scripted `LagStrategy` rests one
//! maker order, the book is seeded with `BookDelta` events, and HL `Trade` events
//! (aggressor side) drive the fills. Latency is 0 so a placed order rests at the
//! start of the next processed event, deterministically.
//!
//! Requirements: 6.1, 6.2, 6.3, 6.4.

use forge_core::{Price, Qty, Side};
use forgelag::{
    FeeSchedule, LagConfig, LagCtx, LagEngine, LagEvent, LagKind, LagOrder, LagStrategy, Role,
};

fn cfg() -> LagConfig {
    LagConfig {
        order_latency_ns: 0,
        cancel_latency_ns: 0,
        exec_book_levels: 20,
        fees: FeeSchedule::legacy(),
    }
}

/// A HL execution-book level update (qty 0 would remove it).
fn book_delta(side: Side, px: f64, qty: f64, ts: u64) -> LagEvent {
    LagEvent {
        role: Role::Exec,
        kind: LagKind::BookDelta,
        exch_ts: ts,
        local_ts: ts,
        side: Some(side),
        price: Price::from_f64(px).unwrap(),
        qty: Qty::from_f64(qty).unwrap(),
        src: 0,
        aux: 0.0,
    }
}

/// An HL trade print. `aggr` is the AGGRESSOR side: an Ask-aggressor trade at or
/// below a resting bid's price prints THROUGH it (executes the resting bid).
fn hl_trade(aggr: Side, px: f64, qty: f64, ts: u64) -> LagEvent {
    LagEvent {
        role: Role::Exec,
        kind: LagKind::Trade,
        exch_ts: ts,
        local_ts: ts,
        side: Some(aggr),
        price: Price::from_f64(px).unwrap(),
        qty: Qty::from_f64(qty).unwrap(),
        src: 0,
        aux: 0.0,
    }
}

/// Rests exactly ONE maker order, once, as soon as both sides of the book exist;
/// never cancels and never exits (so the report's `final_position` reflects the
/// fills directly).
struct PlaceOnce {
    placed: bool,
    id: u64,
    side: Side,
    px: f64,
    qty: f64,
}
impl LagStrategy for PlaceOnce {
    fn on_event(&mut self, ctx: &LagCtx, out: &mut Vec<LagOrder>) {
        if !self.placed && ctx.exec_book.best_bid().is_some() && ctx.exec_book.best_ask().is_some() {
            out.push(LagOrder::place(
                self.id,
                self.side,
                Price::from_f64(self.px).unwrap(),
                Qty::from_f64(self.qty).unwrap(),
            ));
            self.placed = true;
        }
    }
}

/// Rests one maker BID; once it fills (we go long), closes with a taker market
/// sell. Used to record a complete round trip so the entry FILL PRICE is
/// observable through `trip_notionals` (entry notional / qty = entry price).
struct MakerThenExit {
    placed: bool,
    px: f64,
    qty: f64,
}
impl LagStrategy for MakerThenExit {
    fn on_event(&mut self, ctx: &LagCtx, out: &mut Vec<LagOrder>) {
        if ctx.position_qty > 0 {
            out.push(LagOrder::market(Side::Ask, Qty::from_raw(ctx.position_qty)));
        } else if !self.placed
            && ctx.exec_book.best_bid().is_some()
            && ctx.exec_book.best_ask().is_some()
        {
            out.push(LagOrder::place(
                1,
                Side::Bid,
                Price::from_f64(self.px).unwrap(),
                Qty::from_f64(self.qty).unwrap(),
            ));
            self.placed = true;
        }
    }
}

/// Req 6.1 / 6.2: a resting bid is filled by an Ask-aggressor trade that prints
/// THROUGH it, AT THE RESTING LIMIT - never at mid, never price-improved.
#[test]
fn through_trade_fills_at_resting_limit_never_mid_never_improved() {
    // Book: bid 99 / ask 103  => mid = 101. Rest a maker BID at 100 (a clean level
    // with no queue ahead). An Ask-aggressor HL trade prints THROUGH at 99.5.
    let evs = [
        book_delta(Side::Bid, 99.0, 1000.0, 1_000),
        book_delta(Side::Ask, 103.0, 1000.0, 2_000), // -> rest bid@100 q5 (queue_ahead 0)
        hl_trade(Side::Ask, 99.5, 5.0, 3_000),       // prints through 100 -> fills us
    ];
    let strat = MakerThenExit { placed: false, px: 100.0, qty: 5.0 };
    let mut eng = LagEngine::new(strat, cfg());
    eng.run(evs.iter()).unwrap();
    let r = eng.finish();

    assert_eq!(r.maker_fills, 1, "the resting bid must take exactly one maker fill");
    assert_eq!(r.orders_filled, 2, "one maker entry + one taker exit");
    assert_eq!(r.round_trips, 1, "entry then exit = one completed round trip");
    assert_eq!(r.final_position, 0, "the round trip leaves us flat");

    // The entry notional proves the FILL PRICE: resting limit 100.0 * 5.0 = 500.0.
    // A mid fill (101) would give 505.0; a price-improved fill at the trade price
    // 99.5 would give 497.5. Only the resting-limit fill yields 500.0.
    let entry_notional = r.trip_notionals[0];
    assert!(
        (entry_notional - 500.0).abs() < 1e-6,
        "maker must fill at the resting limit 100 (notional 500), got {entry_notional}"
    );
}

/// Req 6.3 (protection half): a trade whose volume is within the queue ahead
/// fills the queue, NOT us - we stay unfilled.
#[test]
fn queue_ahead_blocks_fill_when_trade_within_queue() {
    // Seed 10 units resting AT our price (100) BEFORE we join -> queue_ahead = 10.
    let evs = [
        book_delta(Side::Bid, 100.0, 10.0, 1_000),   // the queue at our level
        book_delta(Side::Ask, 103.0, 1000.0, 2_000), // -> rest bid@100 q5 (queue_ahead 10)
        hl_trade(Side::Ask, 99.5, 6.0, 3_000),       // 6 <= 10 -> no fill, queue -> 4
    ];
    let strat = PlaceOnce { placed: false, id: 1, side: Side::Bid, px: 100.0, qty: 5.0 };
    let mut eng = LagEngine::new(strat, cfg());
    eng.run(evs.iter()).unwrap();
    let r = eng.finish();

    assert_eq!(r.orders_rested, 1, "the maker bid rested behind a queue of 10");
    assert_eq!(r.maker_fills, 0, "a trade within the queue-ahead must NOT fill us");
    assert_eq!(r.final_position, 0, "still flat: someone ahead of us got that trade");
}

/// Req 6.3 (decrement half): the blocked trade DECREMENTS the queue (FIFO), so a
/// later trade that exceeds the remaining queue finally reaches us.
#[test]
fn queue_decrements_then_a_later_trade_fills_the_residual() {
    // Queue of 10. Two trades of 6: the first leaves queue 4 (no fill); the second
    // (6) exceeds the remaining 4 -> residual (12 total - 10 queue) = 2 fills us.
    let evs = [
        book_delta(Side::Bid, 100.0, 10.0, 1_000),
        book_delta(Side::Ask, 103.0, 1000.0, 2_000),
        hl_trade(Side::Ask, 99.5, 6.0, 3_000), // queue 10 -> 4, no fill
        hl_trade(Side::Ask, 99.5, 6.0, 4_000), // 6 > 4 -> fill the residual 2 @ 100
    ];
    let strat = PlaceOnce { placed: false, id: 1, side: Side::Bid, px: 100.0, qty: 5.0 };
    let mut eng = LagEngine::new(strat, cfg());
    eng.run(evs.iter()).unwrap();
    let r = eng.finish();

    assert_eq!(r.maker_fills, 1, "the second trade finally reaches us after the queue drains");
    let two = Qty::from_f64(2.0).unwrap().raw();
    assert_eq!(
        r.final_position, two,
        "only the residual (12 cumulative - 10 queue) = 2 fills us; proves the queue was decremented"
    );
}

/// Req 6.4 (partial half): trade volume beyond the (zero) queue but below the
/// order size partially fills - only the residual volume.
#[test]
fn partial_fill_takes_only_the_residual_volume() {
    // Clean level (no queue). A trade of 2 < order size 5 -> partial fill of 2.
    let evs = [
        book_delta(Side::Bid, 99.0, 1000.0, 1_000),
        book_delta(Side::Ask, 103.0, 1000.0, 2_000), // -> rest bid@100 q5 (queue_ahead 0)
        hl_trade(Side::Ask, 99.5, 2.0, 3_000),       // 2 < 5 -> partial fill of 2
    ];
    let strat = PlaceOnce { placed: false, id: 1, side: Side::Bid, px: 100.0, qty: 5.0 };
    let mut eng = LagEngine::new(strat, cfg());
    eng.run(evs.iter()).unwrap();
    let r = eng.finish();

    assert_eq!(r.maker_fills, 1, "the through-trade partially fills us");
    let two = Qty::from_f64(2.0).unwrap().raw();
    assert_eq!(r.final_position, two, "only the 2 residual fills; 3 remain resting");
}

/// Req 6.4 (remainder half): after a partial fill the remainder keeps resting
/// (with queue_ahead 0) and is filled by a LATER through-trade.
#[test]
fn partial_remainder_keeps_resting_and_a_later_trade_fills_it() {
    let evs = [
        book_delta(Side::Bid, 99.0, 1000.0, 1_000),
        book_delta(Side::Ask, 103.0, 1000.0, 2_000),
        hl_trade(Side::Ask, 99.5, 2.0, 3_000),  // partial fill 2 (remainder 3 rests)
        hl_trade(Side::Ask, 99.5, 10.0, 4_000), // fills the remaining 3
    ];
    let strat = PlaceOnce { placed: false, id: 1, side: Side::Bid, px: 100.0, qty: 5.0 };
    let mut eng = LagEngine::new(strat, cfg());
    eng.run(evs.iter()).unwrap();
    let r = eng.finish();

    assert_eq!(r.maker_fills, 2, "two separate maker fills: 2 then 3");
    let five = Qty::from_f64(5.0).unwrap().raw();
    assert_eq!(
        r.final_position, five,
        "the remainder rested and was filled by the later trade -> full 5"
    );
}

/// Req 6.5 / 6.6 - THE KILLER CASE: a clean reversion with NO HL trade printing
/// through the resting price leaves the order UNFILLED. The microprice stretches
/// (book moves) and the gap reverts, but the only HL trades either sit ABOVE our
/// resting bid (an Ask-aggressor that does not reach us) or come from the WRONG
/// aggressor (a Bid-aggressor). This is the exact adverse-selection ESCAPE the
/// naive maker missed: we only fill on continuation through us, never on the
/// snap-back. An honest model must record zero fills here.
#[test]
fn clean_reversion_without_a_through_trade_never_fills() {
    let evs = [
        book_delta(Side::Bid, 99.0, 1000.0, 1_000),
        book_delta(Side::Ask, 103.0, 1000.0, 2_000), // -> rest bid@100 q5 (queue_ahead 0)
        // Stretch: the ask tightens toward our bid (microprice drifts down)...
        book_delta(Side::Ask, 100.5, 1000.0, 3_000),
        // ...an Ask-aggressor trade prints, but ABOVE our 100 -> does NOT print through us.
        hl_trade(Side::Ask, 100.5, 50.0, 4_000),
        // A Bid-aggressor trade prints at 99 (<=100) but the WRONG aggressor -> no fill.
        hl_trade(Side::Bid, 99.0, 50.0, 5_000),
        // Revert: the book snaps back out, gap closes, still no trade through 100.
        book_delta(Side::Ask, 103.0, 1000.0, 6_000),
        hl_trade(Side::Ask, 101.0, 50.0, 7_000), // still above 100 -> no fill
    ];
    let strat = PlaceOnce { placed: false, id: 1, side: Side::Bid, px: 100.0, qty: 5.0 };
    let mut eng = LagEngine::new(strat, cfg());
    eng.run(evs.iter()).unwrap();
    let r = eng.finish();

    assert_eq!(r.orders_rested, 1, "the maker bid rested (non-marketable at 100)");
    assert_eq!(r.maker_fills, 0, "a clean reversion with no through-trade must NEVER fill");
    assert_eq!(r.orders_filled, 0, "no fills of any kind");
    assert_eq!(r.final_position, 0, "we stay flat: the gap reverted around us, untouched");
}

/// Rests one maker order on the first event where both sides exist, then issues a
/// `Cancel(id)` on the very next event. With `order_latency_ns = 0` the place rests
/// at the top of the next step; the cancel then travels on `cancel_latency_ns`, so
/// the order stays resting (and fillable) until the cancel lands. Lets the tests
/// straddle the cancel-arrival boundary with the trade timestamp.
struct PlaceThenCancel {
    id: u64,
    side: Side,
    px: f64,
    qty: f64,
    placed: bool,
    cancelled: bool,
}
impl LagStrategy for PlaceThenCancel {
    fn on_event(&mut self, ctx: &LagCtx, out: &mut Vec<LagOrder>) {
        if !self.placed {
            if ctx.exec_book.best_bid().is_some() && ctx.exec_book.best_ask().is_some() {
                out.push(LagOrder::place(
                    self.id,
                    self.side,
                    Price::from_f64(self.px).unwrap(),
                    Qty::from_f64(self.qty).unwrap(),
                ));
                self.placed = true;
            }
        } else if !self.cancelled {
            out.push(LagOrder::cancel(self.id));
            self.cancelled = true;
        }
    }
}

fn cfg_cancel(cancel_latency_ns: u64) -> LagConfig {
    LagConfig {
        order_latency_ns: 0,
        cancel_latency_ns,
        exec_book_levels: 20,
        fees: FeeSchedule::legacy(),
    }
}

/// Req 6.7 - CANCEL-LATENCY WINDOW: an order pulled by the strategy is STILL
/// fillable while the cancel is in flight. A through-trade that arrives BEFORE the
/// cancel lands fills the resting order at its price.
///
/// Timeline (ns): place decided @2000 -> rests at the top of the @3000 step; the
/// strategy emits Cancel @3000, which travels on cancel_latency_ns = 5000 and so
/// LANDS at 3000 + 5000 = 8000. The through-trade is at local_ts = 4000, which is
/// < 8000 -> the cancel is still in flight -> `drain_pending(4000)` does not yet
/// remove the order -> it is resting and the Ask-aggressor at 99.5 (<=100) fills it.
#[test]
fn cancel_in_flight_window_still_fills_from_a_through_trade() {
    let evs = [
        book_delta(Side::Bid, 99.0, 1000.0, 1_000),
        book_delta(Side::Ask, 103.0, 1000.0, 2_000), // place bid@100 decided here (arrival 2000)
        book_delta(Side::Bid, 99.0, 1000.0, 3_000),  // place rests at top; Cancel decided (lands 8000)
        hl_trade(Side::Ask, 99.5, 5.0, 4_000),       // 4000 < 8000 -> in flight -> fills
    ];
    let strat = PlaceThenCancel {
        id: 7,
        side: Side::Bid,
        px: 100.0,
        qty: 5.0,
        placed: false,
        cancelled: false,
    };
    let mut eng = LagEngine::new(strat, cfg_cancel(5_000));
    eng.run(evs.iter()).unwrap();
    let r = eng.finish();

    assert_eq!(r.orders_rested, 1, "the maker bid rested before the cancel was issued");
    assert_eq!(
        r.maker_fills, 1,
        "a through-trade inside the cancel-latency window STILL fills the resting order"
    );
    let five = Qty::from_f64(5.0).unwrap().raw();
    assert_eq!(r.final_position, five, "filled long 5 while the cancel was still in flight");
}

/// Req 6.8 - CANCEL TAKES EFFECT AFTER THE WINDOW: the same place/cancel setup, but
/// the through-trade arrives AFTER the cancel has landed, so the order is gone and
/// does NOT fill.
///
/// Timeline (ns): identical place @2000 / rests + Cancel decided @3000 (lands at
/// 3000 + 5000 = 8000). The through-trade is at local_ts = 9000 > 8000, so at the
/// TOP of that step `drain_pending(9000)` pops the cancel first (8000 <= 9000) and
/// removes the resting order; `process_trade` then finds nothing to fill.
#[test]
fn cancel_after_window_removes_the_order_and_it_does_not_fill() {
    let evs = [
        book_delta(Side::Bid, 99.0, 1000.0, 1_000),
        book_delta(Side::Ask, 103.0, 1000.0, 2_000), // place bid@100 decided here (arrival 2000)
        book_delta(Side::Bid, 99.0, 1000.0, 3_000),  // place rests at top; Cancel decided (lands 8000)
        hl_trade(Side::Ask, 99.5, 5.0, 9_000),       // 9000 > 8000 -> cancel landed first -> no fill
    ];
    let strat = PlaceThenCancel {
        id: 7,
        side: Side::Bid,
        px: 100.0,
        qty: 5.0,
        placed: false,
        cancelled: false,
    };
    let mut eng = LagEngine::new(strat, cfg_cancel(5_000));
    eng.run(evs.iter()).unwrap();
    let r = eng.finish();

    assert_eq!(r.orders_rested, 1, "the maker bid rested before the cancel was issued");
    assert_eq!(
        r.maker_fills, 0,
        "the cancel landed (8000) before the trade (9000) -> the order was removed, no fill"
    );
    assert_eq!(r.final_position, 0, "we stay flat: the pull took effect outside the window");
}
