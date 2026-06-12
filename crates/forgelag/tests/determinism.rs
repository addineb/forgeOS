//! Determinism + no-lookahead verification (spec lagshot-maker, Task 10).
//!
//! Integration tests that PROVE the forgelag engine's headline honesty
//! guarantees, driving the real `MakerQuoter` over the real `LagEngine`:
//!
//!   * REPLAY DETERMINISM (Req 9.5 / 6.9): the SAME scripted event stream replayed
//!     TWICE yields byte-for-byte identical `LagReport`s across EVERY field
//!     (counts, `net_pnl` via `to_bits()`, `max_drawdown_pct` bits, and the full
//!     per-trip return vector). The stream is rich enough to produce several maker
//!     fills + round trips so the comparison is meaningful.
//!   * NON-MONOTONIC TIMESTAMP HALTS (Req 9.1 / 9.2): an event whose `local_ts` is
//!     less than the previous event's makes `step` return `Err` (no report).
//!   * LATENCY DELIVERY (Req 9.6): with order latency L, an order submitted at T is
//!     NOT eligible for matching before T+L - a through-trade at T+L-1 does not
//!     fill it, the same trade at T+L does.
//!   * SAMPLED LATENCY >= 0 (Req 9.7): the latency samples are `u64`, so an order's
//!     delivery time `arrival = t + lat` is always `>= t` (never a past delivery);
//!     verified through the sampled-latency path at the timing boundary.
//!   * NO-LOOKAHEAD (Req 9.3 / 9.4): a strategy decision at `now` sees only data
//!     with `ts <= now`. A reference trade that arrives in a LATER event neither
//!     appears in an earlier `LagCtx` nor changes an earlier-emitted quote.

use std::cell::RefCell;
use std::rc::Rc;

use forge_core::{Price, Qty, Side};
use forgelag::{
    FairValueConfig, FairValueOracle, FeeSchedule, InventoryConfig, InventoryController, LagConfig,
    LagCtx, LagEngine, LagEvent, LagKind, LagOrder, LagOrderKind, LagReport, LagStrategy,
    MakerQuoter, MakerQuoterConfig, PlaceMode, QuoteConfig, Role, DEFAULT_STALENESS_NS,
};

// --------------------------------------------------------------------------
// Shared scripted-stream + strategy helpers (mirroring tests/fill_model.rs and
// the maker_quoter end-to-end tests in src/maker.rs).
// --------------------------------------------------------------------------

/// Build one multi-venue event at `ts` (exch_ts == local_ts; no feed latency).
fn lev(role: Role, kind: LagKind, side: Side, px: f64, qty: f64, ts: u64) -> LagEvent {
    LagEvent {
        role,
        kind,
        exch_ts: ts,
        local_ts: ts,
        side: Some(side),
        price: Price::from_f64(px).unwrap(),
        qty: Qty::from_f64(qty).unwrap(),
        src: 0,
        aux: 0.0,
    }
}

fn legacy_cfg(order_latency_ns: u64) -> LagConfig {
    LagConfig {
        order_latency_ns,
        cancel_latency_ns: 0,
        exec_book_levels: 20,
        fees: FeeSchedule::legacy(),
    }
}

/// The proven end-to-end `MakerQuoter` setup from `src/maker.rs`: rests the entry
/// ask well above the (rich) book so it is non-marketable, 16bps entry / 2bps
/// revert exit, qty 0.1, pos cap 10. Deterministic by construction.
fn maker_strategy() -> MakerQuoter {
    let oracle = FairValueOracle::new(FairValueConfig {
        top_n: 5,
        window: 500,
        sample_ns: 1_000_000,
        staleness_ns: DEFAULT_STALENESS_NS,
    });
    let qty = Qty::from_f64(0.1).unwrap();
    let cfg = MakerQuoterConfig {
        quote: QuoteConfig {
            quote_offset_bps: 25.0,
            entry_threshold_bps: 16.0,
            exit_bps: 2.0,
            reprice_tol_bps: 1.0,
            danger_bps: 40.0,
            cancel_latency_ns: 0,
            ack_timeout_ns: 0,
            quote_qty: qty,
            place_mode: PlaceMode::Fade,
        },
        maker_exit: false,
        hold_ns: 0,
    };
    let inv = InventoryController::new(InventoryConfig {
        pos_cap: Qty::from_f64(10.0).unwrap().raw(),
        inv_skew_bps: 0.0,
        quote_qty: qty.raw(),
    });
    MakerQuoter::new(cfg, oracle, inv)
}

