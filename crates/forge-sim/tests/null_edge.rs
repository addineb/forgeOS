//! The NULL-EDGE GATE (docs/roadmap.md Phase 2): a seeded coinflip strategy run
//! over a market with a real spread + fees MUST net negative (~ spread + fees).
//! If a coinflip profits, the engine is lying.
//!
//! Uses a deterministic synthetic 1-level book (random-walk mid, fixed spread)
//! so the gate runs in CI without the box. The same coinflip is also run over
//! real data on the box for validation.

use forge_core::{Event, EventKind, Price, Qty, Side, UnixNanos};
use forge_sim::{money_to_f64, Coinflip, FeeSchedule, SimConfig, SimEngine};

struct Rng(u64);
impl Rng {
    fn bit(&mut self) -> bool {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        (x >> 33) & 1 == 1
    }
}

/// Synthetic market: one bid/ask level around a random-walk mid, held wide so
/// the coinflip fills at the touch.
fn synth_market(seed: u64, ticks: usize, spread_raw: i64, tick_raw: i64) -> Vec<Event> {
    let mut rng = Rng(seed | 1);
    let mut mid: i64 = 100 * 100_000_000;
    let half = spread_raw / 2;
    let size = Qty::from_f64(100.0).unwrap();
    let mut prev: Option<(i64, i64)> = None;
    let mut ts: u64 = 1_000_000_000;
    let mut out = Vec::with_capacity(ticks * 4);

    fn push(out: &mut Vec<Event>, side: Side, px: i64, qty: i64, ts: u64) {
        out.push(
            Event::new(
                EventKind::BookDelta,
                UnixNanos::new(ts),
                UnixNanos::new(ts),
                Some(side),
                Price::from_raw(px),
                Qty::from_raw(qty),
                0,
            )
            .unwrap(),
        );
    }

    for i in 0..ticks {
        ts += 1_000_000; // 1ms/tick so the fill-timeout is proportionally small
        if i > 0 {
            mid += if rng.bit() { tick_raw } else { -tick_raw };
        }
        let bid = mid - half;
        let ask = mid + half;
        // set the new touch BEFORE removing the old, so the book is never
        // one-sided (a market order always has a side to fill against)
        push(&mut out, Side::Bid, bid, size.raw(), ts);
        push(&mut out, Side::Ask, ask, size.raw(), ts);
        if let Some((pb, pa)) = prev {
            if pb != bid {
                push(&mut out, Side::Bid, pb, 0, ts);
            }
            if pa != ask {
                push(&mut out, Side::Ask, pa, 0, ts);
            }
        }
        prev = Some((bid, ask));
    }
    out
}

#[test]
fn coinflip_loses_with_spread_and_fees() {
    let evs = synth_market(0xC0FFEE, 20_000, 2_000_000, 1_000_000);
    let qty = Qty::from_f64(0.1).unwrap();
    let cfg = SimConfig { order_latency_ns: 0, book_max_levels: 20, fees: FeeSchedule::legacy() };
    let mut eng = SimEngine::new(Coinflip::new(99, qty, 2, 1), cfg);
    eng.run(evs.iter()).unwrap();
    let r = eng.finish();

    assert!(r.round_trips > 100, "coinflip must trade (round_trips={})", r.round_trips);
    assert!(r.net_pnl < 0, "coinflip net must be negative; got {}", money_to_f64(r.net_pnl));
}

#[test]
fn spread_alone_loses_and_fees_make_it_worse() {
    let evs = synth_market(0xC0FFEE, 20_000, 2_000_000, 1_000_000);
    let qty = Qty::from_f64(0.1).unwrap();

    let mut eng_zero = SimEngine::new(
        Coinflip::new(99, qty, 2, 1),
        SimConfig { order_latency_ns: 0, book_max_levels: 20, fees: FeeSchedule::zero() },
    );
    eng_zero.run(evs.iter()).unwrap();
    let zero = eng_zero.finish();

    let mut eng_fee = SimEngine::new(
        Coinflip::new(99, qty, 2, 1),
        SimConfig { order_latency_ns: 0, book_max_levels: 20, fees: FeeSchedule::legacy() },
    );
    eng_fee.run(evs.iter()).unwrap();
    let fee = eng_fee.finish();

    assert!(zero.net_pnl < 0, "spread alone must lose: {}", money_to_f64(zero.net_pnl));
    assert!(
        fee.net_pnl < zero.net_pnl,
        "fees must make it worse: fee={} zero={}",
        money_to_f64(fee.net_pnl),
        money_to_f64(zero.net_pnl)
    );
}