//! Strategy layer: a managed execution shell (entry/exit/hold/cooldown/TP-SL/
//! in-flight guard) wrapping a tiny `LagSignal` (direction only). Ships the
//! seeded coinflip (the null-edge control) and the basis-reversion signal with
//! its hunt variants.

use forge_core::{Qty, Side};

use crate::engine::{LagCtx, LagOrder, LagStrategy};

/// A direction-only signal; the shell owns all order/position management.
pub trait LagSignal {
    /// Update state on every event (default no-op).
    fn observe(&mut self, _ctx: &LagCtx) {}
    /// When flat and ready: `Some(side)` to enter, else `None`.
    fn entry(&mut self, ctx: &LagCtx) -> Option<Side>;
}

/// Shared execution knobs.
#[derive(Clone, Copy, Debug)]
pub struct ManagedConfig {
    /// Trade size.
    pub qty: Qty,
    /// Hold (ns) before a timeout exit.
    pub hold_ns: u64,
    /// Cooldown (ns) after a trade.
    pub cooldown_ns: u64,
    /// Take-profit bps of entry mid (0 = off).
    pub tp_bps: f64,
    /// Stop-loss bps of entry mid (0 = off).
    pub sl_bps: f64,
    /// Ns to wait for a fill before assuming none (MUST exceed order latency).
    pub fill_timeout_ns: u64,
}

#[derive(Clone, Copy)]
enum Phase {
    Flat { ready_at: u64 },
    Open { entry_ts: u64, dir: i64, entry_mid: i64 },
}

/// Wraps a [`LagSignal`] into a full [`LagStrategy`] with safe management.
pub struct Managed<S: LagSignal> {
    sig: S,
    cfg: ManagedConfig,
    pending: Option<(i64, u64)>,
    phase: Phase,
}

impl<S: LagSignal> Managed<S> {
    /// Build a managed strategy around a signal.
    pub fn new(sig: S, cfg: ManagedConfig) -> Self {
        Self { sig, cfg, pending: None, phase: Phase::Flat { ready_at: 0 } }
    }

    fn flatten(pos: i64) -> LagOrder {
        let side = if pos > 0 { Side::Ask } else { Side::Bid };
        LagOrder { side, qty: Qty::from_raw(pos.unsigned_abs() as i64) }
    }
}

impl<S: LagSignal> LagStrategy for Managed<S> {
    fn on_event(&mut self, ctx: &LagCtx, out: &mut Vec<LagOrder>) {
        self.sig.observe(ctx);
        let pos = ctx.position_qty;
        let now = ctx.now;

        if let Some((sent_pos, deadline)) = self.pending {
            if pos != sent_pos || now >= deadline {
                self.pending = None;
            }
        }
        if self.pending.is_some() {
            return;
        }

        let mid = match (ctx.exec_book.best_bid(), ctx.exec_book.best_ask()) {
            (Some((b, _)), Some((a, _))) => Some((b.raw() + a.raw()) / 2),
            _ => None,
        };

        match self.phase {
            Phase::Open { entry_ts, dir, entry_mid } => {
                let mut close = now.saturating_sub(entry_ts) >= self.cfg.hold_ns;
                if let Some(m) = mid {
                    let move_bps = ((m - entry_mid) as f64 / entry_mid as f64) * 10_000.0 * dir as f64;
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
                            out.push(LagOrder { side, qty: self.cfg.qty });
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

/// Seeded random-direction signal: the NULL-EDGE control (must lose ~costs).
pub struct CoinSignal {
    state: u64,
}
impl CoinSignal {
    /// New seeded coin.
    #[must_use]
    pub fn new(seed: u64) -> Self {
        Self { state: seed | 1 }
    }
    fn bit(&mut self) -> bool {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) & 1 == 1
    }
}
impl LagSignal for CoinSignal {
    fn entry(&mut self, _ctx: &LagCtx) -> Option<Side> {
        Some(if self.bit() { Side::Bid } else { Side::Ask })
    }
}

/// Basis-reversion signal variant knobs.
#[derive(Clone, Copy, Debug)]
pub struct BasisConfig {
    /// Microprice depth (levels per side).
    pub top_n: usize,
    /// |dev| in bps to enter.
    pub threshold_bps: f64,
    /// Rolling baseline length (samples).
    pub window: usize,
    /// Gap-sample cadence (ns).
    pub sample_ns: u64,
    /// true = fade the stretch (reversion); false = momentum control.
    pub reversion: bool,
}

/// Basis-reversion direction signal: fade large deviations of the HL microprice
/// from its recent basis vs the reference price.
pub struct BasisSignal {
    cfg: BasisConfig,
    ring: Vec<f64>,
    pos: usize,
    sum: f64,
    next_sample: u64,
    started: bool,
    cur_gap: f64,
    have_gap: bool,
}
impl BasisSignal {
    /// New basis signal.
    #[must_use]
    pub fn new(cfg: BasisConfig) -> Self {
        Self {
            cfg: BasisConfig { top_n: cfg.top_n.max(1), window: cfg.window.max(1), sample_ns: cfg.sample_ns.max(1), ..cfg },
            ring: Vec::new(),
            pos: 0,
            sum: 0.0,
            next_sample: 0,
            started: false,
            cur_gap: 0.0,
            have_gap: false,
        }
    }
    fn micro(&self, ctx: &LagCtx) -> Option<f64> {
        let (bb, _) = ctx.exec_book.best_bid()?;
        let (ba, _) = ctx.exec_book.best_ask()?;
        let bb = bb.to_f64();
        let ba = ba.to_f64();
        let bq: f64 = ctx.exec_book.bids_iter().take(self.cfg.top_n).map(|(_, q)| q.raw() as f64).sum();
        let aq: f64 = ctx.exec_book.asks_iter().take(self.cfg.top_n).map(|(_, q)| q.raw() as f64).sum();
        let tot = bq + aq;
        if tot <= 0.0 {
            return Some((bb + ba) / 2.0);
        }
        Some((bb * aq + ba * bq) / tot)
    }
    fn push_sample(&mut self, g: f64) {
        if self.ring.len() < self.cfg.window {
            self.ring.push(g);
            self.sum += g;
        } else {
            let old = self.ring[self.pos];
            self.sum += g - old;
            self.ring[self.pos] = g;
            self.pos = (self.pos + 1) % self.cfg.window;
        }
    }
}
impl LagSignal for BasisSignal {
    fn observe(&mut self, ctx: &LagCtx) {
        if ctx.ref_px > 0.0 {
            if let Some(m) = self.micro(ctx) {
                self.cur_gap = (m - ctx.ref_px) / ctx.ref_px * 10_000.0;
                self.have_gap = true;
            }
        }
        if self.have_gap {
            if !self.started {
                self.next_sample = ctx.now;
                self.started = true;
            }
            if ctx.now >= self.next_sample {
                let g = self.cur_gap;
                self.push_sample(g);
                while self.next_sample <= ctx.now {
                    self.next_sample += self.cfg.sample_ns;
                }
            }
        }
    }
    fn entry(&mut self, _ctx: &LagCtx) -> Option<Side> {
        if !self.have_gap || self.ring.len() < 20 {
            return None;
        }
        let base = self.sum / self.ring.len() as f64;
        let dev = self.cur_gap - base;
        if dev.abs() < self.cfg.threshold_bps {
            return None;
        }
        let rich = dev > 0.0;
        let long = if self.cfg.reversion { !rich } else { rich };
        Some(if long { Side::Bid } else { Side::Ask })
    }
}