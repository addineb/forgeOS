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
    /// When in a position: return true to request an early exit (default never).
    fn exit(&self, _ctx: &LagCtx) -> bool {
        false
    }
    /// Size multiplier for the entry (default 1x); used for magnitude sizing.
    fn size_mult(&self) -> f64 {
        1.0
    }
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
    /// Enter with a LIMIT (maker) at touch instead of MARKET (taker). Exits stay market.
    pub use_limit: bool,
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
        LagOrder::market(side, Qty::from_raw(pos.unsigned_abs() as i64))
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
                if self.sig.exit(ctx) {
                    close = true;
                }
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
                            let mlt = self.sig.size_mult().max(0.01);
                            let sz = forge_core::Qty::from_raw((self.cfg.qty.raw() as f64 * mlt) as i64);
                            let intent = if self.cfg.use_limit {
                                let px = match side {
                                    Side::Bid => ctx.exec_book.best_bid().map(|(p, _)| p),
                                    Side::Ask => ctx.exec_book.best_ask().map(|(p, _)| p),
                                };
                                match px {
                                    Some(p) => LagOrder::limit(side, p, sz),
                                    None => return,
                                }
                            } else {
                                LagOrder::market(side, sz)
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
    /// Randomize entry DIRECTION (no-fake-edge control); same trigger cadence.
    pub shuffle: bool,
    /// Seed for the shuffle control.
    pub seed: u64,
    /// Gate entries on basis VELOCITY (only enter while still widening) to
    /// anticipate our own order latency (fill lands near the peak stretch).
    pub vel_gate: bool,
    /// Min |dev| change over the lookback (bps) to count as stretching fast.
    pub vel_min_bps: f64,
    /// Velocity lookback in samples.
    pub vel_lookback: usize,
    /// Exit when the basis reverts to within exit_bps of its mean (0/false=off,
    /// fall back to the hold timeout).
    pub exit_revert: bool,
    /// |dev| (bps) at/below which the revert-to-mean exit fires.
    pub exit_bps: f64,
    /// Use a z-score trigger: effective threshold = z_k * rolling std of the gap.
    pub zscore: bool,
    /// z multiplier for the z-score trigger.
    pub z_k: f64,
    /// Require HL book pressure to AGREE with the reversion (bid-heavy to buy the
    /// dip / ask-heavy to sell the rip) - the dead imbalance indicator as a CONFIRM.
    pub confirm: bool,
    /// Min top-N book imbalance in [0,1] to count as agreement.
    pub confirm_imb: f64,
    /// Scale entry size by dislocation magnitude (|dev|/threshold, capped).
    pub mag_size: bool,
    /// Max size multiplier for magnitude sizing.
    pub mag_cap: f64,
    /// Only enter when |funding rate| >= fund_min (extreme/crowded positioning).
    pub fund_gate: bool,
    /// Funding magnitude threshold (absolute per-hour rate).
    pub fund_min: f64,
    /// Require funding sign to support the reversion (long needs negative funding,
    /// short needs positive) = the crowded side being unwound.
    pub fund_align: bool,
}

