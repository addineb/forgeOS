//! Fixed-point [`Price`] and [`Qty`], integer-backed at a fixed documented
//! scale.
//!
//! No floating point is stored, nor used in any arithmetic that feeds P&L. `f64`
//! appears only at the ingest boundary ([`Price::from_f64`] / [`Qty::from_f64`]),
//! where external feeds deliver decimal values; that conversion is checked and
//! deterministic.

use crate::error::{ForgeError, ForgeResult};
use core::fmt;

/// Fixed-point scale: 10^8 (8 decimal places). One integer unit = 1e-8.
pub const SCALE: i64 = 100_000_000;
/// `SCALE` as `f64`, for the ingest-boundary conversion only.
pub const SCALE_F64: f64 = 100_000_000.0;
/// Number of decimal places implied by [`SCALE`].
pub const SCALE_DECIMALS: u32 = 8;

/// `i64::MAX` as `f64`, used to bound-check before casting.
const I64_MAX_F: f64 = i64::MAX as f64;
/// Tolerance (in scaled units) for the exact-representability check.
const EXACT_EPS: f64 = 1e-6;

/// Scale and round a finite decimal to fixed-point raw units, failing fast on
/// non-finite or out-of-range input.
fn scale_round(v: f64, field: &'static str) -> ForgeResult<i64> {
    if !v.is_finite() {
        return Err(ForgeError::NonFinite { field });
    }
    let scaled = v * SCALE_F64;
    if !scaled.is_finite() || scaled.abs() >= I64_MAX_F {
        return Err(ForgeError::OutOfRange { field });
    }
    Ok(scaled.round() as i64)
}

/// Like [`scale_round`] but additionally rejects values that are not exactly
/// representable at [`SCALE`] (those that would silently round).
fn scale_exact(v: f64, field: &'static str) -> ForgeResult<i64> {
    if !v.is_finite() {
        return Err(ForgeError::NonFinite { field });
    }
    let scaled = v * SCALE_F64;
    if !scaled.is_finite() || scaled.abs() >= I64_MAX_F {
        return Err(ForgeError::OutOfRange { field });
    }
    let rounded = scaled.round();
    if (scaled - rounded).abs() > EXACT_EPS {
        return Err(ForgeError::Inexact { field });
    }
    Ok(rounded as i64)
}

/// A price as a fixed-point count of `1/SCALE` ticks.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(transparent)]
pub struct Price(i64);

impl Price {
    /// Zero price.
    pub const ZERO: Self = Self(0);

    /// Construct from raw fixed-point units.
    #[inline]
    #[must_use]
    pub const fn from_raw(raw: i64) -> Self {
        Self(raw)
    }

    /// The raw fixed-point units.
    #[inline]
    #[must_use]
    pub const fn raw(self) -> i64 {
        self.0
    }

    /// Convert a decimal feed value, rounding to the nearest tick.
    ///
    /// # Errors
    /// [`ForgeError::NonFinite`] on NaN/Inf, [`ForgeError::OutOfRange`] if the
    /// magnitude exceeds the representable range.
    #[inline]
    pub fn from_f64(v: f64) -> ForgeResult<Self> {
        Ok(Self(scale_round(v, "Price")?))
    }

    /// Convert a decimal feed value, failing fast unless it is exactly
    /// representable at [`SCALE`].
    ///
    /// # Errors
    /// As [`Price::from_f64`], plus [`ForgeError::Inexact`] when rounding would
    /// lose precision.
    #[inline]
    pub fn try_from_f64_exact(v: f64) -> ForgeResult<Self> {
        Ok(Self(scale_exact(v, "Price")?))
    }

    /// Lossy conversion back to `f64`, for display/reporting only.
    #[inline]
    #[must_use]
    pub fn to_f64(self) -> f64 {
        self.0 as f64 / SCALE_F64
    }

    /// Checked addition, failing fast on overflow.
    ///
    /// # Errors
    /// [`ForgeError::Overflow`] on `i64` overflow.
    #[inline]
    pub fn checked_add(self, rhs: Self) -> ForgeResult<Self> {
        self.0
            .checked_add(rhs.0)
            .map(Self)
            .ok_or(ForgeError::Overflow { op: "Price::checked_add" })
    }

    /// Checked subtraction, failing fast on overflow.
    ///
    /// # Errors
    /// [`ForgeError::Overflow`] on `i64` overflow.
    #[inline]
    pub fn checked_sub(self, rhs: Self) -> ForgeResult<Self> {
        self.0
            .checked_sub(rhs.0)
            .map(Self)
            .ok_or(ForgeError::Overflow { op: "Price::checked_sub" })
    }
}

