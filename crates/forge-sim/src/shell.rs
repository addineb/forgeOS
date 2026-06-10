//! The shared EXECUTION SHELL: the order/position plumbing every bot needs,
//! written and tested ONCE so each new thesis only writes its signal.
//!
//! A strategy implements [`EntrySignal`] (when flat, "do I enter, and which
//! way?"); the shell handles everything dangerous and repetitive:
//!   - ONE order in flight at a time (wait for the position to change or a
//!     fill-timeout before acting again) - events can share a timestamp, so an
//!     event-count cooldown passes ~0 real time and naive logic spams orders;
//!   - enter only on a two-sided book (a market order cannot trivially reject);
//!   - exits on take-profit / stop-loss (bps of entry mid) or a hold timeout;
//!   - flatten the exact held size (partial fills do not let inventory drift);
//!   - a cooldown between trades; at most one open position.
//!
//! This is the "execution shell" of the design (engine-design section 4): the
//! cheap, faithful wrapper a sweep fans out per combo over a shared signal.

use forge_core::{Qty, Side};

use crate::strategy::{Ctx, OrderIntent, Strategy};

/// A bot's signal: the only part each thesis writes.
pub trait EntrySignal {
    /// Called every event so the signal can update its state from the book/tape
    /// regardless of position. Default: no-op.
    fn observe(&mut self, _ctx: &Ctx) {}

    /// Called when flat and ready to trade. Return `Some(side)` to enter
    /// (`Bid` = go long, `Ask` = go short), or `None` to stand aside.
    fn entry(&mut self, ctx: &Ctx) -> Option<Side>;
}

/// Execution knobs shared by every bot (the sweepable trade-management cell).
#[derive(Clone, Copy, Debug)]
pub struct ExecConfig {
    /// Trade size.
    pub qty: Qty,
    /// Max events to hold before a timeout exit.
    pub hold: u32,
    /// Events to wait after a trade before the next.
    pub cooldown: u32,
    /// Take-profit in bps of entry mid (0 = disabled).
    pub tp_bps: f64,
    /// Stop-loss in bps of entry mid (0 = disabled).
    pub sl_bps: f64,
    /// Enter with a limit (maker) instead of a market (taker) order.
    pub use_limit: bool,
    /// Nanoseconds to wait for a fill before assuming it will not happen.
    pub fill_timeout_ns: u64,
}

impl Default for ExecConfig {
    fn default() -> Self {
        Self {
            qty: Qty::from_raw(1_000_000), // 0.01
            hold: 50,
            cooldown: 10,
            tp_bps: 0.0,
            sl_bps: 0.0,
            use_limit: false,
            fill_timeout_ns: 200_000_000,
        }
    }
}

#[derive(Clone, Copy)]
enum Phase {
    Flat(u32),
    Open { left: u32, dir: i64, entry_mid: i64 },
}

/// Wraps an [`EntrySignal`] into a full `forge_sim::Strategy` with safe order
/// and position management.
pub struct ExecutionShell<S: EntrySignal> {
    sig: S,
    cfg: ExecConfig,
    pending: Option<(i64, u64)>, // (position when an order was sent, deadline ns)
    phase: Phase,
}

impl<S: EntrySignal> ExecutionShell<S> {
    /// Build a shell around a signal.
    pub fn new(sig: S, cfg: ExecConfig) -> Self {
        Self { sig, cfg, pending: None, phase: Phase::Flat(0) }
    }

    fn flatten(pos: i64) -> OrderIntent {
        let side = if pos > 0 { Side::Ask } else { Side::Bid };
        OrderIntent::market(side, Qty::from_raw(pos.unsigned_abs() as i64))
    }
}

