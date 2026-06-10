//! OFI-momentum thesis ("ride the push"), now just a signal wrapped in the
//! shared `forge_sim::ExecutionShell`. The signal decides direction from
//! depth-normalised OFI; the shell owns all order/position plumbing (entry,
//! TP/SL/hold exits, cooldown, in-flight guard, flatten). `Shuffled` swaps the
//! OFI sign for a seeded coin (same cadence, no direction) = the no-fake-edge
//! control.

use forge_core::Side;
use forge_core::Qty;
use forge_sim::{Ctx, EntrySignal, ExecConfig, ExecutionShell, OrderIntent, Strategy};

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
    /// Nanoseconds to wait for a fill before assuming it will not happen.
    pub fill_timeout_ns: u64,
}

impl Default for MomentumConfig {
    fn default() -> Self {
        Self {
            ofi_window: 20,
            threshold: 1.0,
            qty: Qty::from_raw(1_000_000),
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

/// The OFI entry signal (direction only; execution lives in the shell).
pub struct OfiSignal {
    ofi: Ofi,
    threshold: f64,
    signal: Signal,
    rng: u64,
}

impl OfiSignal {
    fn coin(&mut self) -> bool {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) & 1 == 1
    }
}

impl EntrySignal for OfiSignal {
    fn observe(&mut self, ctx: &Ctx) {
        if let (Some((bp, bq)), Some((ap, aq))) = (ctx.book.best_bid(), ctx.book.best_ask()) {
            self.ofi.observe(bp.raw(), bq.raw(), ap.raw(), aq.raw());
        }
    }

    fn entry(&mut self, _ctx: &Ctx) -> Option<Side> {
        if !self.ofi.ready() {
            return None;
        }
        let norm = self.ofi.normalized();
        if norm.abs() < self.threshold {
            return None;
        }
        let long = match self.signal {
            Signal::Real => norm > 0.0,
            Signal::Shuffled => self.coin(),
        };
        Some(if long { Side::Bid } else { Side::Ask })
    }
}

/// The OFI-momentum strategy: an [`OfiSignal`] in an `ExecutionShell`.
pub struct OfiMomentum(ExecutionShell<OfiSignal>);

impl OfiMomentum {
    /// Build from a sweep config.
    #[must_use]
    pub fn new(cfg: MomentumConfig) -> Self {
        let sig = OfiSignal {
            ofi: Ofi::new(cfg.ofi_window),
            threshold: cfg.threshold,
            signal: cfg.signal,
            rng: cfg.seed | 1,
        };
        let exec = ExecConfig {
            qty: cfg.qty,
            hold: cfg.hold,
            cooldown: cfg.cooldown,
            tp_bps: cfg.tp_bps,
            sl_bps: cfg.sl_bps,
            use_limit: cfg.use_limit,
            fill_timeout_ns: cfg.fill_timeout_ns,
        };
        Self(ExecutionShell::new(sig, exec))
    }
}

impl Strategy for OfiMomentum {
    fn on_event(&mut self, ctx: &Ctx, out: &mut Vec<OrderIntent>) {
        self.0.on_event(ctx, out);
    }
}