impl fmt::Debug for Price {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Price({})", self.to_f64())
    }
}

/// A quantity as a fixed-point count of `1/SCALE` units. Non-negative by
/// domain: a negative size is corrupt data and is rejected at ingest.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(transparent)]
pub struct Qty(i64);

impl Qty {
    /// Zero quantity.
    pub const ZERO: Self = Self(0);

    /// Construct from raw fixed-point units.
    #[inline]
    #[must_use]
    pub const fn from_raw(raw: i64) -> Self {
        Self(raw)
    }

    /// The raw fixed-point units.
    #[inline]
    #[must_use]
    pub const fn raw(self) -> i64 {
        self.0
    }

    /// Convert a decimal feed value, rounding to the nearest unit and rejecting
    /// negatives.
    ///
    /// # Errors
    /// [`ForgeError::NonFinite`] on NaN/Inf, [`ForgeError::OutOfRange`] if out of
    /// range, [`ForgeError::Negative`] if the result is below zero.
    #[inline]
    pub fn from_f64(v: f64) -> ForgeResult<Self> {
        let raw = scale_round(v, "Qty")?;
        if raw < 0 {
            return Err(ForgeError::Negative { field: "Qty", value: raw });
        }
        Ok(Self(raw))
    }

    /// Convert a decimal feed value exactly, rejecting negatives and inexact
    /// values.
    ///
    /// # Errors
    /// As [`Qty::from_f64`], plus [`ForgeError::Inexact`].
    #[inline]
    pub fn try_from_f64_exact(v: f64) -> ForgeResult<Self> {
        let raw = scale_exact(v, "Qty")?;
        if raw < 0 {
            return Err(ForgeError::Negative { field: "Qty", value: raw });
        }
        Ok(Self(raw))
    }

    /// Lossy conversion back to `f64`, for display/reporting only.
    #[inline]
    #[must_use]
    pub fn to_f64(self) -> f64 {
        self.0 as f64 / SCALE_F64
    }

    /// Checked addition, failing fast on overflow.
    ///
    /// # Errors
    /// [`ForgeError::Overflow`] on `i64` overflow.
    #[inline]
    pub fn checked_add(self, rhs: Self) -> ForgeResult<Self> {
        self.0
            .checked_add(rhs.0)
            .map(Self)
            .ok_or(ForgeError::Overflow { op: "Qty::checked_add" })
    }

    /// Checked subtraction, failing fast on overflow or a negative result.
    ///
    /// # Errors
    /// [`ForgeError::Overflow`] on `i64` overflow, [`ForgeError::Negative`] if
    /// the result would be below zero.
    #[inline]
    pub fn checked_sub(self, rhs: Self) -> ForgeResult<Self> {
        let raw = self
            .0
            .checked_sub(rhs.0)
            .ok_or(ForgeError::Overflow { op: "Qty::checked_sub" })?;
        if raw < 0 {
            return Err(ForgeError::Negative { field: "Qty", value: raw });
        }
        Ok(Self(raw))
    }
}

impl fmt::Debug for Qty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Qty({})", self.to_f64())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn price_from_f64_rounds_to_tick() {
        let p = Price::from_f64(100_000.5).unwrap();
        assert_eq!(p.raw(), 10_000_050_000_000); // 100000.5 * 1e8
    }

    #[test]
    fn price_rejects_non_finite() {
        assert!(Price::from_f64(f64::NAN).is_err());
        assert!(Price::from_f64(f64::INFINITY).is_err());
        assert!(Price::from_f64(f64::NEG_INFINITY).is_err());
    }

    #[test]
    fn price_rejects_out_of_range() {
        assert!(Price::from_f64(1e30).is_err());
        assert!(Price::from_f64(-1e30).is_err());
    }

    #[test]
    fn exact_rejects_inexact() {
        // 1e-9 is finer than the 1e-8 scale -> not exactly representable.
        assert!(Price::try_from_f64_exact(0.000_000_001).is_err());
        // exact multiple of the tick is accepted.
        assert!(Price::try_from_f64_exact(1.5).is_ok());
    }

    #[test]
    fn qty_rejects_negative() {
        assert!(Qty::from_f64(-1.0).is_err());
        assert_eq!(Qty::from_f64(0.0).unwrap(), Qty::ZERO);
    }

    #[test]
    fn checked_arithmetic_overflows() {
        assert!(Price::from_raw(i64::MAX).checked_add(Price::from_raw(1)).is_err());
        assert!(Qty::from_raw(0).checked_sub(Qty::from_raw(1)).is_err());
        assert_eq!(
            Price::from_raw(10).checked_add(Price::from_raw(5)).unwrap().raw(),
            15
        );
    }
}