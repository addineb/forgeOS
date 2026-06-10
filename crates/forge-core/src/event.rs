//! The normalized [`Event`]: a flat, fixed-shape value that maps 1:1 onto the
//! packed on-disk `*.forge` record. Every event carries both `exch_ts` (venue
//! time) and `local_ts` (the earliest our strategy could see it).

use crate::error::{ForgeError, ForgeResult};
use crate::fixed::{Price, Qty};
use crate::side::Side;
use crate::time::UnixNanos;

/// Discriminates the event payload. Packed as `u8`; `0` is reserved/invalid so a
/// zeroed record never decodes as a valid event.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[repr(u8)]
pub enum EventKind {
    /// A trade print.
    Trade = 1,
    /// An incremental order-book delta.
    BookDelta = 2,
    /// A periodic order-book snapshot row.
    BookSnapshot = 3,
    /// A top-of-book quote (e.g. Hyperliquid BBO).
    Quote = 4,
}

impl EventKind {
    /// The packed discriminant byte.
    #[inline]
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// Decode a packed kind byte.
    ///
    /// # Errors
    /// [`ForgeError::BadDiscriminant`] for an unknown byte (including `0`).
    #[inline]
    pub fn from_u8(b: u8) -> ForgeResult<Self> {
        match b {
            1 => Ok(EventKind::Trade),
            2 => Ok(EventKind::BookDelta),
            3 => Ok(EventKind::BookSnapshot),
            4 => Ok(EventKind::Quote),
            value => Err(ForgeError::BadDiscriminant { field: "EventKind", value }),
        }
    }
}

/// A normalized market event. Flat by design so the hot replay path is a
/// zero-decode reinterpretation of packed bytes (see `forge-data`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Event {
    /// What kind of event this is.
    pub kind: EventKind,
    /// Venue timestamp.
    pub exch_ts: UnixNanos,
    /// Earliest-visible timestamp (`exch_ts + feed_latency`).
    pub local_ts: UnixNanos,
    /// Side, where applicable (`None` for snapshots/quotes without a side).
    pub side: Option<Side>,
    /// Fixed-point price.
    pub price: Price,
    /// Fixed-point quantity.
    pub qty: Qty,
    /// Strategy-opaque flags bitfield.
    pub flags: u8,
}

impl Event {
    /// Construct an event, validating that `local_ts >= exch_ts` (local time is
    /// venue time plus a non-negative feed latency).
    ///
    /// # Errors
    /// [`ForgeError::NonMonotonicTs`] if `local_ts < exch_ts`.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        kind: EventKind,
        exch_ts: UnixNanos,
        local_ts: UnixNanos,
        side: Option<Side>,
        price: Price,
        qty: Qty,
        flags: u8,
    ) -> ForgeResult<Self> {
        if local_ts < exch_ts {
            return Err(ForgeError::NonMonotonicTs {
                prev: exch_ts.get(),
                got: local_ts.get(),
            });
        }
        Ok(Self { kind, exch_ts, local_ts, side, price, qty, flags })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_byte_roundtrip() {
        for k in [
            EventKind::Trade,
            EventKind::BookDelta,
            EventKind::BookSnapshot,
            EventKind::Quote,
        ] {
            assert_eq!(EventKind::from_u8(k.as_u8()).unwrap(), k);
        }
        assert!(EventKind::from_u8(0).is_err());
        assert!(EventKind::from_u8(9).is_err());
    }

    #[test]
    fn rejects_local_before_exch() {
        let r = Event::new(
            EventKind::Trade,
            UnixNanos::new(100),
            UnixNanos::new(99),
            None,
            Price::ZERO,
            Qty::ZERO,
            0,
        );
        assert!(r.is_err());
    }
}