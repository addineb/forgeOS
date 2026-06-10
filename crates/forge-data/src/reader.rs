//! [`ForgeReader`]: a zero-copy `mmap` over a `*.forge` file, exposing the
//! contents as a typed `&[EventRecord]` slice. The per-event scan allocates
//! nothing.

use std::fs::File;
use std::path::Path;

use forge_core::ForgeError;
use memmap2::Mmap;

use crate::checksum::fnv1a64;
use crate::record::{EventRecord, RECORD_SIZE};
use crate::writer::DataError;

/// Read-only memory-mapped view of a `*.forge` stream.
pub struct ForgeReader {
    mmap: Mmap,
}

impl ForgeReader {
    /// Open and memory-map a `*.forge` file read-only.
    ///
    /// # Errors
    /// [`DataError::Io`] if the file cannot be opened/mapped;
    /// [`DataError::Forge`] with [`ForgeError::OutOfRange`] if the length is not
    /// a whole number of records or the mapping is misaligned for the record
    /// type.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self, DataError> {
        let file = File::open(path)?;
        // SAFETY: the mapping is read-only and never mutated; we treat any
        // concurrent external modification of the backing file as out of scope
        // (the converter writes once, then the file is immutable input).
        let mmap = unsafe { Mmap::map(&file)? };

        if !mmap.len().is_multiple_of(RECORD_SIZE) {
            return Err(DataError::Forge(ForgeError::OutOfRange {
                field: "forge-file-length",
            }));
        }
        if !(mmap.as_ptr() as usize).is_multiple_of(core::mem::align_of::<EventRecord>()) {
            return Err(DataError::Forge(ForgeError::OutOfRange {
                field: "forge-file-alignment",
            }));
        }
        Ok(Self { mmap })
    }

    /// The records as a zero-copy typed slice.
    ///
    /// `open` has already validated length and alignment, so the cast cannot
    /// fail.
    #[inline]
    #[must_use]
    pub fn records(&self) -> &[EventRecord] {
        bytemuck::cast_slice(&self.mmap[..])
    }

    /// Number of records in the stream.
    #[inline]
    #[must_use]
    pub fn len(&self) -> usize {
        self.mmap.len() / RECORD_SIZE
    }

    /// Whether the stream is empty.
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// FNV-1a checksum over the raw mapped bytes (matches the writer's stamp).
    #[inline]
    #[must_use]
    pub fn checksum(&self) -> u64 {
        fnv1a64(&self.mmap[..])
    }

    /// The raw mapped bytes.
    #[inline]
    #[must_use]
    pub fn bytes(&self) -> &[u8] {
        &self.mmap[..]
    }
}