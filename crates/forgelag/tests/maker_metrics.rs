//! MAKER METRICS + the percent reporter (spec lagshot-maker, Task 9).
//!
//! These prove the new `LagReport` maker fields and `LagReport::summary_pct`
//! reporter compute the right numbers under fully scripted, deterministic runs
//! (0 latency, seeded books + HL trades), reusing the scripted-strategy pattern
//! from `tests/fill_model.rs`:
//!   * `quotes_submitted` counts maker `Place` actions, NOT `Market`/taker orders,
//!   * `max_abs_inventory` is the largest |position| reached in the run,
//!   * the fill rate is `quotes_filled` / `quotes_submitted` * 100 (0 when none
//!     submitted), counting a distinct quote once even across partials,
//!   * `max_drawdown_pct` is the peak-to-trough decline of the realized-equity
//!     curve (EUR500 + realized P&L) as a percent of the preceding peak, and
//!   * net expectancy is 0% when there are zero completed round trips.
//!
//! Requirements: 4.5, 10.1, 10.2, 10.3, 10.4, 10.5, 10.6, 10.7, 10.8.

use forge_core::{Price, Qty, Side};
use forgelag::{
    FeeSchedule, LagConfig, LagCtx, LagEngine, LagEvent, LagKind, LagOrder, LagStrategy, Role,
    DEFAULT_STARTING_EQUITY_EUR,
};

fn cfg(fees: FeeSchedule) -> LagConfig {
    LagConfig { order_latency_ns: 0, cancel_latency_ns: 0, exec_book_levels: 20, fees }
}

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

/// Rests one maker BID, then once long closes it with a taker market SELL. One
/// completed round trip with exactly ONE `Place` (the maker entry) and ONE
/// `Market` (the taker exit).
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

/// Places a fixed script of maker quotes (all on the first event where both book
/// sides exist), then never acts again. Lets us submit N quotes and let HL trades
/// fill a known subset M.
struct PlaceScript {
    quotes: Vec<(u64, Side, f64, f64)>,
    placed: bool,
}
impl LagStrategy for PlaceScript {
    fn on_event(&mut self, ctx: &LagCtx, out: &mut Vec<LagOrder>) {
        if !self.placed
            && ctx.exec_book.best_bid().is_some()
            && ctx.exec_book.best_ask().is_some()
        {
            for &(id, side, px, qty) in &self.quotes {
                out.push(LagOrder::place(
                    id,
                    side,
                    Price::from_f64(px).unwrap(),
                    Qty::from_f64(qty).unwrap(),
                ));
            }
            self.placed = true;
        }
    }
}

/// Never submits any order. A run with zero quotes (fill rate must be 0%).
struct Noop;
impl LagStrategy for Noop {
    fn on_event(&mut self, _ctx: &LagCtx, _out: &mut Vec<LagOrder>) {}
}

/// Two sequential maker-entry + taker-exit round trips with controlled fill
/// prices, used to script a peak-then-trough realized-equity curve. Trip 1 is a
/// win (long @e1, exit higher); trip 2 is a loss (long @e2, exit lower). Acts on
/// the real `position_qty` (exchange truth), never assumes a fill.
struct TwoTrips {
    e1: f64,
    e2: f64,
    qty: f64,
    placed1: bool,
    exited1: bool,
    placed2: bool,
    exited2: bool,
}
impl LagStrategy for TwoTrips {
    fn on_event(&mut self, ctx: &LagCtx, out: &mut Vec<LagOrder>) {
        let pos = ctx.position_qty;
        let both = ctx.exec_book.best_bid().is_some() && ctx.exec_book.best_ask().is_some();
        if !self.exited1 {
            if !self.placed1 {
                if both {
                    out.push(LagOrder::place(
                        1,
                        Side::Bid,
                        Price::from_f64(self.e1).unwrap(),
                        Qty::from_f64(self.qty).unwrap(),
                    ));
                    self.placed1 = true;
                }
            } else if pos > 0 {
                out.push(LagOrder::market(Side::Ask, Qty::from_raw(pos)));
                self.exited1 = true;
            }
        } else if !self.exited2 {
            if !self.placed2 {
                if pos == 0 {
                    out.push(LagOrder::place(
                        2,
                        Side::Bid,
                        Price::from_f64(self.e2).unwrap(),
                        Qty::from_f64(self.qty).unwrap(),
                    ));
                    self.placed2 = true;
                }
            } else if pos > 0 {
                out.push(LagOrder::market(Side::Ask, Qty::from_raw(pos)));
                self.exited2 = true;
            }
        }
    }
}

