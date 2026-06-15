//! Forge-depth: depth-pattern feature computation for macrostructure analysis.
//!
//! Computes features from full L2 order book depth at macrostructure timescales
//! (15min - 4hr). Built on top of forge-book's OrderBook, this layer extracts
//! depth patterns that the tick-level microstructure studies never examined:
//!
//! - Depth shape (inter-level gaps, volume concentration, wall tracking)
//! - CVD and aggressive flow (cumulative buy vs sell volume)
//! - Absorption at price (large resting volume that holds)
//! - Volume profile (POC, value area, LVN from tick data)
//! - Order lifetime / spoof detection (place-then-cancel patterns)
//! - Multi-timeframe aggregation (rolling windows from 1s to 4hr)
//!
//! All features are computed no-lookahead: only data <= now is used.

pub mod depth;
pub mod cvd;
pub mod volume_profile;
pub mod wall_tracker;
pub mod features;

pub use depth::DepthSnapshot;
pub use cvd::CVD;
pub use volume_profile::VolumeProfile;
pub use wall_tracker::WallTracker;
pub use features::DepthFeatures;