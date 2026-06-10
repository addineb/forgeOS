//! [`ForgeWriter`]: emit a validated, time-sorted `*.forge` stream and stamp it
//! with an event count + checksum for the round-trip GATE.

use std::io::{self, Write};

use bytemuck::bytes_of;
use forge_core::{Event, ForgeError};

use crate::checksum::Fnv1a64;
use crate::record::{EventRecord, RECORD_SIZE};

/// Error spanning I/O failures and ForgeOS validation failures.
#[derive(Debug)]
pub enum DataError {
    /// An underlying I/O error (open, read, write, mmap).
    Io(io::Error),
    /// A ForgeOS data-integrity / validation error.
    Forge(ForgeError),
    /// A parse/decoding error from an input source (e.g. parquet).
    Parse(String),
}

impl std::fmt::Display for DataError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DataError::Io(e) => write!(f, "io error: {e}"),
            DataError::Forge(e) => write!(f, "forge error: {e}"),
            DataError::Parse(m) => write!(f, "parse error: {m}"),
        }
    }
}

impl std::error::Error for DataError {}

impl From<io::Error> for DataError {
    fn from(e: io::Error) -> Self {
        DataError::Io(e)
    }
}

impl From<ForgeError> for DataError {
    fn from(e: ForgeError) -> Self {
        DataError::Forge(e)
    }
}

/// Metadata describing a finalized stream (consumed by the GATE).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StreamMeta {
    /// Number of records written.
    pub count: u64,
    /// FNV-1a 64-bit checksum over all record bytes.
    pub checksum: u64,
    /// Total bytes written (`count * RECORD_SIZE`).
    pub bytes: u64,
}

/// Streaming writer for `*.forge` files.
pub struct ForgeWriter<W: Write> {
    inner: W,
    count: u64,
    bytes: u64,
    last_local_ts: Option<u64>,
    hasher: Fnv1a64,
}

impl<W: Write> ForgeWriter<W> {
    /// Wrap a sink (file, buffer) in a fresh writer.
    pub fn new(inner: W) -> Self {
        Self {
            inner,
            count: 0,
            bytes: 0,
            last_local_ts: None,
            hasher: Fnv1a64::new(),
        }
    }

    /// Validate and append one event. Enforces non-decreasing `local_ts`.
    ///
    /// # Errors
    /// [`DataError::Forge`] with [`ForgeError::NonMonotonicTs`] if `local_ts`
    /// goes backwards; [`DataError::Io`] on a write failure.
    pub fn write_event(&mut self, ev: &Event) -> Result<(), DataError> {
        let lt = ev.local_ts.get();
        if let Some(prev) = self.last_local_ts {
            if lt < prev {
                return Err(DataError::Forge(ForgeError::NonMonotonicTs { prev, got: lt }));
            }
        }
        let rec = EventRecord::from_event(ev);
        let bytes = bytes_of(&rec);
        debug_assert_eq!(bytes.len(), RECORD_SIZE);
        self.inner.write_all(bytes)?;
        self.hasher.update(bytes);
        self.count += 1;
        self.bytes += bytes.len() as u64;
        self.last_local_ts = Some(lt);
        Ok(())
    }

    /// Flush and return the stream metadata (count + checksum + byte length).
    ///
    /// # Errors
    /// [`DataError::Io`] if the final flush fails.
    pub fn finish(mut self) -> Result<StreamMeta, DataError> {
        self.inner.flush()?;
        Ok(StreamMeta {
            count: self.count,
            checksum: self.hasher.finish(),
            bytes: self.bytes,
        })
    }
}