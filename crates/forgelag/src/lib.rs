//! `forgelag` - a DEDICATED engine for spot-perp basis / cross-venue lag
//! strategies. Built separately from the single-book `forge-sim` so a
//! multi-venue, latency-first thesis is first-class instead of a contortion.
//! Reuses only the proven primitives (`forge-core` fixed-point/time, the
//! `forge-book` order book, `forge-sim` fill pricing/account) - the parts that
//! already pass the null-edge gate - and builds everything basis-specific fresh.

#![forbid(unsafe_code)]

pub mod engine;
pub mod feed;
pub mod strategy;

pub use engine::{LagConfig, LagCtx, LagEngine, LagOrder, LagReport, LagStrategy};
pub use feed::{load_window, FeedConfig, LagEvent, LagKind, Role};
pub use strategy::{BasisConfig, BasisSignal, CoinSignal, LagSignal, Managed, ManagedConfig};
pub use forge_sim::FeeSchedule;