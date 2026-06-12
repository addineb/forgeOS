//! NULL-EDGE MAKER GATE (spec lagshot-maker, Task 3 / Req 7) - the single most
//! important check in the whole maker pivot.
//!
//! A seeded RANDOM-SIDE maker has NO edge: it rests a maker limit on a coinflip
//! side at an offset from fair value (never crossing), gets filled only by HL
//! trades that print THROUGH the resting price (adverse selection, biased to
//! wrong-way flow), then exits with a taker market order. Once the honest fill
//! model is in force and fees are paid (maker on entry + taker on exit), such a
//! control MUST net negative over many round trips. If it nets >= 0 the engine
//! is MANUFACTURING edge and the gate FAILS (the assertions below flip red),
//! blocking any maker promotion.
//!
//! Two streams: a SYNTHETIC stream (a two-sided random-walk book PLUS random
//! two-sided HL exec trades that print through resting quotes so makers can
//! actually fill - the existing taker synth only emits Reference trades + Exec
//! book deltas, which never fill a resting maker) and, when the fresh tick tree
//! is present on the box, one REAL ETH day (HL book+trades vs OKX-spot ref).
//! The synthetic stream is the mandatory gate; the real day runs when its data
//! is readily loadable and is otherwise clearly deferred.
//!
//! Determinism (Req 7.5): identical seed + identical stream -> identical net.
//!
//! Requirements: 7.1, 7.2, 7.3, 7.4, 7.5.

use std::path::{Path, PathBuf};

use forge_core::{Price, Qty, Side};
use forgelag::{
    load_window, FeeSchedule, FeedConfig, LagConfig, LagCtx, LagEngine, LagEvent, LagKind, LagOrder,
    LagStrategy, Role,
};

/// Configurable minimum completed round trips before the gate evaluates a
/// verdict (Req 7.2). 1,000 per the spec.
const MIN_ROUND_TRIPS: u64 = 1_000;

/// Seeded xorshift (same family as the taker null-edge control); the ONLY source
/// of randomness in the control, so the gate is reproducible (Req 7.5).
struct Rng(u64);
impl Rng {
    fn bit(&mut self) -> bool {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) & 1 == 1
    }
}

/// The RANDOM-SIDE MAKER null control (lives in the test file on purpose: it is a
/// throwaway coinflip control that belongs with its gate; the real `MakerQuoter`
/// arrives in Task 7 and keeps `strategy.rs` clean until then). Implements
/// `LagStrategy` directly and is driven by EXCHANGE-TRUTH position (`ctx.position_qty`),
/// mirroring the live-bot design.
///
/// When FLAT it rests exactly one maker quote on a coinflip side at `offset_bps`
/// from mid (never marketable), re-anchoring (cancelling) the quote if it lingers
/// past `ttl_ns` unfilled so the control keeps trading and never deadlocks. When
/// FILLED it cancels any stray remainder and exits with a TAKER market order after
/// `hold_ns` (revert/timeout exit). Maker fee on the entry fill, taker fee on the
/// exit -> the control must still LOSE.
struct MakerCoin {
    rng: Rng,
    qty: Qty,
    offset_bps: f64,
    hold_ns: u64,
    ttl_ns: u64,
    next_id: u64,
    resting: Option<u64>,
    placed_at: u64,
    in_pos: bool,
    entry_ts: u64,
    exiting: bool,
}

impl MakerCoin {
    fn new(seed: u64, qty: Qty, offset_bps: f64, hold_ns: u64, ttl_ns: u64) -> Self {
        Self {
            rng: Rng(seed | 1),
            qty,
            offset_bps,
            hold_ns,
            ttl_ns,
            next_id: 1,
            resting: None,
            placed_at: 0,
            in_pos: false,
            entry_ts: 0,
            exiting: false,
        }
    }
}

