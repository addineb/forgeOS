//! The seeded coinflip strategy - the null-edge harness.
//!
//! Opens a random-direction market position, holds it a fixed number of events,
//! closes the exact size held, waits a cooldown, repeats. Direction is a fair
//! coin and the held move is mean-zero, so the only systematic effect is
//! crossing the spread twice plus fees. In an honest engine its net P&L MUST be
//! negative over a large sample. If it ever profits, the engine is lying.
//!
//! It keeps at most ONE order in flight: after sending an order it waits until
//! it sees its position change (or a time deadline) before acting again, so it
//! never spams orders while a fill is in flight (events can share a timestamp).

use forge_core::{Qty, Side};

use crate::strategy::{Ctx, OrderIntent, Strategy};

/// How long to wait for a fill before assuming the order will not fill.
const FILL_TIMEOUT_NS: u64 = 200_000_000; // 200 ms

#[derive(Clone, Copy)]
enum Phase {
    Flat(u32),
    Open(u32),
}

/// A deterministic, position-aware coinflip taker strategy.
pub struct Coinflip {
    state: u64,
    qty: Qty,
    hold: u32,
    cooldown: u32,
    pending: Option<(i64, u64)>, // (position when order sent, deadline ns)
    phase: Phase,
}

impl Coinflip {
    /// Seeded coinflip: trade `qty`, hold `hold` events, wait `cooldown` events.
    #[must_use]
    pub fn new(seed: u64, qty: Qty, hold: u32, cooldown: u32) -> Self {
        Self { state: seed | 1, qty, hold, cooldown, pending: None, phase: Phase::Flat(0) }
    }

    fn next_bit(&mut self) -> bool {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) & 1 == 1
    }

    fn flatten(pos: i64) -> OrderIntent {
        let side = if pos > 0 { Side::Ask } else { Side::Bid };
        OrderIntent::market(side, Qty::from_raw(pos.unsigned_abs() as i64))
    }
}

impl Strategy for Coinflip {
    fn on_event(&mut self, ctx: &Ctx, out: &mut Vec<OrderIntent>) {
        let pos = ctx.position_qty;
        let now = ctx.now.get();

        // Resolve any in-flight order: cleared once the position changes or the
        // deadline passes.
        if let Some((sent_pos, deadline)) = self.pending {
            if pos != sent_pos || now >= deadline {
                self.pending = None;
            }
        }
        // One order at a time: while waiting on a fill, do nothing.
        if self.pending.is_some() {
            return;
        }

        match self.phase {
            Phase::Flat(0) => {
                if pos == 0 {
                    // only open when the book is two-sided, so the taker fill
                    // cannot trivially reject (and stall on the fill timeout)
                    if ctx.book.best_bid().is_some() && ctx.book.best_ask().is_some() {
                        let side = if self.next_bit() { Side::Bid } else { Side::Ask };
                        out.push(OrderIntent::market(side, self.qty));
                        self.pending = Some((pos, now + FILL_TIMEOUT_NS));
                        self.phase = Phase::Open(self.hold);
                    }
                } else {
                    out.push(Self::flatten(pos));
                    self.pending = Some((pos, now + FILL_TIMEOUT_NS));
                    self.phase = Phase::Flat(self.cooldown);
                }
            }
            Phase::Flat(n) => self.phase = Phase::Flat(n - 1),
            Phase::Open(0) => {
                if pos != 0 {
                    out.push(Self::flatten(pos));
                    self.pending = Some((pos, now + FILL_TIMEOUT_NS));
                }
                self.phase = Phase::Flat(self.cooldown);
            }
            Phase::Open(n) => self.phase = Phase::Open(n - 1),
        }
    }
}