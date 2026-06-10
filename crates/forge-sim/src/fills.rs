//! Fill pricing and fee math, all fixed-point and overflow-safe.
//!
//! Money is tracked as `i128` quote units scaled by `forge_core::SCALE` (1e8).
//! Products use `i128` to avoid the `price_raw * qty_raw` overflow an `i64`
//! would hit on BTC-sized numbers. No floating point touches the money path.
//!
//! Convention: an order `side` is OUR side. `Side::Bid` = we BUY = lift the
//! offers (consume asks). `Side::Ask` = we SELL = hit the bids (consume bids).
//! Our orders never move the replayed market; the book is only read to price
//! fills (walking levels = slippage).

use forge_book::OrderBook;
use forge_core::{Price, Qty, Side, SCALE};

/// Quote currency scaled by `SCALE` (1e8).
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

/// Convert scaled money to a human f64 (display only).
#[inline]
#[must_use]
pub fn money_to_f64(m: Money) -> f64 {
    m as f64 / SCALE as f64
}

/// Taker/maker fee schedule. Rates scaled by `SCALE`: 0.025% = `25_000`. Maker
/// may be negative (a rebate).
#[derive(Clone, Copy, Debug)]
pub struct FeeSchedule {
    /// Taker fee rate, scaled by `SCALE`.
    pub taker_rate_raw: i64,
    /// Maker fee rate, scaled by `SCALE` (negative = rebate).
    pub maker_rate_raw: i64,
}

impl FeeSchedule {
    /// Build from scaled rates.
    #[must_use]
    pub const fn new(taker_rate_raw: i64, maker_rate_raw: i64) -> Self {
        Self { taker_rate_raw, maker_rate_raw }
    }

    /// Legacy/Hyperliquid-style: taker 0.025%, maker -0.002% (rebate).
    #[must_use]
    pub const fn legacy() -> Self {
        Self { taker_rate_raw: 25_000, maker_rate_raw: -2_000 }
    }

    /// Zero fees (isolate the spread in tests).
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

/// Result of pricing a market/marketable order against the book.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Fill {
    /// Volume-weighted average fill price.
    pub avg_price: Price,
    /// Quantity actually filled.
    pub filled: Qty,
    /// Notional of the fill (quote, scaled).
    pub notional: Money,
    /// Book levels consumed.
    pub levels: u32,
    /// Whether the full requested quantity filled.
    pub complete: bool,
}

/// Price a taker market order by walking the book from the touch (no cap).
#[must_use]
pub fn price_market(book: &OrderBook, side: Side, qty: Qty) -> Option<Fill> {
    price_to_limit(book, side, qty, None)
}

/// Price a fill walking the book from the touch, optionally stopping at a price
/// cap (`limit_raw`): a buy will not pay above the cap, a sell will not accept
/// below it. `cap = None` is a pure market order. Returns `None` if nothing
/// fills.
#[must_use]
pub fn price_to_limit(book: &OrderBook, side: Side, qty: Qty, cap: Option<i64>) -> Option<Fill> {
    let want = qty.raw();
    if want <= 0 {
        return None;
    }
    let mut remaining = want;
    let mut notional: Money = 0;
    let mut filled: i64 = 0;
    let mut levels = 0u32;

    match side {
        Side::Bid => {
            for (p, q) in book.asks_iter() {
                if let Some(c) = cap {
                    if p.raw() > c {
                        break;
                    }
                }
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
                if let Some(c) = cap {
                    if p.raw() < c {
                        break;
                    }
                }
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

/// Build a single-price maker fill (filled at the resting limit price).
#[must_use]
pub fn maker_fill(price: Price, qty: Qty) -> Fill {
    Fill {
        avg_price: price,
        filled: qty,
        notional: notional_money(price.raw(), qty.raw()),
        levels: 1,
        complete: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use forge_core::{Event, EventKind, Price, Qty, Side, UnixNanos};

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
        let n = notional_money(Price::from_f64(100.0).unwrap().raw(), Qty::from_f64(2.0).unwrap().raw());
        assert_eq!(money_to_f64(n), 200.0);
        assert!((money_to_f64(FeeSchedule::legacy().taker_fee(n)) - 0.05).abs() < 1e-9);
        assert!((money_to_f64(FeeSchedule::legacy().maker_fee(n)) + 0.004).abs() < 1e-9);
    }

    #[test]
    fn buy_lifts_asks_with_slippage() {
        let mut b = OrderBook::new();
        delta(&mut b, Side::Ask, 100.0, 1.0);
        delta(&mut b, Side::Ask, 101.0, 5.0);
        delta(&mut b, Side::Bid, 99.0, 5.0);
        let fill = price_market(&b, Side::Bid, Qty::from_f64(3.0).unwrap()).unwrap();
        assert_eq!(fill.levels, 2);
        assert!((fill.avg_price.to_f64() - (302.0 / 3.0)).abs() < 1e-6);
    }

    #[test]
    fn price_cap_stops_the_walk() {
        let mut b = OrderBook::new();
        delta(&mut b, Side::Ask, 100.0, 1.0);
        delta(&mut b, Side::Ask, 101.0, 5.0);
        // buy 3 but cap at 100 -> only 1 fills (the 101 level is beyond the cap)
        let fill = price_to_limit(&b, Side::Bid, Qty::from_f64(3.0).unwrap(), Some(Price::from_f64(100.0).unwrap().raw())).unwrap();
        assert!(!fill.complete);
        assert!((fill.filled.to_f64() - 1.0).abs() < 1e-9);
    }

    #[test]
    fn empty_side_does_not_fill() {
        let mut b = OrderBook::new();
        delta(&mut b, Side::Bid, 99.0, 2.0);
        assert!(price_market(&b, Side::Bid, Qty::from_f64(1.0).unwrap()).is_none());
    }
}