impl LagStrategy for MakerCoin {
    fn on_event(&mut self, ctx: &LagCtx, out: &mut Vec<LagOrder>) {
        let mid = match (ctx.exec_book.best_bid(), ctx.exec_book.best_ask()) {
            (Some((b, _)), Some((a, _))) => (b.raw() + a.raw()) / 2,
            _ => return, // need a two-sided book to anchor a quote / price an exit
        };
        let pos = ctx.position_qty;

        if pos != 0 {
            // A maker quote filled -> we hold inventory. Cancel any stray remainder
            // and take a market exit after the hold (revert/timeout). Guard against
            // re-submitting the exit while it is in flight.
            if !self.in_pos {
                self.in_pos = true;
                self.entry_ts = ctx.now;
            }
            if let Some(id) = self.resting.take() {
                out.push(LagOrder::cancel(id));
            }
            if !self.exiting && ctx.now.saturating_sub(self.entry_ts) >= self.hold_ns {
                let side = if pos > 0 { Side::Ask } else { Side::Bid };
                out.push(LagOrder::market(side, Qty::from_raw(pos.unsigned_abs() as i64)));
                self.exiting = true;
            }
            return;
        }

        // FLAT.
        if self.in_pos {
            self.in_pos = false;
            self.exiting = false;
        }
        match self.resting {
            Some(id) => {
                // Re-anchor a stale unfilled quote so the control keeps trading
                // (and so a quote can never deadlock the control).
                if ctx.now.saturating_sub(self.placed_at) >= self.ttl_ns {
                    out.push(LagOrder::cancel(id));
                    self.resting = None;
                }
            }
            None => {
                let side = if self.rng.bit() { Side::Bid } else { Side::Ask };
                let off = ((mid as f64) * self.offset_bps / 1e4) as i64;
                let px = match side {
                    Side::Bid => mid - off,
                    Side::Ask => mid + off,
                };
                if px <= 0 {
                    return;
                }
                let id = self.next_id;
                self.next_id += 1;
                out.push(LagOrder::place(id, side, Price::from_raw(px), self.qty));
                self.resting = Some(id);
                self.placed_at = ctx.now;
            }
        }
    }
}

fn ev(role: Role, kind: LagKind, side: Side, px_raw: i64, qty_raw: i64, ts: u64) -> LagEvent {
    LagEvent {
        role,
        kind,
        exch_ts: ts,
        local_ts: ts,
        side: Some(side),
        price: Price::from_raw(px_raw),
        qty: Qty::from_raw(qty_raw),
        src: 0,
        aux: 0.0,
    }
}

/// SYNTHETIC stream: a random-walk mid with a fixed 2-tick half-spread, full
/// book-delta maintenance, a Reference trade at mid, AND - the key extension over
/// the taker synth - a random two-sided HL EXEC trade each tick that prints at the
/// touch (Ask-aggressor at the bid, Bid-aggressor at the ask). Those exec trades
/// are what let a resting maker fill at all; making them coinflip-two-sided keeps
/// the control a true null (neither side is favored).
fn synth(seed: u64, ticks: usize) -> Vec<LagEvent> {
    let mut rng = Rng(seed | 1);
    let mut mid: i64 = 100 * 100_000_000; // 100.0 in fixed point (1e10 raw)
    let half = 2_000_000; // 2-tick half-spread -> spread = 4 ticks = ~4 bps
    let tick = 1_000_000;
    let size = Qty::from_f64(100.0).unwrap().raw();
    let tqty = Qty::from_f64(50.0).unwrap().raw(); // big enough to fully fill a small maker
    let mut prev: Option<(i64, i64)> = None;
    let mut ts: u64 = 1_000_000_000;
    let mut out = Vec::new();
    for i in 0..ticks {
        ts += 1_000_000; // 1 ms / tick
        if i > 0 {
            mid += if rng.bit() { tick } else { -tick };
        }
        let bid = mid - half;
        let ask = mid + half;
        out.push(ev(Role::Exec, LagKind::BookDelta, Side::Bid, bid, size, ts));
        out.push(ev(Role::Exec, LagKind::BookDelta, Side::Ask, ask, size, ts));
        if let Some((pb, pa)) = prev {
            if pb != bid {
                out.push(ev(Role::Exec, LagKind::BookDelta, Side::Bid, pb, 0, ts));
            }
            if pa != ask {
                out.push(ev(Role::Exec, LagKind::BookDelta, Side::Ask, pa, 0, ts));
            }
        }
        prev = Some((bid, ask));
        // Reference (OKX-style) trade at mid - present for parity with the taker
        // synth; the maker control does not depend on it.
        out.push(ev(Role::Reference, LagKind::Trade, Side::Bid, mid, 100_000_000, ts));
        // HL EXEC trade: coinflip aggressor, printing at the touch so it crosses a
        // resting quote on that side (adverse-selection fill).
        let aggr = if rng.bit() { Side::Ask } else { Side::Bid };
        let tpx = match aggr {
            Side::Ask => bid, // sell into the bid -> fills resting bids at >= bid
            Side::Bid => ask, // buy into the ask -> fills resting asks at <= ask
        };
        out.push(ev(Role::Exec, LagKind::Trade, aggr, tpx, tqty, ts));
    }
    out
}

fn run_synth(seed: u64, ticks: usize) -> forgelag::LagReport {
    let evs = synth(seed, ticks);
    let strat = MakerCoin::new(
        0xC01D_F00D ^ seed,
        Qty::from_f64(0.1).unwrap(),
        1.0,         // offset 1 bp from mid -> rests inside the 2-bp half-spread (true maker)
        0,           // immediate revert/timeout taker exit
        50_000_000,  // 50 ms re-anchor TTL
    );
    let mut eng = LagEngine::new(
        strat,
        LagConfig {
            order_latency_ns: 0,
            cancel_latency_ns: 0,
            exec_book_levels: 20,
            fees: FeeSchedule::legacy(),
        },
    );
    eng.run(evs.iter()).unwrap();
    eng.finish()
}

