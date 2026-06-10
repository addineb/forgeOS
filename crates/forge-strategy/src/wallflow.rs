//! Real-vs-fake WALL classification. A resting "wall" can vanish two opposite
//! ways and they mean opposite things: EATEN (aggressive trades went through it
//! = real liquidity) vs PULLED (cancelled, no trades = a spoof/fake). We can
//! tell them apart because the feed gives BOTH the book (resting size) and the
//! trade tape (price/size/aggressor). When a touch level shrinks, we match the
//! drop against trades that just hit that price: the matched part is EXECUTED,
//! the rest is CANCELLED. Heavy cancellation = the level was fake.
//!
//! See docs/research/real-vs-fake-wall.md.

use std::collections::VecDeque;

use forge_core::{EventKind, Qty, Side};
use forge_sim::{Ctx, EntrySignal, ExecConfig, ExecutionShell, OrderIntent, RegimeFilter, Strategy};

use crate::momentum::Signal;

/// Rolling executed-vs-cancelled tally at the touch of each side.
pub struct WallFlow {
    window: usize,
    wall_min: i64,
    last_bid: Option<(i64, i64)>, // (price_raw, qty_raw)
    last_ask: Option<(i64, i64)>,
    sell_at_bid: i64, // trade volume hitting the bid since the last book change
    buy_at_ask: i64,  // trade volume lifting the ask since the last book change
    hist: VecDeque<(i64, i64, i64, i64)>, // per-event (cancel_bid, cancel_ask, exec_bid, exec_ask)
    cb: i64,
    ca: i64,
    eb: i64,
    ea: i64,
}

impl WallFlow {
    /// New tracker over the last `window` flow events; only levels with resting
    /// size >= `wall_min` (raw) are treated as walls.
    #[must_use]
    pub fn new(window: usize, wall_min: i64) -> Self {
        Self {
            window: window.max(1),
            wall_min: wall_min.max(0),
            last_bid: None,
            last_ask: None,
            sell_at_bid: 0,
            buy_at_ask: 0,
            hist: VecDeque::new(),
            cb: 0,
            ca: 0,
            eb: 0,
            ea: 0,
        }
    }

    /// Fold one trade: a sell (Ask aggressor) at/below the bid consumes bid
    /// liquidity; a buy (Bid aggressor) at/above the ask consumes ask liquidity.
    pub fn observe_trade(&mut self, aggressor: Side, price: i64, qty: i64) {
        match aggressor {
            Side::Ask => {
                if let Some((bpx, _)) = self.last_bid {
                    if price <= bpx {
                        self.sell_at_bid += qty;
                    }
                }
            }
            Side::Bid => {
                if let Some((apx, _)) = self.last_ask {
                    if price >= apx {
                        self.buy_at_ask += qty;
                    }
                }
            }
        }
    }

    /// Fold the current touch. A drop at an unchanged wall price is split into
    /// executed (matched by recent trades) vs cancelled (the residual).
    pub fn observe_book(&mut self, best_bid: Option<(i64, i64)>, best_ask: Option<(i64, i64)>) {
        let (mut dcb, mut dca, mut deb, mut dea) = (0i64, 0i64, 0i64, 0i64);

        if let (Some((px, q)), Some((lpx, lq))) = (best_bid, self.last_bid) {
            if px == lpx && q < lq && lq >= self.wall_min {
                let drop = lq - q;
                let exec = drop.min(self.sell_at_bid.max(0));
                self.sell_at_bid -= exec;
                deb += exec;
                dcb += drop - exec;
            } else if px != lpx {
                self.sell_at_bid = 0; // touch moved; stale trade bucket
            }
        }
        if let (Some((px, q)), Some((lpx, lq))) = (best_ask, self.last_ask) {
            if px == lpx && q < lq && lq >= self.wall_min {
                let drop = lq - q;
                let exec = drop.min(self.buy_at_ask.max(0));
                self.buy_at_ask -= exec;
                dea += exec;
                dca += drop - exec;
            } else if px != lpx {
                self.buy_at_ask = 0;
            }
        }

        if dcb != 0 || dca != 0 || deb != 0 || dea != 0 {
            self.hist.push_back((dcb, dca, deb, dea));
            self.cb += dcb;
            self.ca += dca;
            self.eb += deb;
            self.ea += dea;
            while self.hist.len() > self.window {
                if let Some((a, b, c, d)) = self.hist.pop_front() {
                    self.cb -= a;
                    self.ca -= b;
                    self.eb -= c;
                    self.ea -= d;
                }
            }
        }

        self.last_bid = best_bid;
        self.last_ask = best_ask;
    }

    /// True once a full window of flow events has accumulated.
    #[must_use]
    pub fn ready(&self) -> bool {
        self.hist.len() >= self.window
    }

    /// Share of vanished wall size that was cancelled (pulled), in [0, 1].
    #[must_use]
    pub fn cancel_ratio(&self) -> f64 {
        let tot = (self.cb + self.ca + self.eb + self.ea) as f64;
        if tot <= 0.0 {
            0.0
        } else {
            (self.cb + self.ca) as f64 / tot
        }
    }

    /// Pull skew: > 0 = bid support pulled more (bearish); < 0 = ask pulled.
    #[must_use]
    pub fn pull_skew(&self) -> i64 {
        self.cb - self.ca
    }
}

