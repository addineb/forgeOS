//! Position tracking and fixed-point P&L accounting.
//!
//! Long/short position with a volume-weighted average entry; realizes gross
//! P&L when a fill reduces or flips the position, and tracks fees separately.
//! Net P&L = realized gross - fees paid. All money is `i128` scaled by
//! `forge_core::SCALE` (see `fills`).

use forge_core::{Side, SCALE};

use crate::fills::{Fill, FeeSchedule, Money};

/// Running account: one position, realized P&L, fees, and per-round-trip nets.
#[derive(Clone, Debug, Default)]
pub struct Account {
    net_qty: i64,         // signed position size (raw qty): + long, - short
    avg_entry: i64,       // average entry price (raw), valid when net_qty != 0
    realized: Money,      // realized GROSS pnl (before fees)
    fees: Money,          // total fees paid (negative = net rebate)
    fills: u64,           // fills applied
    round_trips: u64,     // positions fully closed
    open_fees: Money,     // fees attributed to the currently open position
    trip_pnls: Vec<Money>, // per-round-trip NET pnl (gross - entry/exit fees)
}

impl Account {
    /// A flat, empty account.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply a taker fill on `side` (Bid = bought, Ask = sold).
    pub fn apply_taker(&mut self, side: Side, fill: &Fill, fees: &FeeSchedule) {
        let fee = fees.taker_fee(fill.notional);
        self.apply(side, fill, fee);
    }

    /// Apply a maker fill on `side` (fee may be a rebate).
    pub fn apply_maker(&mut self, side: Side, fill: &Fill, fees: &FeeSchedule) {
        let fee = fees.maker_fee(fill.notional);
        self.apply(side, fill, fee);
    }

    fn apply(&mut self, side: Side, fill: &Fill, fee: Money) {
        self.fees += fee;
        self.fills += 1;
        let delta = match side {
            Side::Bid => fill.filled.raw(),
            Side::Ask => -fill.filled.raw(),
        };
        self.apply_position(delta, fill.avg_price.raw(), fee);
    }

    fn apply_position(&mut self, delta: i64, price: i64, fee: Money) {
        let old = self.net_qty;

        if old == 0 {
            self.net_qty = delta;
            self.avg_entry = price;
            self.open_fees = fee;
            return;
        }

        let same_dir = (old > 0) == (delta > 0);
        if same_dir {
            // Increasing the position: volume-weighted new average entry.
            let oa = i128::from(old.unsigned_abs()) * i128::from(self.avg_entry);
            let da = i128::from(delta.unsigned_abs()) * i128::from(price);
            let tot = i128::from(old.unsigned_abs()) + i128::from(delta.unsigned_abs());
            self.avg_entry = ((oa + da) / tot) as i64;
            self.net_qty = old + delta;
            self.open_fees += fee;
            return;
        }

        // Opposite direction: close part or all of the position.
        let closing = old.unsigned_abs().min(delta.unsigned_abs()) as i64;
        let sign_old: i128 = if old > 0 { 1 } else { -1 };
        let pnl =
            sign_old * (i128::from(price) - i128::from(self.avg_entry)) * i128::from(closing)
                / i128::from(SCALE);
        self.realized += pnl;

        let new_qty = old + delta;
        if new_qty == 0 {
            // Fully closed -> one round trip.
            self.round_trips += 1;
            self.trip_pnls.push(pnl - self.open_fees - fee);
            self.net_qty = 0;
            self.avg_entry = 0;
            self.open_fees = 0;
        } else if (new_qty > 0) == (old > 0) {
            // Partially closed, same side remains; entry price unchanged.
            self.net_qty = new_qty;
        } else {
            // Flipped through zero: old fully closed, remainder opens new leg.
            self.round_trips += 1;
            self.trip_pnls.push(pnl - self.open_fees - fee);
            self.net_qty = new_qty;
            self.avg_entry = price;
            self.open_fees = 0;
        }
    }

    /// Signed position size (raw qty): positive long, negative short.
    #[must_use]
    pub fn net_qty(&self) -> i64 {
        self.net_qty
    }

    /// Realized gross P&L (before fees), scaled money.
    #[must_use]
    pub fn realized(&self) -> Money {
        self.realized
    }

    /// Total fees paid (negative = net rebate), scaled money.
    #[must_use]
    pub fn fees(&self) -> Money {
        self.fees
    }