/// THE GATE (synthetic, mandatory): a coinflip maker must trade a lot and LOSE.
#[test]
fn coinflip_maker_loses_on_synthetic_stream() {
    let r = run_synth(0xC0FFEE, 12_000);
    eprintln!(
        "[null-edge maker / synthetic] round_trips={} maker_fills={} orders_filled={} net_pnl={:.6}",
        r.round_trips, r.maker_fills, r.orders_filled, r.net_pnl
    );
    assert!(
        r.maker_fills > 0,
        "the control must actually fill as a MAKER (else it is not testing the maker fill model); maker_fills={}",
        r.maker_fills
    );
    assert!(
        r.round_trips >= MIN_ROUND_TRIPS,
        "coinflip maker must complete >= {} round trips before a verdict (Req 7.2); got {}",
        MIN_ROUND_TRIPS,
        r.round_trips
    );
    assert!(
        r.net_pnl < 0.0,
        "NULL-EDGE GATE FAILED: a coinflip maker netted {} >= 0 on the synthetic stream -> the fill model is manufacturing edge. Block promotion (Req 7.1/7.3).",
        r.net_pnl
    );
}

/// DETERMINISM (Req 7.5): identical seed + stream -> byte-identical net + trips.
#[test]
fn coinflip_maker_is_deterministic_across_runs() {
    let a = run_synth(0xC0FFEE, 12_000);
    let b = run_synth(0xC0FFEE, 12_000);
    assert_eq!(a.round_trips, b.round_trips, "round trips must be identical across runs");
    assert_eq!(a.maker_fills, b.maker_fills, "maker fills must be identical across runs");
    assert_eq!(
        a.net_pnl.to_bits(),
        b.net_pnl.to_bits(),
        "net P&L must be byte-for-byte identical across runs (Req 7.5)"
    );
}

/// REAL ETH day (HL book+trades vs OKX-spot ref). Wired into the suite (Req 7.4)
/// but DATA-GUARDED: when the fresh tick tree is not present (off-box / no data)
/// the run is clearly DEFERRED with a note rather than failing the suite. When the
/// data is present (the box) it asserts the same negativity gate.
#[test]
fn coinflip_maker_loses_on_real_eth_day() {
    let root = PathBuf::from("/root/chd/fresh/ticks");
    let coin = "ETH";
    let date = "2025-11-01";
    let probe = Path::new(&root).join(coin).join("hlbook").join(date);
    if !probe.exists() {
        eprintln!(
            "[null-edge maker / real ETH day] DEFERRED: fresh tick tree not present at {} - the SYNTHETIC stream satisfies the mandatory gate; the real-day run is deferred to a box with data.",
            probe.display()
        );
        return;
    }

    let cfg = FeedConfig {
        root,
        coin: coin.to_string(),
        ref_symbols: vec!["ETH-USDT".to_string()], // OKX spot anchor
        lead_symbols: vec![],
        date: date.to_string(),
        hours: (0..8).map(|h| format!("{h:02}")).collect(),
        exec_latency_ns: 0,
        ref_latency_ns: 0,
    };
    let evs = load_window(&cfg).expect("load real ETH day");
    assert!(!evs.is_empty(), "real day produced no events");

    let strat = MakerCoin::new(
        0xBEEF_F00D,
        Qty::from_f64(0.01).unwrap(),
        0.05,        // ~touch offset so the maker actually fills on a real book
        0,           // immediate revert/timeout taker exit
        200_000_000, // 200 ms re-anchor TTL
    );
    let mut eng = LagEngine::new(
        strat,
        LagConfig {
            order_latency_ns: 0,
            cancel_latency_ns: 0,
            exec_book_levels: 20,
            fees: FeeSchedule::legacy(),
        },
    );
    eng.run(evs.iter()).unwrap();
    let r = eng.finish();
    eprintln!(
        "[null-edge maker / real ETH day] events={} round_trips={} maker_fills={} orders_filled={} net_pnl={:.6}",
        r.events, r.round_trips, r.maker_fills, r.orders_filled, r.net_pnl
    );
    assert!(r.maker_fills > 0, "the control must fill as a MAKER on the real day; maker_fills=0");
    assert!(
        r.round_trips >= MIN_ROUND_TRIPS,
        "coinflip maker must complete >= {} round trips on the real day before a verdict (Req 7.2); got {}",
        MIN_ROUND_TRIPS,
        r.round_trips
    );
    assert!(
        r.net_pnl < 0.0,
        "NULL-EDGE GATE FAILED: a coinflip maker netted {} >= 0 on the REAL ETH day -> the fill model is manufacturing edge. Block promotion (Req 7.1/7.3).",
        r.net_pnl
    );
}