/// One sweep cell for the wall-flow bot.
#[derive(Clone, Copy, Debug)]
pub struct WallFlowConfig {
    /// Minimum resting size (in qty units) for a level to count as a wall.
    pub wall_min: f64,
    /// Rolling window of flow events.
    pub window: usize,
    /// Minimum cancelled-share to act on (0..1).
    pub cancel_ratio_min: f64,
    /// `false` = fade pulled support/resistance; `true` = invert.
    pub reversion: bool,
    /// Trade size.
    pub qty: Qty,
    /// Hold duration (ns).
    pub hold_ns: u64,
    /// Cooldown (ns).
    pub cooldown_ns: u64,
    /// Take-profit bps (0 = off).
    pub tp_bps: f64,
    /// Stop-loss bps (0 = off).
    pub sl_bps: f64,
    /// Market vs limit entry.
    pub use_limit: bool,
    /// Direction source (Real or the Shuffled control).
    pub signal: Signal,
    /// Seed.
    pub seed: u64,
    /// Fill timeout (ns).
    pub fill_timeout_ns: u64,
    /// Only enter in this market regime (Any = no gate).
    pub regime_filter: RegimeFilter,
}

impl Default for WallFlowConfig {
    fn default() -> Self {
        Self {
            wall_min: 5.0,
            window: 50,
            cancel_ratio_min: 0.6,
            reversion: false,
            qty: Qty::from_raw(1_000_000),
            hold_ns: 5_000_000_000,
            cooldown_ns: 1_000_000_000,
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

/// The wall-flow entry signal.
pub struct WallFlowSignal {
    wf: WallFlow,
    cancel_ratio_min: f64,
    reversion: bool,
    signal: Signal,
    rng: u64,
}

impl WallFlowSignal {
    fn coin(&mut self) -> bool {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) & 1 == 1
    }
}

impl EntrySignal for WallFlowSignal {
    fn observe(&mut self, ctx: &Ctx) {
        if ctx.event.kind == EventKind::Trade {
            if let Some(side) = ctx.event.side {
                self.wf.observe_trade(side, ctx.event.price.raw(), ctx.event.qty.raw());
            }
        }
        let bb = ctx.book.best_bid().map(|(p, q)| (p.raw(), q.raw()));
        let ba = ctx.book.best_ask().map(|(p, q)| (p.raw(), q.raw()));
        self.wf.observe_book(bb, ba);
    }

    fn entry(&mut self, _ctx: &Ctx) -> Option<Side> {
        if !self.wf.ready() || self.wf.cancel_ratio() < self.cancel_ratio_min {
            return None;
        }
        let skew = self.wf.pull_skew();
        if skew == 0 {
            return None;
        }
        // bid support pulled (skew > 0) -> floor is fake -> short.
        let mut short = skew > 0;
        if self.reversion {
            short = !short;
        }
        if self.signal == Signal::Shuffled {
            short = self.coin();
        }
        Some(if short { Side::Ask } else { Side::Bid })
    }
}

/// The wall-flow bot: a [`WallFlowSignal`] in an `ExecutionShell`.
pub struct WallFlowBot(ExecutionShell<WallFlowSignal>);

impl WallFlowBot {
    /// Build from a sweep config.
    #[must_use]
    pub fn new(cfg: WallFlowConfig) -> Self {
        let wall_min = Qty::from_f64(cfg.wall_min).map_or(0, |q| q.raw());
        let sig = WallFlowSignal {
            wf: WallFlow::new(cfg.window, wall_min),
            cancel_ratio_min: cfg.cancel_ratio_min,
            reversion: cfg.reversion,
            signal: cfg.signal,
            rng: cfg.seed | 1,
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

impl Strategy for WallFlowBot {
    fn on_event(&mut self, ctx: &Ctx, out: &mut Vec<OrderIntent>) {
        self.0.on_event(ctx, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // wall_min 0 so every level counts; window 1 so one event is ready.
    fn wf() -> WallFlow {
        WallFlow::new(1, 0)
    }

    #[test]
    fn eaten_wall_reads_as_executed() {
        let mut w = wf();
        // establish a bid wall of 100 at price 100
        w.observe_book(Some((100, 100)), Some((101, 100)));
        // a sell trade of 40 hits the bid
        w.observe_trade(Side::Ask, 100, 40);
        // bid shrinks by exactly 40 -> all executed
        w.observe_book(Some((100, 60)), Some((101, 100)));
        assert!(w.cancel_ratio() < 1e-9, "ratio={}", w.cancel_ratio());
    }

    #[test]
    fn pulled_wall_reads_as_cancelled() {
        let mut w = wf();
        w.observe_book(Some((100, 100)), Some((101, 100)));
        // no trades; bid shrinks by 40 -> all cancelled
        w.observe_book(Some((100, 60)), Some((101, 100)));
        assert!((w.cancel_ratio() - 1.0).abs() < 1e-9, "ratio={}", w.cancel_ratio());
    }

    #[test]
    fn pulled_bid_skews_bearish() {
        let mut w = WallFlow::new(4, 0);
        w.observe_book(Some((100, 100)), Some((101, 100)));
        w.observe_book(Some((100, 50)), Some((101, 100))); // bid pulled
        assert!(w.pull_skew() > 0, "skew={}", w.pull_skew());
    }
}