    /// Net P&L = realized gross - fees.
    #[must_use]
    pub fn net_pnl(&self) -> Money {
        self.realized - self.fees
    }

    /// Unrealized P&L marking the open position at `mark_raw`.
    #[must_use]
    pub fn unrealized(&self, mark_raw: i64) -> Money {
        if self.net_qty == 0 {
            return 0;
        }
        let sign: i128 = if self.net_qty > 0 { 1 } else { -1 };
        sign * (i128::from(mark_raw) - i128::from(self.avg_entry))
            * i128::from(self.net_qty.unsigned_abs())
            / i128::from(SCALE)
    }

    /// Number of fills applied.
    #[must_use]
    pub fn fills(&self) -> u64 {
        self.fills
    }

    /// Number of completed round trips.
    #[must_use]
    pub fn round_trips(&self) -> u64 {
        self.round_trips
    }

    /// Per-round-trip NET P&L (gross minus that trip entry+exit fees).
    #[must_use]
    pub fn trip_pnls(&self) -> &[Money] {
        &self.trip_pnls
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fills::{money_to_f64, Fill};
    use forge_core::{Price, Qty, Side};

    fn fill(px: f64, qty: f64) -> Fill {
        let price = Price::from_f64(px).unwrap();
        let q = Qty::from_f64(qty).unwrap();
        let notional = crate::fills::notional_money(price.raw(), q.raw());
        Fill { avg_price: price, filled: q, notional, levels: 1, complete: true }
    }

    #[test]
    fn long_round_trip_profit_minus_fees() {
        let fees = FeeSchedule::legacy();
        let mut a = Account::new();
        a.apply_taker(Side::Bid, &fill(100.0, 1.0), &fees); // buy 1 @100
        a.apply_taker(Side::Ask, &fill(110.0, 1.0), &fees); // sell 1 @110
        // gross = +10; fees = 0.025% of (100 + 110) = 0.025 + 0.0275 = 0.0525
        assert!((money_to_f64(a.realized()) - 10.0).abs() < 1e-6);
        assert!((money_to_f64(a.fees()) - 0.0525).abs() < 1e-6);
        assert!((money_to_f64(a.net_pnl()) - (10.0 - 0.0525)).abs() < 1e-6);
        assert_eq!(a.round_trips(), 1);
        assert!(money_to_f64(a.trip_pnls()[0]) > 0.0);
        assert_eq!(a.net_qty(), 0);
    }

    #[test]
    fn short_round_trip_profit() {
        let fees = FeeSchedule::zero();
        let mut a = Account::new();
        a.apply_taker(Side::Ask, &fill(110.0, 1.0), &fees); // short 1 @110
        a.apply_taker(Side::Bid, &fill(100.0, 1.0), &fees); // cover @100
        assert!((money_to_f64(a.realized()) - 10.0).abs() < 1e-6); // shorted high, bought low
        assert_eq!(a.round_trips(), 1);
    }

    #[test]
    fn round_trip_same_price_loses_exactly_the_fees() {
        // THE null-edge mechanic at a single-trade level: no price move, so the
        // only thing that happens is paying fees twice -> guaranteed net loss.
        let fees = FeeSchedule::legacy();
        let mut a = Account::new();
        a.apply_taker(Side::Bid, &fill(100.0, 1.0), &fees);
        a.apply_taker(Side::Ask, &fill(100.0, 1.0), &fees);
        assert_eq!(money_to_f64(a.realized()), 0.0);
        assert!(a.net_pnl() < 0, "a flat round trip must lose the fees");
        assert!((money_to_f64(a.net_pnl()) + 0.05).abs() < 1e-6); // -(0.025+0.025)
    }

    #[test]
    fn averaging_in_then_closing() {
        let fees = FeeSchedule::zero();
        let mut a = Account::new();
        a.apply_taker(Side::Bid, &fill(100.0, 1.0), &fees);
        a.apply_taker(Side::Bid, &fill(102.0, 1.0), &fees); // avg entry 101
        a.apply_taker(Side::Ask, &fill(105.0, 2.0), &fees); // close 2 @105
        // gross = (105-101)*2 = 8
        assert!((money_to_f64(a.realized()) - 8.0).abs() < 1e-6);
        assert_eq!(a.net_qty(), 0);
        assert_eq!(a.round_trips(), 1);
    }
}