/// Req 10.2 numerator scope / Req 1.1: `quotes_submitted` counts maker `Place`
/// actions only - the taker `Market` exit does NOT count toward it.
#[test]
fn quotes_submitted_counts_place_not_market() {
    let evs = [
        book_delta(Side::Bid, 99.0, 1000.0, 1_000),
        book_delta(Side::Ask, 103.0, 1000.0, 2_000), // place bid@100 (arrival 2000)
        book_delta(Side::Ask, 103.0, 1000.0, 3_000), // rests
        hl_trade(Side::Ask, 99.5, 5.0, 4_000),       // fills the maker bid -> long
        book_delta(Side::Ask, 103.0, 1000.0, 5_000), // taker market exit drains here
    ];
    let mut eng = LagEngine::new(MakerThenExit { placed: false, px: 100.0, qty: 5.0 }, cfg(FeeSchedule::legacy()));
    eng.run(evs.iter()).unwrap();
    let r = eng.finish();

    assert_eq!(r.quotes_submitted, 1, "exactly one maker Place quote was submitted");
    assert_eq!(r.orders_submitted, 2, "one maker Place + one taker Market = two orders");
    assert_eq!(r.maker_fills, 1, "the maker entry filled once");
    assert_eq!(r.round_trips, 1, "entry + exit = one round trip");
}

/// Req 4.5 / 4.6: `max_abs_inventory` equals the largest |position| reached in a
/// scripted run (here +7 base units), even after we flatten back to zero.
#[test]
fn max_abs_inventory_tracks_largest_position() {
    let evs = [
        book_delta(Side::Bid, 99.0, 1000.0, 1_000),
        book_delta(Side::Ask, 103.0, 1000.0, 2_000),
        book_delta(Side::Ask, 103.0, 1000.0, 3_000),
        hl_trade(Side::Ask, 99.5, 10.0, 4_000),       // fills 7 -> peak long 7
        book_delta(Side::Ask, 103.0, 1000.0, 5_000),  // taker exit -> back to flat
    ];
    let mut eng = LagEngine::new(MakerThenExit { placed: false, px: 100.0, qty: 7.0 }, cfg(FeeSchedule::legacy()));
    eng.run(evs.iter()).unwrap();
    let r = eng.finish();

    let seven = Qty::from_f64(7.0).unwrap().raw();
    assert_eq!(r.max_abs_inventory, seven, "the peak absolute inventory was 7 base units");
    assert_eq!(r.final_position, 0, "the run ends flat after the taker exit");
}

/// Req 10.2 / 10.3: fill rate = distinct filled quotes / submitted quotes * 100.
/// Submit 4 maker bids, let one HL trade print through exactly 2 of them -> 50%.
#[test]
fn fill_rate_counts_distinct_quotes_over_submitted() {
    let evs = [
        book_delta(Side::Bid, 80.0, 1000.0, 1_000),
        book_delta(Side::Ask, 1000.0, 1000.0, 2_000), // 4 quotes placed (arrival 2000)
        book_delta(Side::Ask, 1000.0, 1000.0, 3_000), // all 4 rest
        hl_trade(Side::Ask, 97.0, 100.0, 4_000),      // through-fills bids @100 and @98 only
    ];
    let quotes = vec![
        (1u64, Side::Bid, 100.0, 1.0),
        (2u64, Side::Bid, 98.0, 1.0),
        (3u64, Side::Bid, 90.0, 1.0),
        (4u64, Side::Bid, 88.0, 1.0),
    ];
    let mut eng = LagEngine::new(PlaceScript { quotes, placed: false }, cfg(FeeSchedule::legacy()));
    eng.run(evs.iter()).unwrap();
    let r = eng.finish();

    assert_eq!(r.quotes_submitted, 4, "four maker quotes submitted");
    assert_eq!(r.quotes_filled, 2, "the trade printed through exactly two of them");
    let s = r.summary_pct(DEFAULT_STARTING_EQUITY_EUR);
    assert!((s.fill_rate_pct - 50.0).abs() < 1e-9, "2 / 4 * 100 = 50.00%, got {}", s.fill_rate_pct);
}

