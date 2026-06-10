//! The shared EXECUTION SHELL: the order/position plumbing every bot needs,
//! written and tested ONCE so each new thesis only writes its signal.
//!
//! A strategy implements [`EntrySignal`] (when flat, "do I enter, and which
//! way?"); the shell handles everything dangerous and repetitive:
//!   - ONE order in flight at a time (wait for the position to change or a
//!     fill-timeout before acting again);
//!   - enter only on a two-sided book;
//!   - exits on take-profit / stop-loss (bps of entry mid) or a hold TIMEOUT
//!     measured in virtual time (nanoseconds), not event counts;
//!   - flatten the exact held size; a cooldown (ns) between trades; one position.
//!
//! Holds and cooldowns are in NANOSECONDS of virtual time, so "hold 30 minutes"
//! is expressible and robust to event clustering (events can share a timestamp).

use forge_core::{Qty, Side};

use crate::strategy::{Ctx, OrderIntent, Strategy};

/// A bot's signal: the only part each thesis writes.
pub trait EntrySignal {
    /// Called every event so the signal can update its state. Default: no-op.
    fn observe(&mut self, _ctx: &Ctx) {}
    /// Called when flat and ready: `Some(side)` to enter (`Bid` long / `Ask`
    /// short), or `None` to stand aside.
    fn entry(&mut self, ctx: &Ctx) -> Option<Side>;
}

/// Execution knobs shared by every bot (the sweepable trade-management cell).
#[derive(Clone, Copy, Debug)]
pub struct ExecConfig {
    /// Trade size.
    pub qty: Qty,
    /// Hold duration in nanoseconds before a timeout exit.
    pub hold_ns: u64,
    /// Cooldown in nanoseconds after a trade before the next.
    pub cooldown_ns: u64,
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
            hold_ns: 5_000_000_000,        // 5 s
            cooldown_ns: 1_000_000_000,    // 1 s
            tp_bps: 0.0,
            sl_bps: 0.0,
            use_limit: false,
            fill_timeout_ns: 200_000_000,
        }
    }
}

#[derive(Clone, Copy)]
enum Phase {
    Flat { ready_at: u64 },
    Open { entry_ts: u64, dir: i64, entry_mid: i64 },
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
        Self { sig, cfg, pending: None, phase: Phase::Flat { ready_at: 0 } }
    }

    fn flatten(pos: i64) -> OrderIntent {
        let side = if pos > 0 { Side::Ask } else { Side::Bid };
        OrderIntent::market(side, Qty::from_raw(pos.unsigned_abs() as i64))
    }
}

impl<S: EntrySignal> Strategy for ExecutionShell<S> {
    fn on_event(&mut self, ctx: &Ctx, out: &mut Vec<OrderIntent>) {
        self.sig.observe(ctx);

        let pos = ctx.position_qty;
        let now = ctx.now.get();

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
            Phase::Open { entry_ts, dir, entry_mid } => {
                let mut close = now.saturating_sub(entry_ts) >= self.cfg.hold_ns;
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
                    self.phase = Phase::Flat { ready_at: now + self.cfg.cooldown_ns };
                }
            }
            Phase::Flat { ready_at } => {
                if pos != 0 {
                    out.push(Self::flatten(pos));
                    self.pending = Some((pos, now + self.cfg.fill_timeout_ns));
                    self.phase = Phase::Flat { ready_at: now + self.cfg.cooldown_ns };
                } else if now >= ready_at {
                    if let Some(m) = mid {
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
                                entry_ts: now,
                                dir: if side == Side::Bid { 1 } else { -1 },
                                entry_mid: m,
                            };
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{FeeSchedule, SimConfig, SimEngine};
    use forge_core::{Event, EventKind, Price, Qty, Side, UnixNanos};

    struct AlwaysLong;
    impl EntrySignal for AlwaysLong {
        fn entry(&mut self, _ctx: &Ctx) -> Option<Side> {
            Some(Side::Bid)
        }
    }

    fn delta(side: Side, px: i64, qty: i64, ts: u64) -> Event {
        Event::new(EventKind::BookDelta, UnixNanos::new(ts), UnixNanos::new(ts), Some(side), Price::from_raw(px), Qty::from_raw(qty), 0).unwrap()
    }

    #[test]
    fn shell_round_trips_and_pays_costs() {
        let mut evs = Vec::new();
        let mut ts = 1_000_000u64;
        for _ in 0..200 {
            ts += 1_000_000; // 1 ms / tick
            evs.push(delta(Side::Bid, 100 * 100_000_000, 100 * 100_000_000, ts));
            evs.push(delta(Side::Ask, 101 * 100_000_000, 100 * 100_000_000, ts));
        }
        // hold 2 ms, cooldown 1 ms
        let cfg = ExecConfig { qty: Qty::from_f64(0.1).unwrap(), hold_ns: 2_000_000, cooldown_ns: 1_000_000, ..ExecConfig::default() };
        let shell = ExecutionShell::new(AlwaysLong, cfg);
        let mut eng = SimEngine::new(shell, SimConfig { order_latency_ns: 0, book_max_levels: 20, fees: FeeSchedule::legacy() });
        eng.run(evs.iter()).unwrap();
        let r = eng.finish();
        assert!(r.round_trips > 5, "shell must trade (round_trips={})", r.round_trips);
        assert!(r.net_pnl < 0, "always-long on a static book pays the spread+fees");
    }
}