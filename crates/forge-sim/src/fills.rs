//! Fill pricing and fee math, all fixed-point and overflow-safe.
//!
//! Money is tracked as `i128` quote units scaled by `forge_core::SCALE` (1e8),
//! so a notional like 200.00 USDT is `20_000_000_000`. Products use `i128` to
//! avoid the `price_raw * qty_raw` overflow that an `i64` would hit on BTC-sized
//! numbers. No floating point ever touches the money path.
//!
//! Convention: an order `side` is OUR side. `Side::Bid` = we BUY = we lift the
//! offers (consume asks). `Side::Ask` = we SELL = we hit the bids (consume
//! bids). Our orders never move the replayed market; the book is only read to
//! price the fill (walking levels = slippage).

use forge_book::OrderBook;
use forge_core::{Price, Qty, Side, SCALE};

/// Quote currency scaled by `SCALE` (1e8). Signed: profit positive, cost/loss
/// negative where applicable.
pub type Money = i128;

#[inline]
fn scale_i128() -> i128 {
    SCALE as i128
}

/// Notional (quote, scaled) of `qty_raw` filled at `price_raw`.
#[inline]
#[must_use]
pub fn notional_money(price_raw: i64, qty_raw: i64) -> Money {
    i128::from(price_raw) * i128::from(qty_raw) / scale_i128()
}

/// Convert scaled money to a human f64 (display/reporting only).
#[inline]
#[must_use]
pub fn money_to_f64(m: Money) -> f64 {
    m as f64 / SCALE as f64
}

/// Taker/maker fee schedule. Rates are scaled by `SCALE`: 0.025% = 0.00025 is
/// stored as `25_000`. Maker may be negative (a rebate).
#[derive(Clone, Copy, Debug)]
pub struct FeeSchedule {
    /// Taker fee rate, scaled by `SCALE`.
    pub taker_rate_raw: i64,
    /// Maker fee rate, scaled by `SCALE` (negative = rebate).
    pub maker_rate_raw: i64,
}

impl FeeSchedule {
    /// Build a schedule from scaled rates.
    #[must_use]
    pub const fn new(taker_rate_raw: i64, maker_rate_raw: i64) -> Self {
        Self { taker_rate_raw, maker_rate_raw }
    }

    /// The legacy/Hyperliquid-style schedule: taker 0.025%, maker -0.002%.
    #[must_use]
    pub const fn legacy() -> Self {
        Self { taker_rate_raw: 25_000, maker_rate_raw: -2_000 }
    }

    /// Zero fees (for isolating the spread in tests).
    #[must_use]
    pub const fn zero() -> Self {
        Self { taker_rate_raw: 0, maker_rate_raw: 0 }
    }

    /// Taker fee (>= 0 cost) on a notional.
    #[inline]
    #[must_use]
    pub fn taker_fee(&self, notional: Money) -> Money {
        notional * i128::from(self.taker_rate_raw) / scale_i128()
    }

    /// Maker fee on a notional (negative = rebate income).
    #[inline]
    #[must_use]
    pub fn maker_fee(&self, notional: Money) -> Money {
        notional * i128::from(self.maker_rate_raw) / scale_i128()
    }
}

/// The result of pricing a market order against the book.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fill {
    /// Volume-weighted average fill price.
    pub avg_price: Price,
    /// Quantity actually filled (<= requested if the book was too thin).
    pub filled: Qty,
    /// Notional of the fill (quote, scaled).
    pub notional: Money,
    /// Book levels consumed (1 = no slippage beyond the touch).
    pub levels: u32,
    /// Whether the full requested quantity was filled.
    pub complete: bool,
}

