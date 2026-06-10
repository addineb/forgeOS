//! The seeded coinflip strategy - the null-edge harness.
//!
//! It opens a random-direction market position, holds it a fixed number of
//! events, then closes the EXACT size it is holding (position-aware, so partial
//! fills do not let inventory drift), waits a cooldown, and repeats. Direction
//! is a fair coin and the held move is mean-zero, so the only systematic effect
//! is crossing the spread twice plus fees. In an honest engine its net P&L MUST
//! be negative over a large sample. If it ever profits, the engine is lying.

use forge_core::{Qty, Side};

use crate::strategy::{Ctx, OrderIntent, Strategy};

#[derive(Clone, Copy)]
enum Phase {
    /// Flat, counting down a cooldown before the next trade.
    Flat(u32),
    /// In a trade, counting down the hold before closing.
    Open(u32),
}

/// A deterministic, position-aware coinflip taker strategy.
pub struct Coinflip {
    state: u64,
    qty: Qty,
    hold: u32,
    cooldown: u32,
    phase: Phase,
}

impl Coinflip {
    /// Seeded coinflip: trade `qty` each time, hold `hold` events, then wait
    /// `cooldown` events before the next trade.
    #[must_use]
    pub fn new(seed: u64, qty: Qty, hold: u32, cooldown: u32) -> Self {
        Self { state: seed | 1, qty, hold, cooldown, phase: Phase::Flat(0) }
    }

    fn next_bit(&mut self) -> bool {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) & 1 == 1
    }

    /// A market order that closes the current signed position exactly.
    fn flatten(pos: i64) -> OrderIntent {
        let side = if pos > 0 { Side::Ask } else { Side::Bid };
        OrderIntent::market(side, Qty::from_raw(pos.unsigned_abs() as i64))
    }
}

impl Strategy for Coinflip {
    fn on_event(&mut self, ctx: &Ctx, out: &mut Vec<OrderIntent>) {
        let pos = ctx.position_qty;
        match self.phase {
            Phase::Flat(0) => {
                if pos == 0 {
                    let side = if self.next_bit() { Side::Bid } else { Side::Ask };
                    out.push(OrderIntent::market(side, self.qty));
                    self.phase = Phase::Open(self.hold);
                } else {
                    // clear any residual inventory before opening a new trade
                    out.push(Self::flatten(pos));
                }
            }
            Phase::Flat(n) => self.phase = Phase::Flat(n - 1),
            Phase::Open(0) => {
                if pos != 0 {
                    out.push(Self::flatten(pos));
                }
                self.phase = Phase::Flat(self.cooldown);
            }
            Phase::Open(n) => self.phase = Phase::Open(n - 1),
        }
    }
}