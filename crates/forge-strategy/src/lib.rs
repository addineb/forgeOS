//! `forge-strategy` - the strategy layer: pure flow primitives and the thesis
//! strategies built on them. Strategies implement `forge_sim::Strategy` and are
//! pure functions of `(event, book, position, now)` - no clock, no I/O, no
//! unseeded randomness - so two runs over the same stream are identical.

#![forbid(unsafe_code)]

mod absorption;
mod basis;
mod cvd;
mod imbalance;
mod momentum;
mod ofi;
mod wallflow;

pub use absorption::{Absorption, AbsorptionBot, AbsorptionConfig, AbsorptionSignal};
pub use basis::{BasisBot, BasisConfig, BasisSignal};
pub use cvd::{Cvd, CvdBot, CvdConfig, CvdSignal};
pub use imbalance::{ImbalanceConfig, ObiBot, ObiSignal};
pub use momentum::{MomentumConfig, OfiMomentum, OfiSignal, Signal};
pub use ofi::Ofi;
pub use wallflow::{WallFlow, WallFlowBot, WallFlowConfig, WallFlowSignal};
pub use forge_sim::{EfficiencyRatio, Regime, RegimeConfig, RegimeFilter};