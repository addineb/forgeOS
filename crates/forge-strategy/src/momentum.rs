//! OFI-momentum thesis: "ride the push". When depth-normalised OFI over the
//! window exceeds a threshold, enter in that direction; exit on take-profit,
//! stop-loss, or a hold timeout. Entry can be market (taker) or limit (maker) -
//! a swept knob. `Shuffled` replaces the OFI sign with a seeded coin (same
//! cadence, no direction) - the Phase 3 no-fake-edge control.
//!
//! Gates: one open position; a cooldown between trades; ONE order in flight at
//! a time (it waits for a fill before acting again, so it never spams while an
//! order is in flight - important because events can share a timestamp).
//!
//! NOTE: unfilled limit (maker) entries are not yet auto-cancelled (the engine
//! has no cancel op); market entry is the fully-clean path.

use forge_core::{Qty, Side};
use forge_sim::{Ctx, OrderIntent, Strategy};

use crate::ofi::Ofi;

/// Direction source for entries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Signal {
    /// Trade the sign of OFI (the real thesis).
    Real,
    /// Trade a seeded coin instead (control: edge must collapse to ~fees).
    Shuffled,
}

/// One sweep cell: the knobs that define a momentum variant.
#[derive(Clone, Copy, Debug)]
pub struct MomentumConfig {
    /// OFI rolling window (updates).
    pub ofi_window: usize,
    /// Absolute depth-normalised OFI needed to enter.
    pub threshold: f64,
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
    /// Direction source.
    pub signal: Signal,
    /// Seed for the shuffled-direction control.
    pub seed: u64,
    /// Nanoseconds to wait for a fill before assuming it will not fill.
    pub fill_timeout_ns: u64,
}

impl Default for MomentumConfig {
    fn default() -> Self {
        Self {
            ofi_window: 20,
            threshold: 1.0,
            qty: Qty::from_raw(1_000_000), // 0.01
            hold: 50,
            cooldown: 10,
            tp_bps: 0.0,
            sl_bps: 0.0,
            use_limit: false,
            signal: Signal::Real,
            seed: 1,
            fill_timeout_ns: 200_000_000,
        }
    }
}

#[derive(Clone, Copy)]
enum Phase {
    Flat(u32),
    Open { left: u32, dir: i64, entry_mid: i64 },
}

/// The OFI-momentum strategy.
pub struct OfiMomentum {
    ofi: Ofi,
    cfg: MomentumConfig,
    rng: u64,
    pending: Option<(i64, u64)>, // (position when order sent, deadline ns)
    phase: Phase,
}

impl OfiMomentum {
    /// Build from a config.
    #[must_use]
    pub fn new(cfg: MomentumConfig) -> Self {
        Self { ofi: Ofi::new(cfg.ofi_window), rng: cfg.seed | 1, cfg, pending: None, phase: Phase::Flat(0) }
    }

    fn coin(&mut self) -> bool {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) & 1 == 1
    }

    fn flatten(pos: i64) -> OrderIntent {
        let side = if pos > 0 { Side::Ask } else { Side::Bid };
        OrderIntent::market(side, Qty::from_raw(pos.unsigned_abs() as i64))
    }
}

impl Strategy for OfiMomentum {
    fn on_event(&mut self, ctx: &Ctx, out: &mut Vec<OrderIntent>) {
        let mid = match (ctx.book.best_bid(), ctx.book.best_ask()) {
            (Some((bp, bq)), Some((ap, aq))) => {
                self.ofi.observe(bp.raw(), bq.raw(), ap.raw(), aq.raw());
                Some((bp.raw() + ap.raw()) / 2)
            }
            _ => None,
        };
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
                } else if self.ofi.ready() {
                    let norm = self.ofi.normalized();
                    if norm.abs() >= self.cfg.threshold {
                        let long = match self.cfg.signal {
                            Signal::Real => norm > 0.0,
                            Signal::Shuffled => self.coin(),
                        };
                        let side = if long { Side::Bid } else { Side::Ask };
                        if let Some(m) = mid {
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
                                dir: if long { 1 } else { -1 },
                                entry_mid: m,
                            };
                        }
                    }
                }
            }
            Phase::Flat(n) => self.phase = Phase::Flat(n - 1),
        }
    }
}