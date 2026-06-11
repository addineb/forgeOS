//! Spot-perp BASIS-REVERSION thesis (the lag-subspace lead). The instrument we
//! TRADE is the Hyperliquid book (`ctx.book`, fed from HL full-depth snapshot
//! deltas); the REFERENCE leg is Binance, read from `Trade` events. We measure
//! the basis gap = (HL_microprice - Binance) / Binance in bps, keep a rolling
//! time-sampled baseline, and FADE large deviations (dev = gap - baseline): HL
//! rich vs its recent basis -> short HL; HL cheap -> long HL. Execution (taker
//! fills paying the real HL spread + slippage, plus order latency / adverse
//! selection) lives in the shared `ExecutionShell`. `Shuffled` swaps the
//! direction for a seeded coin = the no-fake-edge control.

use forge_core::{EventKind, Qty, Side};
use forge_sim::{Ctx, EntrySignal, ExecConfig, ExecutionShell, OrderIntent, RegimeFilter, Strategy};

use crate::momentum::Signal;

/// One sweep cell for the basis-reversion bot.
#[derive(Clone, Copy, Debug)]
pub struct BasisConfig {
    /// Depth (levels per side) used for the HL microprice.
    pub top_n: usize,
    /// |dev| in bps required to enter.
    pub threshold_bps: f64,
    /// Rolling baseline length, in gap samples.
    pub window: usize,
    /// Virtual-time cadence (ns) at which a gap sample feeds the baseline.
    pub sample_ns: u64,
    /// `true` = reversion (fade the stretch, the thesis); `false` = momentum control.
    pub reversion: bool,
    /// Trade size.
    pub qty: Qty,
    /// Hold duration (ns) before a timeout exit.
    pub hold_ns: u64,
    /// Cooldown (ns) between trades.
    pub cooldown_ns: u64,
    /// Take-profit bps of entry mid (0 = off).
    pub tp_bps: f64,
    /// Stop-loss bps of entry mid (0 = off).
    pub sl_bps: f64,
    /// Limit (maker) vs market (taker) entry.
    pub use_limit: bool,
    /// Direction source (Real / Shuffled control).
    pub signal: Signal,
    /// Seed for the shuffled control.
    pub seed: u64,
    /// Fill timeout (ns).
    pub fill_timeout_ns: u64,
    /// Regime entry gate (Any = no gate).
    pub regime_filter: RegimeFilter,
}

impl Default for BasisConfig {
    fn default() -> Self {
        Self {
            top_n: 5,
            threshold_bps: 8.0,
            window: 500,
            sample_ns: 500_000_000,
            reversion: true,
            qty: Qty::from_raw(1_000_000),
            hold_ns: 120_000_000_000,
            cooldown_ns: 5_000_000_000,
            tp_bps: 0.0,
            sl_bps: 0.0,
            use_limit: false,
            signal: Signal::Real,
            seed: 1,
            fill_timeout_ns: 200_000_000,
            regime_filter: RegimeFilter::Any,
        }
    }
}

/// The basis-reversion entry signal (direction only; execution in the shell).
pub struct BasisSignal {
    top_n: usize,
    threshold_bps: f64,
    window: usize,
    sample_ns: u64,
    reversion: bool,
    signal: Signal,
    rng: u64,
    ref_px: f64,
    ring: Vec<f64>,
    pos: usize,
    sum: f64,
    next_sample: u64,
    started: bool,
    cur_gap: f64,
    have_gap: bool,
}

impl BasisSignal {
    fn coin(&mut self) -> bool {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) & 1 == 1
    }

    /// Depth-weighted HL microprice over the top `top_n` levels each side.
    fn micro(&self, ctx: &Ctx) -> Option<f64> {
        let (bb, _) = ctx.book.best_bid()?;
        let (ba, _) = ctx.book.best_ask()?;
        let bb = bb.to_f64();
        let ba = ba.to_f64();
        let bq: f64 = ctx.book.bids_iter().take(self.top_n).map(|(_, q)| q.raw() as f64).sum();
        let aq: f64 = ctx.book.asks_iter().take(self.top_n).map(|(_, q)| q.raw() as f64).sum();
        let tot = bq + aq;
        if tot <= 0.0 {
            return Some((bb + ba) / 2.0);
        }
        Some((bb * aq + ba * bq) / tot)
    }

    fn push_sample(&mut self, g: f64) {
        if self.ring.len() < self.window {
            self.ring.push(g);
            self.sum += g;
        } else {
            let old = self.ring[self.pos];
            self.sum += g - old;
            self.ring[self.pos] = g;
            self.pos = (self.pos + 1) % self.window;
        }
    }
}

impl EntrySignal for BasisSignal {
    fn observe(&mut self, ctx: &Ctx) {
        if ctx.event.kind == EventKind::Trade {
            let p = ctx.event.price.to_f64();
            if p > 0.0 {
                self.ref_px = p;
            }
        }
        if self.ref_px > 0.0 {
            if let Some(m) = self.micro(ctx) {
                self.cur_gap = (m - self.ref_px) / self.ref_px * 10_000.0;
                self.have_gap = true;
            }
        }
        if self.have_gap {
            let now = ctx.now.get();
            if !self.started {
                self.next_sample = now;
                self.started = true;
            }
            if now >= self.next_sample {
                let g = self.cur_gap;
                self.push_sample(g);
                while self.next_sample <= now {
                    self.next_sample += self.sample_ns;
                }
            }
        }
    }

