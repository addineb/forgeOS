//! [`UnixNanos`]: unsigned 64-bit nanoseconds since the Unix epoch.

use crate::error::{ForgeError, ForgeResult};
use core::fmt;

/// Nanoseconds since the Unix epoch (UTC) as an unsigned 64-bit integer.
///
/// Unsigned by construction: a negative timestamp is corrupt data and is
/// rejected at the conversion boundary ([`UnixNanos::from_i64`]).
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[repr(transparent)]
pub struct UnixNanos(u64);

impl UnixNanos {
    /// The zero instant (Unix epoch).
    pub const ZERO: Self = Self(0);

    /// Construct from a raw `u64` nanosecond count.
    #[inline]
    #[must_use]
    pub const fn new(ns: u64) -> Self {
        Self(ns)
    }

    /// Construct from a signed nanosecond count, rejecting negatives.
    ///
    /// # Errors
    /// Returns [`ForgeError::Negative`] if `ns < 0`.
    #[inline]
    pub fn from_i64(ns: i64) -> ForgeResult<Self> {
        if ns < 0 {
            return Err(ForgeError::Negative { field: "UnixNanos", value: ns });
        }
        Ok(Self(ns as u64))
    }

    /// The raw nanosecond count.
    #[inline]
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }

    /// Add a nanosecond delta, failing fast on overflow.
    ///
    /// # Errors
    /// Returns [`ForgeError::Overflow`] on `u64` overflow.
    #[inline]
    pub fn checked_add(self, delta_ns: u64) -> ForgeResult<Self> {
        self.0
            .checked_add(delta_ns)
            .map(Self)
            .ok_or(ForgeError::Overflow { op: "UnixNanos::checked_add" })
    }

    /// Nanoseconds elapsed from `earlier` to `self`, failing fast if negative.
    ///
    /// # Errors
    /// Returns [`ForgeError::Overflow`] if `earlier > self`.
    #[inline]
    pub fn checked_sub(self, earlier: Self) -> ForgeResult<u64> {
        self.0
            .checked_sub(earlier.0)
            .ok_or(ForgeError::Overflow { op: "UnixNanos::checked_sub" })
    }
}

impl fmt::Debug for UnixNanos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "UnixNanos({})", self.0)
    }
}

impl fmt::Display for UnixNanos {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_i64_rejects_negative() {
        assert!(UnixNanos::from_i64(-1).is_err());
        assert_eq!(UnixNanos::from_i64(0).unwrap(), UnixNanos::ZERO);
        assert_eq!(UnixNanos::from_i64(42).unwrap().get(), 42);
    }

    #[test]
    fn checked_add_overflows() {
        assert!(UnixNanos::new(u64::MAX).checked_add(1).is_err());
        assert_eq!(UnixNanos::new(10).checked_add(5).unwrap().get(), 15);
    }

    #[test]
    fn checked_sub_underflows() {
        assert!(UnixNanos::new(5).checked_sub(UnixNanos::new(10)).is_err());
        assert_eq!(UnixNanos::new(10).checked_sub(UnixNanos::new(4)).unwrap(), 6);
    }
}