//! Fail-fast error type for the ForgeOS core and data layers.
//!
//! The prior engine's fatal defect was trusting bad inputs and letting silent
//! coercion/overflow corrupt accounting. Every fallible boundary here returns a
//! [`ForgeError`] that names the offending field, so corruption stops at ingest
//! rather than surfacing later as a fake edge.

use core::fmt;

/// Errors raised at any ForgeOS ingest or arithmetic boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ForgeError {
    /// A non-finite float (NaN / +Inf / -Inf) where a finite value is required.
    NonFinite { field: &'static str },
    /// A negative value where a non-negative value is required.
    Negative { field: &'static str, value: i64 },
    /// Integer / fixed-point arithmetic overflowed or underflowed.
    Overflow { op: &'static str },
    /// A timestamp moved backwards where a non-decreasing sequence is required.
    NonMonotonicTs { prev: u64, got: u64 },
    /// A value falls outside the representable range of its type.
    OutOfRange { field: &'static str },
    /// A value cannot be represented exactly in fixed point (it would silently
    /// round in a way that affects accounting).
    Inexact { field: &'static str },
    /// An unknown discriminant byte was decoded from a packed record.
    BadDiscriminant { field: &'static str, value: u8 },
}

impl fmt::Display for ForgeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ForgeError::NonFinite { field } => {
                write!(f, "non-finite value in field `{field}`")
            }
            ForgeError::Negative { field, value } => {
                write!(f, "negative value {value} in field `{field}`")
            }
            ForgeError::Overflow { op } => write!(f, "arithmetic overflow in `{op}`"),
            ForgeError::NonMonotonicTs { prev, got } => {
                write!(f, "non-monotonic timestamp: prev={prev} got={got}")
            }
            ForgeError::OutOfRange { field } => {
                write!(f, "value out of range in field `{field}`")
            }
            ForgeError::Inexact { field } => {
                write!(f, "value not exactly representable in fixed point in field `{field}`")
            }
            ForgeError::BadDiscriminant { field, value } => {
                write!(f, "bad discriminant {value} in field `{field}`")
            }
        }
    }
}

impl std::error::Error for ForgeError {}

/// Convenience alias for fallible core operations.
pub type ForgeResult<T> = Result<T, ForgeError>;