/// Price a taker market order of `qty` on `side` by walking the book from the
/// touch. Returns `None` if the relevant side is empty or `qty <= 0`. Does not
/// mutate the book.
#[must_use]
pub fn price_market(book: &OrderBook, side: Side, qty: Qty) -> Option<Fill> {
    let want = qty.raw();
    if want <= 0 {
        return None;
    }
    let mut remaining = want;
    let mut notional: Money = 0;
    let mut filled: i64 = 0;
    let mut levels = 0u32;

    // Buying lifts the asks (ascending); selling hits the bids (descending).
    match side {
        Side::Bid => {
            for (p, q) in book.asks_iter() {
                let take = remaining.min(q.raw());
                if take <= 0 {
                    continue;
                }
                notional += notional_money(p.raw(), take);
                filled += take;
                remaining -= take;
                levels += 1;
                if remaining == 0 {
                    break;
                }
            }
        }
        Side::Ask => {
            for (p, q) in book.bids_iter() {
                let take = remaining.min(q.raw());
                if take <= 0 {
                    continue;
                }
                notional += notional_money(p.raw(), take);
                filled += take;
                remaining -= take;
                levels += 1;
                if remaining == 0 {
                    break;
                }
            }
        }
    }

    if filled == 0 {
        return None;
    }
    let avg_price_raw = (notional * scale_i128() / i128::from(filled)) as i64;
    Some(Fill {
        avg_price: Price::from_raw(avg_price_raw),
        filled: Qty::from_raw(filled),
        notional,
        levels,
        complete: remaining == 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_core::{EventKind, Event, Price, Qty, Side, UnixNanos};

    fn delta(book: &mut OrderBook, side: Side, px: f64, qty: f64) {
        let ev = Event::new(
            EventKind::BookDelta,
            UnixNanos::new(1),
            UnixNanos::new(1),
            Some(side),
            Price::from_f64(px).unwrap(),
            Qty::from_f64(qty).unwrap(),
            0,
        )
        .unwrap();
        book.apply(&ev).unwrap();
    }

    #[test]
    fn notional_and_fee_math() {
        // 2.0 BTC @ 100.0 = 200.0 notional.
        let n = notional_money(Price::from_f64(100.0).unwrap().raw(), Qty::from_f64(2.0).unwrap().raw());
        assert_eq!(money_to_f64(n), 200.0);
        // taker 0.025% of 200 = 0.05.
        let f = FeeSchedule::legacy().taker_fee(n);
        assert!((money_to_f64(f) - 0.05).abs() < 1e-9);
        // maker rebate -0.002% of 200 = -0.004.
        let m = FeeSchedule::legacy().maker_fee(n);
        assert!((money_to_f64(m) + 0.004).abs() < 1e-9);
    }

    #[test]
    fn buy_lifts_asks_with_slippage() {
        let mut b = OrderBook::new();
        delta(&mut b, Side::Ask, 100.0, 1.0);
        delta(&mut b, Side::Ask, 101.0, 5.0);
        delta(&mut b, Side::Bid, 99.0, 5.0);
        // buy 3: 1 @100 + 2 @101 -> vwap = (100 + 202)/3 = 100.6667
        let fill = price_market(&b, Side::Bid, Qty::from_f64(3.0).unwrap()).unwrap();
        assert_eq!(fill.levels, 2);
        assert!(fill.complete);
        assert!((fill.avg_price.to_f64() - (302.0 / 3.0)).abs() < 1e-6);
    }

    #[test]
    fn sell_hits_bids() {
        let mut b = OrderBook::new();
        delta(&mut b, Side::Bid, 99.0, 2.0);
        delta(&mut b, Side::Bid, 98.0, 5.0);
        delta(&mut b, Side::Ask, 100.0, 5.0);
        // sell 1 -> best bid 99
        let fill = price_market(&b, Side::Ask, Qty::from_f64(1.0).unwrap()).unwrap();
        assert_eq!(fill.levels, 1);
        assert!((fill.avg_price.to_f64() - 99.0).abs() < 1e-9);
    }

    #[test]
    fn empty_side_does_not_fill() {
        let mut b = OrderBook::new();
        delta(&mut b, Side::Bid, 99.0, 2.0); // only bids
        assert!(price_market(&b, Side::Bid, Qty::from_f64(1.0).unwrap()).is_none());
    }
}