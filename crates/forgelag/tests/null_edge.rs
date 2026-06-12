//! NULL-EDGE GATE for forgelag: a seeded coinflip over a synthetic two-sided
//! exec book (random-walk mid, fixed spread) + reference trades MUST net
//! negative (~ spread + fees). If a coinflip profits here, the lag engine lies.

use forge_core::{Price, Qty, Side};
use forgelag::{
    CoinSignal, FeeSchedule, LagConfig, LagEngine, LagEvent, LagKind, Managed, ManagedConfig, Role,
};

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

fn synth(seed: u64, ticks: usize) -> Vec<LagEvent> {
    let mut rng = Rng(seed | 1);
    let mut mid: i64 = 100 * 100_000_000;
    let half = 1_000_000;
    let tick = 1_000_000;
    let size = Qty::from_f64(100.0).unwrap().raw();
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
        out.push(ev(Role::Reference, LagKind::Trade, Side::Bid, mid, 100_000_000, ts));
    }
    out
}

#[test]
fn coinflip_loses_with_spread_and_fees() {
    let evs = synth(0xC0FFEE, 20_000);
    let cfg = ManagedConfig {
        qty: Qty::from_f64(0.1).unwrap(),
        hold_ns: 2_000_000,
        cooldown_ns: 1_000_000,
        tp_bps: 0.0,
        sl_bps: 0.0,
        fill_timeout_ns: 200_000_000,
        use_limit: false,
    };
    let strat = Managed::new(CoinSignal::new(99), cfg);
    let mut eng = LagEngine::new(
        strat,
        LagConfig { order_latency_ns: 0, cancel_latency_ns: 0, exec_book_levels: 20, fees: FeeSchedule::legacy() },
    );
    eng.run(evs.iter()).unwrap();
    let r = eng.finish();
    assert!(r.round_trips > 50, "coinflip must trade (round_trips={})", r.round_trips);
    assert!(r.net_pnl < 0.0, "coinflip net must be negative; got {}", r.net_pnl);
}