    fn entry(&mut self, _ctx: &Ctx) -> Option<Side> {
        if !self.have_gap || self.ring.len() < 20 {
            return None;
        }
        let base = self.sum / self.ring.len() as f64;
        let dev = self.cur_gap - base;
        if dev.abs() < self.threshold_bps {
            return None;
        }
        let rich = dev > 0.0; // HL above its recent basis
        // reversion: rich -> short (Ask); cheap -> long (Bid). momentum = opposite.
        let mut long = if self.reversion { !rich } else { rich };
        if self.signal == Signal::Shuffled {
            long = self.coin();
        }
        Some(if long { Side::Bid } else { Side::Ask })
    }
}

/// The basis-reversion bot: a [`BasisSignal`] in an `ExecutionShell`.
pub struct BasisBot(ExecutionShell<BasisSignal>);

impl BasisBot {
    /// Build from a sweep config.
    #[must_use]
    pub fn new(cfg: BasisConfig) -> Self {
        let sig = BasisSignal {
            top_n: cfg.top_n.max(1),
            threshold_bps: cfg.threshold_bps,
            window: cfg.window.max(1),
            sample_ns: cfg.sample_ns.max(1),
            reversion: cfg.reversion,
            signal: cfg.signal,
            rng: cfg.seed | 1,
            ref_px: 0.0,
            ring: Vec::new(),
            pos: 0,
            sum: 0.0,
            next_sample: 0,
            started: false,
            cur_gap: 0.0,
            have_gap: false,
        };
        let exec = ExecConfig {
            qty: cfg.qty,
            hold_ns: cfg.hold_ns,
            cooldown_ns: cfg.cooldown_ns,
            tp_bps: cfg.tp_bps,
            sl_bps: cfg.sl_bps,
            use_limit: cfg.use_limit,
            fill_timeout_ns: cfg.fill_timeout_ns,
            regime_filter: cfg.regime_filter,
        };
        Self(ExecutionShell::new(sig, exec))
    }
}

impl Strategy for BasisBot {
    fn on_event(&mut self, ctx: &Ctx, out: &mut Vec<OrderIntent>) {
        self.0.on_event(ctx, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_book::OrderBook;
    use forge_core::{Event, EventKind, Price, UnixNanos};

    fn hl_book(bid: f64, ask: f64) -> OrderBook {
        let mut b = OrderBook::new();
        for (s, px) in [(Side::Bid, bid), (Side::Ask, ask)] {
            b.apply(&Event::new(EventKind::BookDelta, UnixNanos::new(1), UnixNanos::new(1), Some(s), Price::from_f64(px).unwrap(), Qty::from_f64(10.0).unwrap(), 0).unwrap()).unwrap();
        }
        b
    }

    fn trade_ev(px: f64, ts: u64) -> Event {
        Event::new(EventKind::Trade, UnixNanos::new(ts), UnixNanos::new(ts), Some(Side::Bid), Price::from_f64(px).unwrap(), Qty::from_f64(1.0).unwrap(), 0).unwrap()
    }

    #[test]
    fn rich_hl_fades_short() {
        let mut s = BasisSignal {
            top_n: 5, threshold_bps: 5.0, window: 100, sample_ns: 1, reversion: true,
            signal: Signal::Real, rng: 1, ref_px: 0.0, ring: Vec::new(), pos: 0, sum: 0.0,
            next_sample: 0, started: false, cur_gap: 0.0, have_gap: false,
        };
        // Seed a flat basis baseline at ~0, then a rich HL spike.
        let book0 = hl_book(99.99, 100.01); // HL mid ~100 == Binance 100 -> gap ~0
        for ts in 1..200u64 {
            let ev = trade_ev(100.0, ts);
            let ctx = Ctx { now: UnixNanos::new(ts), event: &ev, book: &book0, position_qty: 0 };
            s.observe(&ctx);
        }
        // Now HL jumps rich relative to Binance 100.
        let book1 = hl_book(100.19, 100.21); // HL mid ~100.2 -> +20bps
        let ev = trade_ev(100.0, 1000);
        let ctx = Ctx { now: UnixNanos::new(1000), event: &ev, book: &book1, position_qty: 0 };
        s.observe(&ctx);
        assert_eq!(s.entry(&ctx), Some(Side::Ask), "rich HL -> short (fade)");
    }

    #[test]
    fn small_dev_stands_aside() {
        let mut s = BasisSignal {
            top_n: 5, threshold_bps: 50.0, window: 100, sample_ns: 1, reversion: true,
            signal: Signal::Real, rng: 1, ref_px: 0.0, ring: Vec::new(), pos: 0, sum: 0.0,
            next_sample: 0, started: false, cur_gap: 0.0, have_gap: false,
        };
        let book = hl_book(99.99, 100.01);
        for ts in 1..200u64 {
            let ev = trade_ev(100.0, ts);
            let ctx = Ctx { now: UnixNanos::new(ts), event: &ev, book: &book, position_qty: 0 };
            s.observe(&ctx);
        }
        let ev = trade_ev(100.0, 1000);
        let ctx = Ctx { now: UnixNanos::new(1000), event: &ev, book: &book, position_qty: 0 };
        assert_eq!(s.entry(&ctx), None, "tiny basis dev below threshold -> stand aside");
    }
}