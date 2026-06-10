//! `forge-core` - fail-fast domain types for ForgeOS.
//!
//! Provides [`UnixNanos`], fixed-point [`Price`]/[`Qty`], [`Side`], and the
//! normalized [`Event`]. No floating point is ever stored; every fallible
//! boundary rejects bad data (NaN/Inf, negative/non-monotonic timestamps,
//! arithmetic overflow) instead of silently coercing it. This is the trust
//! floor the prior TS engine lacked.

#![forbid(unsafe_code)]

pub mod error;
pub mod event;
pub mod fixed;
pub mod side;
pub mod time;

pub use error::{ForgeError, ForgeResult};
pub use event::{Event, EventKind};
pub use fixed::{Price, Qty, SCALE, SCALE_DECIMALS, SCALE_F64};
pub use side::{side_to_u8, Side};
pub use time::UnixNanos;