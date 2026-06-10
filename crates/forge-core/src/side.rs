//! [`Side`]: bid/ask, with a packed `u8` encoding where `0` means "not
//! applicable" (e.g. a book snapshot row).

use crate::error::{ForgeError, ForgeResult};

/// Order-book / trade side.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
#[repr(u8)]
pub enum Side {
    /// Bid / buy side.
    Bid = 1,
    /// Ask / sell side.
    Ask = 2,
}

impl Side {
    /// The packed byte for this side (never `0`; `0` encodes "not applicable").
    #[inline]
    #[must_use]
    pub const fn as_u8(self) -> u8 {
        self as u8
    }

    /// Decode a packed side byte: `0` -> `None`, `1` -> `Bid`, `2` -> `Ask`.
    ///
    /// # Errors
    /// [`ForgeError::BadDiscriminant`] for any other byte.
    #[inline]
    pub fn from_u8(b: u8) -> ForgeResult<Option<Self>> {
        match b {
            0 => Ok(None),
            1 => Ok(Some(Side::Bid)),
            2 => Ok(Some(Side::Ask)),
            value => Err(ForgeError::BadDiscriminant { field: "Side", value }),
        }
    }
}

/// Encode an optional side to its packed byte (`None` -> `0`).
#[inline]
#[must_use]
pub fn side_to_u8(side: Option<Side>) -> u8 {
    match side {
        None => 0,
        Some(s) => s.as_u8(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_side_bytes() {
        assert_eq!(side_to_u8(None), 0);
        assert_eq!(side_to_u8(Some(Side::Bid)), 1);
        assert_eq!(side_to_u8(Some(Side::Ask)), 2);
        assert_eq!(Side::from_u8(0).unwrap(), None);
        assert_eq!(Side::from_u8(1).unwrap(), Some(Side::Bid));
        assert_eq!(Side::from_u8(2).unwrap(), Some(Side::Ask));
    }

    #[test]
    fn bad_side_byte_fails() {
        assert!(Side::from_u8(7).is_err());
    }
}