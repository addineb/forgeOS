//! The seeded coinflip - the null-edge harness, now just a tiny signal wrapped
//! in the shared [`ExecutionShell`]. It always wants to enter (random
//! direction); the shell handles hold/cooldown/flatten/in-flight safely. In an
//! honest engine its net P&L MUST be negative over a large sample.

use forge_core::{Qty, Side};

use crate::regime::RegimeFilter;
use crate::shell::{EntrySignal, ExecConfig, ExecutionShell};
use crate::strategy::{Ctx, OrderIntent, Strategy};

/// Random-direction entry signal.
pub struct CoinSignal {
    state: u64,
}

impl CoinSignal {
    fn new(seed: u64) -> Self {
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

impl EntrySignal for CoinSignal {
    fn entry(&mut self, _ctx: &Ctx) -> Option<Side> {
        Some(if self.bit() { Side::Bid } else { Side::Ask })
    }
}

/// The coinflip strategy: a [`CoinSignal`] in an [`ExecutionShell`].
pub struct Coinflip(ExecutionShell<CoinSignal>);

impl Coinflip {
    /// Seeded coinflip: trade `qty`, hold `hold_ns` ns, wait `cooldown_ns` ns.
    #[must_use]
    pub fn new(seed: u64, qty: Qty, hold_ns: u64, cooldown_ns: u64) -> Self {
        let cfg = ExecConfig {
            qty,
            hold_ns,
            cooldown_ns,
            tp_bps: 0.0,
            sl_bps: 0.0,
            use_limit: false,
            fill_timeout_ns: 200_000_000,
            regime_filter: RegimeFilter::Any,
        };
        Self(ExecutionShell::new(CoinSignal::new(seed), cfg))
    }
}

impl Strategy for Coinflip {
    fn on_event(&mut self, ctx: &Ctx, out: &mut Vec<OrderIntent>) {
        self.0.on_event(ctx, out);
    }
}