/// A synthetic stream rich enough to produce SEVERAL maker fills + round trips so
/// the determinism comparison is meaningful. The HL book is FIXED (bid 999.9 /
/// ask 1000.1, micro ~1000); each cycle creates a dislocation by dropping the OKX
/// REFERENCE price to 998 (HL rich ~20bps => rest an Ask above fair value), prints
/// an HL buy THROUGH the resting ask (maker fill => short), then reverts the ref to
/// 1000 (dev ~0 => taker revert-to-mean exit => flat). Four cycles => ~4 round trips.
fn rich_round_trip_stream() -> Vec<LagEvent> {
    let size = 100.0;
    let mut evs = Vec::new();
    let mut ts = 1_000_000_000u64;
    let step = 1_000_000u64; // 1 ms
    evs.push(lev(Role::Exec, LagKind::BookDelta, Side::Bid, 999.9, size, ts));
    evs.push(lev(Role::Exec, LagKind::BookDelta, Side::Ask, 1000.1, size, ts));
    // Warm-up: ref at 1000 for > MIN_DEV_SAMPLES samples (baseline ~0).
    for _ in 0..45 {
        ts += step;
        evs.push(lev(Role::Reference, LagKind::Trade, Side::Bid, 1000.0, size, ts));
    }
    for _ in 0..4 {
        // Dislocation: ref drops to 998 => HL rich ~20bps => rest an Ask.
        ts += step;
        evs.push(lev(Role::Reference, LagKind::Trade, Side::Bid, 998.0, size, ts));
        // Through-trade: an HL BUY printing above the resting ask => maker fill (short).
        ts += step;
        evs.push(lev(Role::Exec, LagKind::Trade, Side::Bid, 1001.0, 50.0, ts));
        // Hold one beat, still dislocated.
        ts += step;
        evs.push(lev(Role::Reference, LagKind::Trade, Side::Bid, 998.0, size, ts));
        // Reversion: ref back to 1000 => dev ~0 => taker revert-to-mean exit.
        ts += step;
        evs.push(lev(Role::Reference, LagKind::Trade, Side::Bid, 1000.0, size, ts));
        // Trailing beats so the exit market drains + the oracle re-baselines.
        for _ in 0..8 {
            ts += step;
            evs.push(lev(Role::Reference, LagKind::Trade, Side::Bid, 1000.0, size, ts));
        }
    }
    evs
}

fn run_maker(evs: &[LagEvent]) -> LagReport {
    let mut eng = LagEngine::new(maker_strategy(), legacy_cfg(0));
    eng.run(evs.iter()).unwrap();
    eng.finish()
}

/// Assert two reports are byte-for-byte identical across EVERY field (Req 9.5).
fn assert_reports_identical(a: &LagReport, b: &LagReport) {
    assert_eq!(a.events, b.events, "events");
    assert_eq!(a.orders_submitted, b.orders_submitted, "orders_submitted");
    assert_eq!(a.orders_filled, b.orders_filled, "orders_filled");
    assert_eq!(a.maker_fills, b.maker_fills, "maker_fills");
    assert_eq!(a.orders_rested, b.orders_rested, "orders_rested");
    assert_eq!(a.orders_rejected, b.orders_rejected, "orders_rejected");
    assert_eq!(a.quotes_submitted, b.quotes_submitted, "quotes_submitted");
    assert_eq!(a.quotes_filled, b.quotes_filled, "quotes_filled");
    assert_eq!(a.round_trips, b.round_trips, "round_trips");
    assert_eq!(a.final_position, b.final_position, "final_position");
    assert_eq!(a.max_abs_inventory, b.max_abs_inventory, "max_abs_inventory");
    assert_eq!(a.net_pnl.to_bits(), b.net_pnl.to_bits(), "net_pnl bit-identical (Req 9.5)");
    assert_eq!(
        a.max_drawdown_pct.to_bits(),
        b.max_drawdown_pct.to_bits(),
        "max_drawdown_pct bit-identical (Req 9.5)"
    );
    assert_eq!(a.trip_returns.len(), b.trip_returns.len(), "trip_returns len");
    for (i, (x, y)) in a.trip_returns.iter().zip(b.trip_returns.iter()).enumerate() {
        assert_eq!(x.0, y.0, "trip_returns[{i}] close_ts");
        assert_eq!(x.1.to_bits(), y.1.to_bits(), "trip_returns[{i}] return bit-identical");
    }
    assert_eq!(a.trip_notionals.len(), b.trip_notionals.len(), "trip_notionals len");
    for (i, (x, y)) in a.trip_notionals.iter().zip(b.trip_notionals.iter()).enumerate() {
        assert_eq!(x.to_bits(), y.to_bits(), "trip_notionals[{i}] bit-identical");
    }
}

