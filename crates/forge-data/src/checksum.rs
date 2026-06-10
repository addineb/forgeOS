//! FNV-1a 64-bit checksum over raw record bytes: dependency-free and
//! deterministic. The writer stamps a stream with it; the GATE recomputes it
//! over the read-back bytes to prove the round-trip is lossless.

/// FNV-1a 64-bit offset basis.
const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
/// FNV-1a 64-bit prime.
const PRIME: u64 = 0x0000_0100_0000_01b3;

/// Compute the FNV-1a 64-bit hash of `bytes` in one shot.
#[inline]
#[must_use]
pub fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h = Fnv1a64::new();
    h.update(bytes);
    h.finish()
}

/// Incremental FNV-1a 64-bit hasher for streaming writes.
#[derive(Clone, Copy, Debug)]
pub struct Fnv1a64 {
    state: u64,
}

impl Default for Fnv1a64 {
    #[inline]
    fn default() -> Self {
        Self { state: OFFSET_BASIS }
    }
}

impl Fnv1a64 {
    /// A fresh hasher seeded with the FNV offset basis.
    #[inline]
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Fold `bytes` into the running hash.
    #[inline]
    pub fn update(&mut self, bytes: &[u8]) {
        let mut h = self.state;
        for &b in bytes {
            h ^= u64::from(b);
            h = h.wrapping_mul(PRIME);
        }
        self.state = h;
    }

    /// The final 64-bit digest.
    #[inline]
    #[must_use]
    pub fn finish(self) -> u64 {
        self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_is_offset_basis() {
        assert_eq!(fnv1a64(&[]), OFFSET_BASIS);
    }

    #[test]
    fn incremental_equals_oneshot() {
        let data = b"forgeOS-null-edge";
        let oneshot = fnv1a64(data);
        let mut h = Fnv1a64::new();
        h.update(&data[..5]);
        h.update(&data[5..]);
        assert_eq!(h.finish(), oneshot);
    }

    #[test]
    fn known_vector() {
        // FNV-1a 64 of the single byte 0x00.
        assert_eq!(fnv1a64(&[0x00]), 0xaf63bd4c8601b7df);
    }
}