impl<S: EntrySignal> Strategy for ExecutionShell<S> {
    fn on_event(&mut self, ctx: &Ctx, out: &mut Vec<OrderIntent>) {
        // The signal always gets to update its state.
        self.sig.observe(ctx);

        let pos = ctx.position_qty;
        let now = ctx.now.get();

        // Resolve any in-flight order, then refuse to act while one is pending.
        if let Some((sent_pos, deadline)) = self.pending {
            if pos != sent_pos || now >= deadline {
                self.pending = None;
            }
        }
        if self.pending.is_some() {
            return;
        }

        let mid = match (ctx.book.best_bid(), ctx.book.best_ask()) {
            (Some((b, _)), Some((a, _))) => Some((b.raw() + a.raw()) / 2),
            _ => None,
        };

        match self.phase {
            Phase::Open { left, dir, entry_mid } => {
                let mut close = left == 0;
                if let Some(m) = mid {
                    let move_bps =
                        ((m - entry_mid) as f64 / entry_mid as f64) * 10_000.0 * dir as f64;
                    if self.cfg.tp_bps > 0.0 && move_bps >= self.cfg.tp_bps {
                        close = true;
                    }
                    if self.cfg.sl_bps > 0.0 && move_bps <= -self.cfg.sl_bps {
                        close = true;
                    }
                }
                if close {
                    if pos != 0 {
                        out.push(Self::flatten(pos));
                        self.pending = Some((pos, now + self.cfg.fill_timeout_ns));
                    }
                    self.phase = Phase::Flat(self.cfg.cooldown);
                } else {
                    self.phase = Phase::Open { left: left.saturating_sub(1), dir, entry_mid };
                }
            }
            Phase::Flat(0) => {
                if pos != 0 {
                    out.push(Self::flatten(pos));
                    self.pending = Some((pos, now + self.cfg.fill_timeout_ns));
                    self.phase = Phase::Flat(self.cfg.cooldown);
                } else if let Some(m) = mid {
                    if let Some(side) = self.sig.entry(ctx) {
                        let intent = if self.cfg.use_limit {
                            let px = match side {
                                Side::Bid => ctx.book.best_bid().map(|(p, _)| p),
                                Side::Ask => ctx.book.best_ask().map(|(p, _)| p),
                            };
                            match px {
                                Some(p) => OrderIntent::limit(side, p, self.cfg.qty),
                                None => return,
                            }
                        } else {
                            OrderIntent::market(side, self.cfg.qty)
                        };
                        out.push(intent);
                        self.pending = Some((pos, now + self.cfg.fill_timeout_ns));
                        self.phase = Phase::Open {
                            left: self.cfg.hold,
                            dir: if side == Side::Bid { 1 } else { -1 },
                            entry_mid: m,
                        };
                    }
                }
            }
            Phase::Flat(n) => self.phase = Phase::Flat(n - 1),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FeeSchedule, SimConfig, SimEngine};
    use forge_core::{Event, EventKind, Price, Qty, Side, UnixNanos};

    /// A signal that always wants to go long - to exercise the shell plumbing.
    struct AlwaysLong;
    impl EntrySignal for AlwaysLong {
        fn entry(&mut self, _ctx: &Ctx) -> Option<Side> {
            Some(Side::Bid)
        }
    }

    fn delta(side: Side, px: i64, qty: i64, ts: u64) -> Event {
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

    #[test]
    fn shell_round_trips_and_pays_costs() {
        // two-sided book at 100/101, size 100, walking timestamps
        let mut evs = Vec::new();
        let mut ts = 1_000_000u64;
        for _ in 0..200 {
            ts += 1_000_000;
            evs.push(delta(Side::Bid, 100 * 100_000_000, 100 * 100_000_000, ts));
            evs.push(delta(Side::Ask, 101 * 100_000_000, 100 * 100_000_000, ts));
        }
        let cfg = ExecConfig { qty: Qty::from_f64(0.1).unwrap(), hold: 2, cooldown: 1, ..ExecConfig::default() };
        let shell = ExecutionShell::new(AlwaysLong, cfg);
        let mut eng = SimEngine::new(shell, SimConfig { order_latency_ns: 0, book_max_levels: 20, fees: FeeSchedule::legacy() });
        eng.run(evs.iter()).unwrap();
        let r = eng.finish();
        // it traded, and a buy-high/sell-low round trip on a static book loses.
        assert!(r.round_trips > 5, "shell must trade (round_trips={})", r.round_trips);
        assert!(r.net_pnl < 0, "always-long on a static book pays the spread+fees");
        // clean accounting: roughly two fills per round trip, no runaway.
        assert!(r.orders_filled <= r.round_trips * 3 + 5, "no order spam");
    }
}