// --------------------------------------------------------------------------
// Req 9.5 / 6.9: replay determinism (byte-identical report across runs).
// --------------------------------------------------------------------------

#[test]
fn replay_is_byte_identical_across_two_runs() {
    let evs = rich_round_trip_stream();
    let a = run_maker(&evs);
    let b = run_maker(&evs);

    // The stream must actually DO something or the comparison is vacuous.
    assert!(a.maker_fills >= 1, "stream must produce >=1 maker fill; got {}", a.maker_fills);
    assert!(a.round_trips >= 1, "stream must produce >=1 round trip; got {}", a.round_trips);
    eprintln!(
        "[determinism] events={} submitted={} filled={} maker_fills={} quotes_sub={} quotes_fill={} trips={} net={:.6}",
        a.events,
        a.orders_submitted,
        a.orders_filled,
        a.maker_fills,
        a.quotes_submitted,
        a.quotes_filled,
        a.round_trips,
        a.net_pnl
    );
    assert_reports_identical(&a, &b);
}

// --------------------------------------------------------------------------
// Req 9.1 / 9.2: a non-monotonic local_ts halts the sim with an error (no report).
// --------------------------------------------------------------------------

/// Emits nothing - we only care that `step` surfaces the timestamp error.
struct Noop;
impl LagStrategy for Noop {
    fn on_event(&mut self, _ctx: &LagCtx, _out: &mut Vec<LagOrder>) {}
}

#[test]
fn non_monotonic_local_ts_halts_with_error() {
    let mut eng = LagEngine::new(Noop, legacy_cfg(0));
    // First event at ts=5000 is accepted; the clock advances to 5000.
    let e1 = lev(Role::Reference, LagKind::Trade, Side::Bid, 1000.0, 1.0, 5_000);
    assert!(eng.step(&e1).is_ok(), "first event accepted");
    // A later event presenting an EARLIER ts (4000 < 5000) must halt (Req 9.2).
    let e2 = lev(Role::Reference, LagKind::Trade, Side::Bid, 1000.0, 1.0, 4_000);
    let res = eng.step(&e2);
    assert!(res.is_err(), "non-monotonic local_ts must return Err (Req 9.2)");
    let msg = res.unwrap_err();
    assert!(
        msg.contains("non-monotonic"),
        "the error must name the non-monotonic timestamp, got: {msg}"
    );
}

// --------------------------------------------------------------------------
// Req 9.6 / 9.7: latency delivery boundary + non-negative sampled latency.
// --------------------------------------------------------------------------

