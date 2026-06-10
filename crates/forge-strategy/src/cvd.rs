//! Cumulative Volume Delta (CVD) thesis. Uses the TRADE tape: aggressive buys
//! (+qty) minus aggressive sells (-qty), summed over a rolling window of trades,
//! normalised by total traded volume -> a net buy/sell pressure ratio in
//! [-1, 1]. The aggressor sign is EXACT from the feed (no Lee-Ready inference,
//! which the research flags as our edge). Follow the pressure (momentum) or fade
//! exhaustion (reversion); both are sweepable.

use std::collections::VecDeque;

use forge_core::{EventKind, Qty, Side};
use forge_sim::{Ctx, EntrySignal, ExecConfig, ExecutionShell, OrderIntent, Strategy};

use crate::momentum::Signal;

/// Rolling-window cumulative volume delta over the trade tape.
pub struct Cvd {
    window: usize,
    signed: VecDeque<i64>,
    sum: i64,
    abs_sum: i64,
}

impl Cvd {
    /// New estimator over the last `window` trades (>= 1).
    #[must_use]
    pub fn new(window: usize) -> Self {
        Self { window: window.max(1), signed: VecDeque::new(), sum: 0, abs_sum: 0 }
    }

    /// Fold one trade: aggressive buy (`Bid`) = +qty, aggressive sell = -qty.
    pub fn observe_trade(&mut self, aggressor: Side, qty_raw: i64) {
        let s = match aggressor {
            Side::Bid => qty_raw,
            Side::Ask => -qty_raw,
        };
        self.signed.push_back(s);
        self.sum += s;
        self.abs_sum += qty_raw.abs();
        if self.signed.len() > self.window {
            if let Some(old) = self.signed.pop_front() {
                self.sum -= old;
                self.abs_sum -= old.abs();
            }
        }
    }

    /// True once a full window of trades has accumulated.
    #[must_use]
    pub fn ready(&self) -> bool {
        self.signed.len() >= self.window
    }

    /// Net buy/sell pressure in [-1, 1] (signed volume / total volume).
    #[must_use]
    pub fn normalized(&self) -> f64 {
        if self.abs_sum <= 0 {
            return 0.0;
        }
        self.sum as f64 / self.abs_sum as f64
    }
}

/// One sweep cell for the CVD bot.
#[derive(Clone, Copy, Debug)]
pub struct CvdConfig {
    /// Rolling window in number of trades.
    pub window: usize,
    /// Absolute normalised CVD needed to enter.
    pub threshold: f64,
    /// `false` = follow the pressure; `true` = fade it (exhaustion reversal).
    pub reversion: bool,
    /// Trade size.
    pub qty: Qty,
    /// Hold duration in nanoseconds before a timeout exit.
    pub hold_ns: u64,
    /// Cooldown in nanoseconds between trades.
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
}

impl Default for CvdConfig {
    fn default() -> Self {
        Self {
            window: 100,
            threshold: 0.3,
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
        }
    }
}

/// The CVD entry signal.
pub struct CvdSignal {
    cvd: Cvd,
    threshold: f64,
    reversion: bool,
    signal: Signal,
    rng: u64,
}

impl CvdSignal {
    fn coin(&mut self) -> bool {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) & 1 == 1
    }
}

impl EntrySignal for CvdSignal {
    fn observe(&mut self, ctx: &Ctx) {
        if ctx.event.kind == EventKind::Trade {
            if let Some(side) = ctx.event.side {
                self.cvd.observe_trade(side, ctx.event.qty.raw());
            }
        }
    }

    fn entry(&mut self, _ctx: &Ctx) -> Option<Side> {
        if !self.cvd.ready() {
            return None;
        }
        let norm = self.cvd.normalized();
        if norm.abs() < self.threshold {
            return None;
        }
        let buy_pressure = norm > 0.0;
        let mut long = if self.reversion { !buy_pressure } else { buy_pressure };
        if self.signal == Signal::Shuffled {
            long = self.coin();
        }
        Some(if long { Side::Bid } else { Side::Ask })
    }
}

/// The CVD bot: a [`CvdSignal`] in an `ExecutionShell`.
pub struct CvdBot(ExecutionShell<CvdSignal>);

impl CvdBot {
    /// Build from a sweep config.
    #[must_use]
    pub fn new(cfg: CvdConfig) -> Self {
        let sig = CvdSignal {
            cvd: Cvd::new(cfg.window),
            threshold: cfg.threshold,
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
        };
        Self(ExecutionShell::new(sig, exec))
    }
}

impl Strategy for CvdBot {
    fn on_event(&mut self, ctx: &Ctx, out: &mut Vec<OrderIntent>) {
        self.0.on_event(ctx, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buy_pressure_is_positive() {
        let mut c = Cvd::new(4);
        c.observe_trade(Side::Bid, 10);
        c.observe_trade(Side::Bid, 10);
        c.observe_trade(Side::Ask, 5);
        assert!(c.normalized() > 0.0); // net buying
    }

    #[test]
    fn balanced_tape_is_neutral() {
        let mut c = Cvd::new(4);
        c.observe_trade(Side::Bid, 10);
        c.observe_trade(Side::Ask, 10);
        assert!(c.normalized().abs() < 1e-9);
    }

    #[test]
    fn window_rolls_off() {
        let mut c = Cvd::new(2);
        c.observe_trade(Side::Bid, 10);
        c.observe_trade(Side::Bid, 10);
        c.observe_trade(Side::Ask, 10); // window now [+10, -10]
        assert!(c.ready());
        assert!(c.normalized().abs() < 1e-9);
    }
}