/// Req 10.3: with zero quotes submitted the fill rate is 0% (no divide-by-zero),
/// and Req 10.8: zero round trips -> net expectancy 0%.
#[test]
fn fill_rate_and_expectancy_zero_when_nothing_submitted() {
    let evs = [
        book_delta(Side::Bid, 99.0, 1000.0, 1_000),
        book_delta(Side::Ask, 103.0, 1000.0, 2_000),
        hl_trade(Side::Ask, 99.5, 5.0, 3_000),
    ];
    let mut eng = LagEngine::new(Noop, cfg(FeeSchedule::legacy()));
    eng.run(evs.iter()).unwrap();
    let r = eng.finish();

    assert_eq!(r.quotes_submitted, 0, "the no-op strategy submits nothing");
    assert_eq!(r.round_trips, 0, "no round trips");
    let s = r.summary_pct(DEFAULT_STARTING_EQUITY_EUR);
    assert!((s.fill_rate_pct - 0.0).abs() < 1e-9, "0 submitted -> 0% fill rate");
    assert!((s.net_expectancy_pct - 0.0).abs() < 1e-9, "0 round trips -> 0% expectancy");
}

/// Req 10.5 / 10.6: max drawdown = peak-to-trough decline of the realized-equity
/// curve (EUR500 + realized P&L) as a percent of the preceding peak. Scripted:
/// trip 1 wins +10 (long 1 @100, exit @110) lifting equity 500 -> 510 (peak);
/// trip 2 loses -15 (long 1 @105, exit @90) dropping equity 510 -> 495 (trough).
/// Drawdown = (510 - 495) / 510 * 100 = 2.9412% (with zero fees for predictability).
#[test]
fn max_drawdown_pct_peak_then_trough() {
    let evs = [
        book_delta(Side::Ask, 1000.0, 1000.0, 1_000),
        book_delta(Side::Bid, 90.0, 1000.0, 2_000),  // place trip-1 entry bid@100
        book_delta(Side::Bid, 110.0, 1000.0, 3_000), // entry1 rests; book bid 110 (exit-1 price)
        hl_trade(Side::Ask, 99.5, 5.0, 4_000),        // fills entry1 @100 -> long 1; emit exit1
        book_delta(Side::Ask, 1000.0, 1000.0, 5_000), // exit1 market sells @110 -> +10; place entry2 @105
        book_delta(Side::Bid, 110.0, 0.0, 6_000),     // entry2 rests; remove the 110 bid (best bid -> 90)
        hl_trade(Side::Ask, 104.0, 5.0, 7_000),       // fills entry2 @105 -> long 1; emit exit2
        book_delta(Side::Ask, 1000.0, 1000.0, 8_000), // exit2 market sells @90 -> -15
    ];
    let strat = TwoTrips {
        e1: 100.0,
        e2: 105.0,
        qty: 1.0,
        placed1: false,
        exited1: false,
        placed2: false,
        exited2: false,
    };
    let mut eng = LagEngine::new(strat, cfg(FeeSchedule::zero()));
    eng.run(evs.iter()).unwrap();
    let r = eng.finish();

    assert_eq!(r.round_trips, 2, "two completed round trips");
    assert!((r.net_pnl - (-5.0)).abs() < 1e-6, "realized P&L = +10 - 15 = -5, got {}", r.net_pnl);
    let expected = 15.0 / 510.0 * 100.0; // 2.94117...
    assert!(
        (r.max_drawdown_pct - expected).abs() < 1e-6,
        "drawdown (510 -> 495) / 510 = {expected}%, got {}",
        r.max_drawdown_pct
    );

    let s = r.summary_pct(DEFAULT_STARTING_EQUITY_EUR);
    assert!((s.max_drawdown_pct - 2.94).abs() < 1e-9, "rounded to 2dp = 2.94%, got {}", s.max_drawdown_pct);
    // Cross-check the percent reporter: mean of (+10%, -14.2857%) = -2.14% (2dp).
    assert!((s.net_expectancy_pct - (-2.14)).abs() < 1e-9, "net expectancy mean = -2.14%, got {}", s.net_expectancy_pct);
}