/// Rests exactly ONE maker bid the first time both book sides exist; never
/// cancels (so the report reflects whether the through-trade reached it).
struct PlaceOnceBid {
    placed: bool,
    px: f64,
    qty: f64,
}
impl LagStrategy for PlaceOnceBid {
    fn on_event(&mut self, ctx: &LagCtx, out: &mut Vec<LagOrder>) {
        if !self.placed && ctx.exec_book.best_bid().is_some() && ctx.exec_book.best_ask().is_some() {
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

/// Run a place-then-through-trade scenario: a maker bid@100 is decided at T=2000
/// (when the ask arrives), and an Ask-aggressor trade prints through it at
/// `trade_ts`. `sampled` chooses the fixed vs the empirical-distribution latency
/// path; the applied latency is always `lat_ns`.
fn run_latency_case(lat_ns: u64, trade_ts: u64, sampled: bool) -> LagReport {
    let evs = [
        lev(Role::Exec, LagKind::BookDelta, Side::Bid, 99.0, 1000.0, 1_000),
        lev(Role::Exec, LagKind::BookDelta, Side::Ask, 103.0, 1000.0, 2_000), // place decided here (T)
        lev(Role::Exec, LagKind::Trade, Side::Ask, 99.5, 5.0, trade_ts),      // prints through bid@100
    ];
    let strat = PlaceOnceBid { placed: false, px: 100.0, qty: 5.0 };
    let mut eng = if sampled {
        let mut e = LagEngine::new(strat, legacy_cfg(0));
        // A single-element distribution makes every sampled latency exactly lat_ns.
        e.set_latency_samples(vec![lat_ns]);
        e
    } else {
        LagEngine::new(strat, legacy_cfg(lat_ns))
    };
    eng.run(evs.iter()).unwrap();
    eng.finish()
}

#[test]
fn order_not_eligible_before_arrival_then_fills_at_arrival() {
    // L = 5000ns, submitted at T = 2000 => arrival T+L = 7000.
    let l = 5_000u64;
    // A through-trade ONE ns BEFORE arrival (6999 < 7000): the order has not yet
    // reached the matcher, so it cannot fill (Req 9.6).
    let before = run_latency_case(l, 6_999, false);
    assert_eq!(before.maker_fills, 0, "no fill before T+L (order not delivered yet, Req 9.6)");
    assert_eq!(before.final_position, 0, "still flat: the order had not arrived");
    // The SAME trade exactly AT arrival (7000 >= 7000): the order is now resting
    // and the through-trade fills it.
    let at = run_latency_case(l, 7_000, false);
    assert_eq!(at.maker_fills, 1, "fills at T+L once delivered (Req 9.6)");
    let five = Qty::from_f64(5.0).unwrap().raw();
    assert_eq!(at.final_position, five, "filled long 5 at the arrival boundary");
}

#[test]
fn sampled_latency_is_nonnegative_and_delivery_not_before_submission() {
    // Req 9.7: sampled latencies are `u64`, so for an order submitted at `t` the
    // delivery time `arrival = t + lat` satisfies `arrival >= t` BY TYPE (a u64
    // latency can never be negative => never a past delivery). We verify this
    // through the sampled-latency path at the timing boundary: with a sampled
    // latency of exactly L the order is not eligible before T+L and is eligible
    // at T+L - identical to the fixed case, confirming delivery >= submission.
    let l = 5_000u64;
    let before = run_latency_case(l, 6_999, true);
    assert_eq!(before.maker_fills, 0, "sampled L: not eligible before T+L (delivery >= submission)");
    let at = run_latency_case(l, 7_000, true);
    assert_eq!(at.maker_fills, 1, "sampled L: eligible at T+L");
}

// --------------------------------------------------------------------------
// Req 9.3 / 9.4: no-lookahead. A strategy decision at `now` sees only `ts <= now`.
// --------------------------------------------------------------------------

/// Records the `(now, ref_px)` it is handed on each event, via shared interior
/// mutability so the log can be read after the engine has run.
struct RecordRef {
    log: Rc<RefCell<Vec<(u64, f64)>>>,
}
impl LagStrategy for RecordRef {
    fn on_event(&mut self, ctx: &LagCtx, _out: &mut Vec<LagOrder>) {
        self.log.borrow_mut().push((ctx.now, ctx.ref_px));
    }
}

#[test]
fn ctx_only_exposes_reference_data_at_or_before_now() {
    // Refs sit at 1000 through ts=3000, then a LATE ref CRASHES to 500 at ts=10000.
    // No decision taken at or before 3000 may ever see the 500 that arrives later.
    let evs = [
        lev(Role::Reference, LagKind::Trade, Side::Bid, 1000.0, 1.0, 1_000),
        lev(Role::Reference, LagKind::Trade, Side::Bid, 1000.0, 1.0, 2_000),
        lev(Role::Reference, LagKind::Trade, Side::Bid, 1000.0, 1.0, 3_000),
        lev(Role::Reference, LagKind::Trade, Side::Bid, 500.0, 1.0, 10_000),
        lev(Role::Reference, LagKind::Trade, Side::Bid, 500.0, 1.0, 11_000),
    ];
    let log = Rc::new(RefCell::new(Vec::new()));
    let strat = RecordRef { log: Rc::clone(&log) };
    let mut eng = LagEngine::new(strat, legacy_cfg(0));
    eng.run(evs.iter()).unwrap();

    let seen = log.borrow();
    for &(now, ref_px) in seen.iter() {
        if now < 10_000 {
            assert!(
                (ref_px - 1000.0).abs() < 1e-9,
                "at now={now} the strategy saw ref_px={ref_px}; the LATE 500 (ts=10000) must NOT be visible (Req 9.3/9.4)"
            );
        } else {
            assert!(
                (ref_px - 500.0).abs() < 1e-9,
                "at now={now} (>= the late event) ref_px should reflect the 500 just delivered, got {ref_px}"
            );
        }
    }
}

/// Wraps any strategy and records every `(now, order)` it emits, so the orders a
/// strategy produced can be inspected after the run.
struct Spy<S: LagStrategy> {
    inner: S,
    log: Rc<RefCell<Vec<(u64, LagOrder)>>>,
}
impl<S: LagStrategy> LagStrategy for Spy<S> {
    fn on_event(&mut self, ctx: &LagCtx, out: &mut Vec<LagOrder>) {
        let before = out.len();
        self.inner.on_event(ctx, out);
        let mut log = self.log.borrow_mut();
        for o in &out[before..] {
            log.push((ctx.now, *o));
        }
    }
}

/// Common prefix that warms the oracle and then dislocates (ref 998) at the
/// returned `t_disloc`, where the MakerQuoter emits its entry quote.
fn prefix_until_dislocation() -> (Vec<LagEvent>, u64) {
    let size = 100.0;
    let mut evs = Vec::new();
    let mut ts = 1_000_000_000u64;
    let step = 1_000_000u64;
    evs.push(lev(Role::Exec, LagKind::BookDelta, Side::Bid, 999.9, size, ts));
    evs.push(lev(Role::Exec, LagKind::BookDelta, Side::Ask, 1000.1, size, ts));
    for _ in 0..45 {
        ts += step;
        evs.push(lev(Role::Reference, LagKind::Trade, Side::Bid, 1000.0, size, ts));
    }
    ts += step;
    let t_disloc = ts;
    evs.push(lev(Role::Reference, LagKind::Trade, Side::Bid, 998.0, size, ts));
    (evs, t_disloc)
}

fn run_spy(evs: &[LagEvent]) -> Vec<(u64, LagOrder)> {
    let log = Rc::new(RefCell::new(Vec::new()));
    {
        // Scope the engine so its `Rc` clone of `log` is released before we take
        // sole ownership below (the engine owns the strategy, hence a clone).
        let strat = Spy { inner: maker_strategy(), log: Rc::clone(&log) };
        let mut eng = LagEngine::new(strat, legacy_cfg(0));
        eng.run(evs.iter()).unwrap();
    }
    Rc::try_unwrap(log).expect("sole owner of the spy log").into_inner()
}

#[test]
fn a_late_reference_does_not_change_an_earlier_quote() {
    let (mut base, t_disloc) = prefix_until_dislocation();
    let step = 1_000_000u64;

    // Stream A: after the dislocation the ref simply reverts toward 1000.
    let mut a = base.clone();
    let mut ts = t_disloc;
    for _ in 0..4 {
        ts += step;
        a.push(lev(Role::Reference, LagKind::Trade, Side::Bid, 1000.0, 100.0, ts));
    }

    // Stream B: identical up to the dislocation, but a LATE ref CRASH to 500
    // arrives AFTER the entry quote was already emitted at t_disloc.
    let mut ts = t_disloc;
    for _ in 0..4 {
        ts += step;
        base.push(lev(Role::Reference, LagKind::Trade, Side::Bid, 500.0, 100.0, ts));
    }
    let b = base;

    let orders_a = run_spy(&a);
    let orders_b = run_spy(&b);

    // The first emitted order is the entry quote, placed AT the dislocation event
    // (t_disloc) - strictly before any of the diverging later events. It must be
    // byte-identical between the two streams: the later 500-crash in B cannot
    // reach back and change a decision already taken at t_disloc (Req 9.4).
    let first_a = orders_a.first().expect("stream A emits an entry quote");
    let first_b = orders_b.first().expect("stream B emits an entry quote");
    assert_eq!(first_a.0, t_disloc, "the entry quote is emitted at the dislocation event");
    assert_eq!(first_a.0, first_b.0, "same emission timestamp");
    assert_eq!(first_a.1.kind, LagOrderKind::Place, "the first order is the maker entry quote");
    assert_eq!(first_b.1.kind, LagOrderKind::Place);
    assert_eq!(first_a.1.side, first_b.1.side, "same side - unaffected by the later ref");
    assert_eq!(first_a.1.id, first_b.1.id, "same client id");
    assert_eq!(
        first_a.1.price.raw(),
        first_b.1.price.raw(),
        "the earlier quote PRICE is identical: a later ref cannot change it (Req 9.4)"
    );
}