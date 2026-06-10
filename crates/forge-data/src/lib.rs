//! `forge-data` - the ForgeOS hot-path data layer.
//!
//! Defines the packed, fixed-width [`EventRecord`] (`*.forge` on-disk format),
//! a streaming [`ForgeWriter`] that validates ordering and stamps a checksum, a
//! zero-copy [`ForgeReader`] (mmap -> typed slice, no per-event allocation), and
//! an FNV-1a checksum. A deterministic [`synthetic`] generator backs the
//! round-trip GATE so it runs in CI without the box's parquet feed.

pub mod checksum;
#[cfg(feature = "convert")]
pub mod convert;
pub mod reader;
pub mod record;
pub mod synthetic;
pub mod writer;

pub use checksum::{fnv1a64, Fnv1a64};
pub use reader::ForgeReader;
pub use record::{EventRecord, FORGE_FORMAT_VERSION, RECORD_SIZE};
pub use writer::{DataError, ForgeWriter, StreamMeta};