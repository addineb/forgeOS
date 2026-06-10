//! `forge-strategy` - the strategy layer: pure flow primitives and the thesis
//! strategies built on them. Strategies implement `forge_sim::Strategy` and are
//! pure functions of `(event, book, position, now)` - no clock, no I/O, no
//! unseeded randomness - so two runs over the same stream are identical.

#![forbid(unsafe_code)]

mod imbalance;
mod momentum;
mod ofi;

pub use imbalance::{ImbalanceConfig, ObiBot, ObiSignal};
pub use momentum::{MomentumConfig, OfiMomentum, OfiSignal, Signal};
pub use ofi::Ofi;