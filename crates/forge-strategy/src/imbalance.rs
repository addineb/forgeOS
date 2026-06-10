//! Order-book imbalance / wall thesis. Distinct from OFI (which measures the
//! CHANGE in resting size); this reads the standing LEVELS - where the walls
//! are. Imbalance over the top-N levels:
//!   imb = (sum_bid_qty - sum_ask_qty) / (sum_bid_qty + sum_ask_qty)  in [-1, 1]
//! A bid-heavy book (imb > 0) is a buy-side wall / support; ask-heavy is the
//! mirror. The `reversion` knob captures the legacy wall ambiguity: follow the
//! wall (imb > 0 -> long) or fade it (the absorption read: imb > 0 -> short).
//! Both directions are sweepable so the data decides.

use forge_core::{Qty, Side};
use forge_sim::{Ctx, EntrySignal, ExecConfig, ExecutionShell, OrderIntent, RegimeFilter, Strategy};

use crate::momentum::Signal;

/// One sweep cell for the imbalance bot.
#[derive(Clone, Copy, Debug)]
pub struct ImbalanceConfig {
    /// Levels per side to sum for the imbalance.
    pub top_n: usize,
    /// Absolute imbalance needed to enter.
    pub threshold: f64,
    /// `true` = fade the wall (imb>0 -> short); `false` = follow it.
    pub reversion: bool,
    /// Trade size.
    pub qty: Qty,
    /// Hold duration in nanoseconds before a timeout exit.
    pub hold_ns: u64,
    /// Cooldown in nanoseconds between trades.
    pub cooldown_ns: u64,
    /// Take-profit bps of entry mid (0 = off).
    pub tp_bps: f64,
    /// Stop-loss bps of entry mid (0 = off).
    pub sl_bps: f64,
    /// Limit (maker) vs market (taker) entry.
    pub use_limit: bool,
    /// Direction source (Real or the Shuffled control).
    pub signal: Signal,
    /// Seed for the shuffled control.
    pub seed: u64,
    /// Fill timeout (ns).
    pub fill_timeout_ns: u64,
    /// Only enter in this market regime (Any = no gate).
    pub regime_filter: RegimeFilter,
}

impl Default for ImbalanceConfig {
    fn default() -> Self {
        Self {
            top_n: 5,
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
            regime_filter: RegimeFilter::Any,
        }
    }
}

/// The imbalance entry signal (direction only; execution in the shell).
pub struct ObiSignal {
    top_n: usize,
    threshold: f64,
    reversion: bool,
    signal: Signal,
    rng: u64,
}

impl ObiSignal {
    fn coin(&mut self) -> bool {
        let mut x = self.rng;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.rng = x;
        (x.wrapping_mul(0x2545_F491_4F6C_DD1D) >> 33) & 1 == 1
    }
}

impl EntrySignal for ObiSignal {
    fn entry(&mut self, ctx: &Ctx) -> Option<Side> {
        let bid: i64 = ctx.book.bids_iter().take(self.top_n).map(|(_, q)| q.raw()).sum();
        let ask: i64 = ctx.book.asks_iter().take(self.top_n).map(|(_, q)| q.raw()).sum();
        let tot = bid + ask;
        if tot <= 0 {
            return None;
        }
        let imb = (bid - ask) as f64 / tot as f64;
        if imb.abs() < self.threshold {
            return None;
        }
        let bid_heavy = imb > 0.0;
        // follow: bid-heavy -> long. reversion (fade the wall): bid-heavy -> short.
        let mut long = if self.reversion { !bid_heavy } else { bid_heavy };
        if self.signal == Signal::Shuffled {
            long = self.coin();
        }
        Some(if long { Side::Bid } else { Side::Ask })
    }
}

/// The order-book-imbalance / wall bot: an [`ObiSignal`] in an `ExecutionShell`.
pub struct ObiBot(ExecutionShell<ObiSignal>);

impl ObiBot {
    /// Build from a sweep config.
    #[must_use]
    pub fn new(cfg: ImbalanceConfig) -> Self {
        let sig = ObiSignal {
            top_n: cfg.top_n.max(1),
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
            regime_filter: cfg.regime_filter,
        };
        Self(ExecutionShell::new(sig, exec))
    }
}

impl Strategy for ObiBot {
    fn on_event(&mut self, ctx: &Ctx, out: &mut Vec<OrderIntent>) {
        self.0.on_event(ctx, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_book::OrderBook;
    use forge_core::{Event, EventKind, Price, Qty, UnixNanos};

    fn book_with(bid_qty: f64, ask_qty: f64) -> OrderBook {
        let mut b = OrderBook::new();
        for (side, px, q) in [
            (Side::Bid, 100.0, bid_qty),
            (Side::Ask, 101.0, ask_qty),
        ] {
            b.apply(
                &Event::new(
                    EventKind::BookDelta,
                    UnixNanos::new(1),
                    UnixNanos::new(1),
                    Some(side),
                    Price::from_f64(px).unwrap(),
                    Qty::from_f64(q).unwrap(),
                    0,
                )
                .unwrap(),
            )
            .unwrap();
        }
        b
    }

    #[test]
    fn follow_goes_long_when_bid_heavy() {
        let mut s = ObiSignal { top_n: 5, threshold: 0.3, reversion: false, signal: Signal::Real, rng: 1 };
        let book = book_with(10.0, 1.0); // bid-heavy
        let ctx = Ctx { now: UnixNanos::new(1), event: &dummy(), book: &book, position_qty: 0 };
        assert_eq!(s.entry(&ctx), Some(Side::Bid));
    }

    #[test]
    fn reversion_fades_the_wall() {
        let mut s = ObiSignal { top_n: 5, threshold: 0.3, reversion: true, signal: Signal::Real, rng: 1 };
        let book = book_with(10.0, 1.0); // bid-heavy -> fade -> short
        let ctx = Ctx { now: UnixNanos::new(1), event: &dummy(), book: &book, position_qty: 0 };
        assert_eq!(s.entry(&ctx), Some(Side::Ask));
    }

    #[test]
    fn balanced_book_stands_aside() {
        let mut s = ObiSignal { top_n: 5, threshold: 0.3, reversion: false, signal: Signal::Real, rng: 1 };
        let book = book_with(5.0, 5.0);
        let ctx = Ctx { now: UnixNanos::new(1), event: &dummy(), book: &book, position_qty: 0 };
        assert_eq!(s.entry(&ctx), None);
    }

    fn dummy() -> Event {
        Event::new(EventKind::Trade, UnixNanos::new(1), UnixNanos::new(1), None, Price::ZERO, Qty::ZERO, 0).unwrap()
    }
}