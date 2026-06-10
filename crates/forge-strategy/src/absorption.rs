//! Absorption thesis. When aggressive orders keep HITTING a level but it WON'T
//! break - heavy sell volume trades into the bid yet the bid price holds (or
//! refills) - a large passive buyer is ABSORBING the flow. Trade WITH the
//! absorber (bid absorbed -> long; ask absorbed -> short). The mirror of a
//! sweep: there liquidity vanishes; here it stands firm under fire.
//!
//! Distinct from imbalance (which reads standing size) and wall-flow (which
//! splits a vanished wall into eaten vs pulled): absorption reads aggressive
//! TRADE volume that fails to move the touch price.

use std::collections::VecDeque;

use forge_core::{EventKind, Qty, Side};
use forge_sim::{Ctx, EntrySignal, ExecConfig, ExecutionShell, OrderIntent, RegimeFilter, Strategy};

use crate::momentum::Signal;

/// Rolling tracker of aggressive volume hitting each touch vs whether the touch
/// price held, over a window of trades.
pub struct Absorption {
    window: usize,
    // per trade: (sell_at_bid, buy_at_ask, bid_px, ask_px)
    recs: VecDeque<(i64, i64, i64, i64)>,
    sum_sell: i64,
    sum_buy: i64,
}

impl Absorption {
    /// New tracker over the last `window` trades (>= 1).
    #[must_use]
    pub fn new(window: usize) -> Self {
        Self { window: window.max(1), recs: VecDeque::new(), sum_sell: 0, sum_buy: 0 }
    }

    /// Fold one trade plus the touch at the time: a sell (Ask aggressor) at/below
    /// the bid hits the bid; a buy (Bid aggressor) at/above the ask hits the ask.
    pub fn observe(&mut self, aggressor: Side, price: i64, qty: i64, bid: i64, ask: i64) {
        let (mut s, mut b) = (0i64, 0i64);
        match aggressor {
            Side::Ask => {
                if price <= bid {
                    s = qty;
                }
            }
            Side::Bid => {
                if price >= ask {
                    b = qty;
                }
            }
        }
        self.recs.push_back((s, b, bid, ask));
        self.sum_sell += s;
        self.sum_buy += b;
        while self.recs.len() > self.window {
            if let Some((os, ob, _, _)) = self.recs.pop_front() {
                self.sum_sell -= os;
                self.sum_buy -= ob;
            }
        }
    }

    /// True once a full window of trades has accumulated.
    #[must_use]
    pub fn ready(&self) -> bool {
        self.recs.len() >= self.window
    }

    /// Aggressive sell volume that hit the bid over the window (raw).
    #[must_use]
    pub fn sell_vol(&self) -> i64 {
        self.sum_sell
    }

    /// Aggressive buy volume that hit the ask over the window (raw).
    #[must_use]
    pub fn buy_vol(&self) -> i64 {
        self.sum_buy
    }

    /// The bid held or rose across the window (absorbed the selling).
    #[must_use]
    pub fn bid_held(&self) -> bool {
        match (self.recs.front(), self.recs.back()) {
            (Some(f), Some(b)) => b.2 >= f.2,
            _ => false,
        }
    }

    /// The ask held or fell across the window (absorbed the buying).
    #[must_use]
    pub fn ask_held(&self) -> bool {
        match (self.recs.front(), self.recs.back()) {
            (Some(f), Some(b)) => b.3 <= f.3,
            _ => false,
        }
    }
}

/// One sweep cell for the absorption bot.
#[derive(Clone, Copy, Debug)]
pub struct AbsorptionConfig {
    /// Rolling window in number of trades.
    pub window: usize,
    /// Minimum aggressive volume (qty units) at a touch to call it absorption.
    pub min_vol: f64,
    /// `false` = trade with the absorber; `true` = fade it.
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

impl Default for AbsorptionConfig {
    fn default() -> Self {
        Self {
            window: 100,
            min_vol: 10.0,
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

/// The absorption entry signal.
pub struct AbsorptionSignal {
    abs: Absorption,
    min_vol: i64,
    reversion: bool,
    signal: Signal,
    rng: u64,
}

impl AbsorptionSignal {
    fn coin(&mut self) -> bool {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) & 1 == 1
    }
}

impl EntrySignal for AbsorptionSignal {
    fn observe(&mut self, ctx: &Ctx) {
        if ctx.event.kind == EventKind::Trade {
            if let Some(side) = ctx.event.side {
                if let (Some((bp, _)), Some((ap, _))) = (ctx.book.best_bid(), ctx.book.best_ask()) {
                    self.abs.observe(side, ctx.event.price.raw(), ctx.event.qty.raw(), bp.raw(), ap.raw());
                }
            }
        }
    }

    fn entry(&mut self, _ctx: &Ctx) -> Option<Side> {
        if !self.abs.ready() {
            return None;
        }
        let bid_absorb = self.abs.sell_vol() >= self.min_vol && self.abs.bid_held();
        let ask_absorb = self.abs.buy_vol() >= self.min_vol && self.abs.ask_held();
        // need exactly one side absorbing to take a directional read
        if bid_absorb == ask_absorb {
            return None;
        }
        // bid absorbed (a buyer soaks the selling) -> long.
        let mut long = bid_absorb;
        if self.reversion {
            long = !long;
        }
        if self.signal == Signal::Shuffled {
            long = self.coin();
        }
        Some(if long { Side::Bid } else { Side::Ask })
    }
}

/// The absorption bot: an [`AbsorptionSignal`] in an `ExecutionShell`.
pub struct AbsorptionBot(ExecutionShell<AbsorptionSignal>);

impl AbsorptionBot {
    /// Build from a sweep config.
    #[must_use]
    pub fn new(cfg: AbsorptionConfig) -> Self {
        let min_vol = Qty::from_f64(cfg.min_vol).map_or(0, |q| q.raw());
        let sig = AbsorptionSignal {
            abs: Absorption::new(cfg.window),
            min_vol,
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

impl Strategy for AbsorptionBot {
    fn on_event(&mut self, ctx: &Ctx, out: &mut Vec<OrderIntent>) {
        self.0.on_event(ctx, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heavy_selling_into_a_held_bid_is_bid_absorption() {
        let mut a = Absorption::new(3);
        // bid stays at 100, ask 101; sells keep hitting the bid
        a.observe(Side::Ask, 100, 5, 100, 101);
        a.observe(Side::Ask, 100, 5, 100, 101);
        a.observe(Side::Ask, 100, 5, 100, 101);
        assert!(a.ready());
        assert_eq!(a.sell_vol(), 15);
        assert!(a.bid_held(), "bid did not fall under selling");
    }

    #[test]
    fn falling_bid_is_not_absorption() {
        let mut a = Absorption::new(3);
        a.observe(Side::Ask, 100, 5, 100, 101);
        a.observe(Side::Ask, 99, 5, 99, 100);
        a.observe(Side::Ask, 98, 5, 98, 99); // bid walked down
        assert!(!a.bid_held());
    }

    #[test]
    fn window_rolls_off() {
        let mut a = Absorption::new(2);
        a.observe(Side::Ask, 100, 5, 100, 101);
        a.observe(Side::Ask, 100, 5, 100, 101);
        a.observe(Side::Ask, 100, 5, 100, 101);
        assert_eq!(a.sell_vol(), 10); // only last 2 trades
    }
}