/// Basis-reversion direction signal: fade large deviations of the HL microprice
/// from its recent basis vs the reference price.
pub struct BasisSignal {
    cfg: BasisConfig,
    rng: u64,
    ring: Vec<f64>,
    pos: usize,
    sum: f64,
    sum_sq: f64,
    next_sample: u64,
    started: bool,
    cur_gap: f64,
    have_gap: bool,
    cur_dev: f64,
    dev_hist: Vec<f64>,
    have_dev: bool,
}
impl BasisSignal {
    /// New basis signal.
    #[must_use]
    pub fn new(cfg: BasisConfig) -> Self {
        Self {
            cfg: BasisConfig { top_n: cfg.top_n.max(1), window: cfg.window.max(1), sample_ns: cfg.sample_ns.max(1), ..cfg },
            rng: cfg.seed | 1,
            ring: Vec::new(),
            pos: 0,
            sum: 0.0,
            sum_sq: 0.0,
            next_sample: 0,
            started: false,
            cur_gap: 0.0,
            have_gap: false,
            cur_dev: 0.0,
            dev_hist: Vec::new(),
            have_dev: false,
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
    fn coin(&mut self) -> bool {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) & 1 == 1
    }
    fn push_sample(&mut self, g: f64) {
        if self.ring.len() < self.cfg.window {
            self.ring.push(g);
            self.sum += g;
            self.sum_sq += g * g;
        } else {
            let old = self.ring[self.pos];
            self.sum += g - old;
            self.sum_sq += g * g - old * old;
            self.ring[self.pos] = g;
            self.pos = (self.pos + 1) % self.cfg.window;
        }
    }
    fn gap_std(&self) -> f64 {
        let n = self.ring.len() as f64;
        if n < 2.0 {
            return 0.0;
        }
        let mean = self.sum / n;
        (self.sum_sq / n - mean * mean).max(0.0).sqrt()
    }
    fn book_imb(&self, ctx: &LagCtx) -> f64 {
        let bq: f64 = ctx.exec_book.bids_iter().take(self.cfg.top_n).map(|(_, q)| q.raw() as f64).sum();
        let aq: f64 = ctx.exec_book.asks_iter().take(self.cfg.top_n).map(|(_, q)| q.raw() as f64).sum();
        let tot = bq + aq;
        if tot <= 0.0 { 0.0 } else { (bq - aq) / tot }
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
                let base = if self.ring.is_empty() { self.cur_gap } else { self.sum / self.ring.len() as f64 };
                self.cur_dev = self.cur_gap - base;
                self.have_dev = self.ring.len() >= 20;
                self.dev_hist.push(self.cur_dev);
                self.push_sample(self.cur_gap);
                while self.next_sample <= ctx.now {
                    self.next_sample += self.cfg.sample_ns;
                }
            }
        }
    }
    fn entry(&mut self, ctx: &LagCtx) -> Option<Side> {
        if !self.have_dev {
            return None;
        }
        let dev = self.cur_dev;
        let thr = if self.cfg.zscore { self.cfg.z_k * self.gap_std() } else { self.cfg.threshold_bps };
        if dev.abs() < thr {
            return None;
        }
        if self.cfg.vel_gate {
            let n = self.dev_hist.len();
            if n <= self.cfg.vel_lookback {
                return None;
            }
            let vel = dev - self.dev_hist[n - 1 - self.cfg.vel_lookback];
            // only enter while the gap is STILL widening fast -> our delayed fill
            // lands near the peak stretch instead of chasing a reverting gap.
            if vel.signum() != dev.signum() || vel.abs() < self.cfg.vel_min_bps {
                return None;
            }
        }
        let rich = dev > 0.0;
        let mut long = if self.cfg.reversion { !rich } else { rich };
        if self.cfg.confirm {
            let imb = self.book_imb(ctx);
            let agree = if long { imb >= self.cfg.confirm_imb } else { imb <= -self.cfg.confirm_imb };
            if !agree {
                return None;
            }
        }
        if self.cfg.fund_gate && ctx.funding.abs() < self.cfg.fund_min {
            return None;
        }
        if self.cfg.fund_align {
            let supportive = if long { ctx.funding < 0.0 } else { ctx.funding > 0.0 };
            if !supportive {
                return None;
            }
        }
        if self.cfg.shuffle {
            long = self.coin();
        }
        Some(if long { Side::Bid } else { Side::Ask })
    }
    fn exit(&self, _ctx: &LagCtx) -> bool {
        self.cfg.exit_revert && self.have_dev && self.cur_dev.abs() <= self.cfg.exit_bps
    }
    fn size_mult(&self) -> f64 {
        if !self.cfg.mag_size || !self.have_dev {
            return 1.0;
        }
        let reference = if self.cfg.threshold_bps > 0.0 { self.cfg.threshold_bps } else { 10.0 };
        (self.cur_dev.abs() / reference).clamp(1.0, self.cfg.mag_cap.max(1.0))
    }
}