//! Phase 3 GATE: a flow strategy must not manufacture edge from nothing.
//!
//! On a deterministic random-walk market (no real predictability), the OFI
//! momentum strategy must LOSE ~spread+fees - just like a coinflip - whether it
//! trades the real OFI sign or a shuffled (random) direction. If it profited on
//! random data, the engine would be leaking lookahead or P&L. We also assert the
//! run is deterministic.

use forge_core::{Event, EventKind, Price, Qty, Side, UnixNanos};
use forge_sim::{money_to_f64, FeeSchedule, SimConfig, SimEngine};
use forge_strategy::{MomentumConfig, OfiMomentum, RegimeFilter, Signal};

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

fn synth_market(seed: u64, ticks: usize) -> Vec<Event> {
    let mut rng = Rng(seed | 1);
    let mut mid: i64 = 100 * 100_000_000;
    let half = 1_000_000; // 0.01 half-spread
    let tick = 1_000_000; // 0.01 walk step
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
            mid += if rng.bit() { tick } else { -tick };
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

fn cfg(signal: Signal) -> MomentumConfig {
    MomentumConfig {
        ofi_window: 5,
        threshold: 0.3,
        qty: Qty::from_f64(0.1).unwrap(),
        hold_ns: 3_000_000,
        cooldown_ns: 1_000_000,
        tp_bps: 0.0,
        sl_bps: 0.0,
        use_limit: false,
        signal,
        seed: 7,
        fill_timeout_ns: 200_000_000,
        regime_filter: RegimeFilter::Any,
    }
}

fn run(evs: &[Event], signal: Signal) -> forge_sim::SimReport {
    let sim_cfg = SimConfig { order_latency_ns: 0, book_max_levels: 20, fees: FeeSchedule::legacy() };
    let mut eng = SimEngine::new(OfiMomentum::new(cfg(signal)), sim_cfg);
    eng.run(evs.iter()).unwrap();
    eng.finish()
}

#[test]
fn ofi_momentum_has_no_edge_on_random_data() {
    let evs = synth_market(0xABCD, 20_000);
    let r = run(&evs, Signal::Real);
    assert!(r.round_trips > 50, "must actually trade (round_trips={})", r.round_trips);
    assert!(
        r.net_pnl < 0,
        "OFI momentum must lose on random data; got {}",
        money_to_f64(r.net_pnl)
    );
}

#[test]
fn shuffled_direction_also_collapses_to_costs() {
    let evs = synth_market(0xABCD, 20_000);
    let r = run(&evs, Signal::Shuffled);
    assert!(r.net_pnl < 0, "shuffled direction must also lose; got {}", money_to_f64(r.net_pnl));
}

#[test]
fn strategy_is_deterministic() {
    let evs = synth_market(0xABCD, 10_000);
    assert_eq!(run(&evs, Signal::Real), run(&evs, Signal::Real));
}