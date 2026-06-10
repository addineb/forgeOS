//! Deterministic synthetic event generator for the round-trip GATE, so the gate
//! runs in CI without access to the box's parquet feed (Requirement 8.6).

use forge_core::{Event, EventKind, Price, Qty, Side, UnixNanos};

/// A tiny deterministic `xorshift64*` PRNG (no external dependency).
struct XorShift64 {
    state: u64,
}

impl XorShift64 {
    fn new(seed: u64) -> Self {
        // Avoid the all-zero state, which is a fixed point of xorshift.
        Self { state: seed | 1 }
    }

    fn next_u64(&mut self) -> u64 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.state = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

/// Generate `n` deterministic, `local_ts`-monotonic events from `seed`.
///
/// Prices/quantities/kinds/sides vary so the round-trip exercises every field,
/// while `local_ts` is strictly non-decreasing so the writer accepts the stream.
#[must_use]
pub fn generate(seed: u64, n: usize) -> Vec<Event> {
    let mut rng = XorShift64::new(seed);
    let mut out = Vec::with_capacity(n);
    // A fixed epoch base so output is reproducible run to run.
    let mut exch_ts: u64 = 1_700_000_000_000_000_000;
    let feed_latency: u64 = 2_000_000; // 2 ms
    for _ in 0..n {
        // 1ns .. 1ms forward gaps keep exch_ts (and thus local_ts) monotonic.
        exch_ts += 1 + (rng.next_u64() % 1_000_000);
        let local_ts = exch_ts + feed_latency;
        let kind = match rng.next_u64() % 4 {
            0 => EventKind::Trade,
            1 => EventKind::BookDelta,
            2 => EventKind::BookSnapshot,
            _ => EventKind::Quote,
        };
        let side = match rng.next_u64() % 3 {
            0 => None,
            1 => Some(Side::Bid),
            _ => Some(Side::Ask),
        };
        // ~100_000.00 +/- a little, in raw 1e-8 ticks.
        let px_raw = 10_000_000_000_000_i64 + (rng.next_u64() % 1_000_000) as i64;
        let qty_raw = (1 + rng.next_u64() % 5_000_000) as i64;
        let ev = Event::new(
            kind,
            UnixNanos::new(exch_ts),
            UnixNanos::new(local_ts),
            side,
            Price::from_raw(px_raw),
            Qty::from_raw(qty_raw),
            0,
        )
        .expect("synthetic event is valid by construction");
        out.push(ev);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deterministic_for_seed() {
        assert_eq!(generate(42, 100), generate(42, 100));
    }

    #[test]
    fn local_ts_monotonic() {
        let evs = generate(7, 1_000);
        for w in evs.windows(2) {
            assert!(w[1].local_ts >= w[0].local_ts);
        }
    }
}