//! The packed, fixed-width `*.forge` record: a `#[repr(C)]` plain-old-data
//! struct safe to reinterpret directly from mapped bytes (zero decode, zero
//! allocation).

use bytemuck::{Pod, Zeroable};
use forge_core::{
    side_to_u8, Event, EventKind, ForgeResult, Price, Qty, Side, UnixNanos,
};

/// On-disk format version. Bump on any layout change; this tag plus the
/// compile-time size assertion make an incompatible layout fail loudly instead
/// of silently misreading.
pub const FORGE_FORMAT_VERSION: u16 = 1;

/// Size in bytes of one packed record. Pinned by a compile-time assertion.
pub const RECORD_SIZE: usize = 40;

/// One packed market event.
///
/// The four 8-byte fields lead (natural alignment), followed by the small tag
/// bytes and explicit zeroed padding. All padding is an explicit field, so
/// records are bit-reproducible and checksummable.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Pod, Zeroable)]
pub struct EventRecord {
    /// Venue timestamp (Unix nanoseconds).
    pub exch_ts: u64,
    /// Earliest-visible timestamp = `exch_ts + feed_latency` (Unix nanoseconds).
    pub local_ts: u64,
    /// Fixed-point price (forge-core `SCALE`).
    pub price: i64,
    /// Fixed-point quantity (forge-core `SCALE`).
    pub qty: i64,
    /// Event-kind discriminant (see `EventKind`); `0` is invalid.
    pub kind: u8,
    /// Side discriminant (`0` = n/a, `1` = bid, `2` = ask).
    pub side: u8,
    /// Strategy-opaque flags bitfield.
    pub flags: u8,
    /// Explicit zeroed padding to a 40-byte, 8-aligned record.
    pub reserved: [u8; 5],
}

// Pin the layout: any change to size/alignment fails the build.
const _: () = assert!(core::mem::size_of::<EventRecord>() == RECORD_SIZE);
const _: () = assert!(core::mem::align_of::<EventRecord>() == 8);

impl EventRecord {
    /// Build a packed record from a validated [`Event`]. Padding is zeroed.
    #[inline]
    #[must_use]
    pub fn from_event(ev: &Event) -> Self {
        Self {
            exch_ts: ev.exch_ts.get(),
            local_ts: ev.local_ts.get(),
            price: ev.price.raw(),
            qty: ev.qty.raw(),
            kind: ev.kind.as_u8(),
            side: side_to_u8(ev.side),
            flags: ev.flags,
            reserved: [0u8; 5],
        }
    }

    /// Decode this record into an [`Event`], failing fast on bad discriminants
    /// or an invalid timestamp relationship.
    ///
    /// # Errors
    /// Propagates [`forge_core::ForgeError`] from discriminant/timestamp checks.
    #[inline]
    pub fn to_event(&self) -> ForgeResult<Event> {
        let kind = EventKind::from_u8(self.kind)?;
        let side = Side::from_u8(self.side)?;
        Event::new(
            kind,
            UnixNanos::new(self.exch_ts),
            UnixNanos::new(self.local_ts),
            side,
            Price::from_raw(self.price),
            Qty::from_raw(self.qty),
            self.flags,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use core::mem::offset_of;

    #[test]
    fn layout_is_pinned() {
        assert_eq!(core::mem::size_of::<EventRecord>(), 40);
        assert_eq!(core::mem::align_of::<EventRecord>(), 8);
        assert_eq!(offset_of!(EventRecord, exch_ts), 0);
        assert_eq!(offset_of!(EventRecord, local_ts), 8);
        assert_eq!(offset_of!(EventRecord, price), 16);
        assert_eq!(offset_of!(EventRecord, qty), 24);
        assert_eq!(offset_of!(EventRecord, kind), 32);
        assert_eq!(offset_of!(EventRecord, side), 33);
        assert_eq!(offset_of!(EventRecord, flags), 34);
        assert_eq!(offset_of!(EventRecord, reserved), 35);
    }

    #[test]
    fn event_record_roundtrip() {
        let ev = Event::new(
            EventKind::Trade,
            UnixNanos::new(1_700_000_000_000_000_000),
            UnixNanos::new(1_700_000_000_002_000_000),
            Some(Side::Bid),
            Price::from_raw(10_000_050_000_000),
            Qty::from_raw(250_000_000),
            0,
        )
        .unwrap();
        let rec = EventRecord::from_event(&ev);
        assert_eq!(rec.reserved, [0u8; 5]);
        assert_eq!(rec.to_event().unwrap(), ev);
    }

    #[test]
    fn bad_kind_byte_fails_decode() {
        let rec = EventRecord {
            exch_ts: 1,
            local_ts: 1,
            price: 0,
            qty: 0,
            kind: 0, // invalid
            side: 0,
            flags: 0,
            reserved: [0u8; 5],
        };
        assert!(rec.to_event().is_err());
    }
}