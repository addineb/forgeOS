//! # forge-anomaly
//!
//! Volume-bar anomaly engine for ForgeOS. Detects repetitive behavioral patterns
//! in order-book depth that precede momentum continuations or reversals.
//!
//! ## Two engine modes
//!
//! `EngineConfig::engine_mode` selects between:
//!
//! - **`Legacy`** (default): multivariate Mahalanobis distance + z-score + FDR
//!   + PatternRepeat. Statistical-deviation detector. Kept for A/B comparison.
//! - **`Causal`**: structural order-flow template matching. Reuses
//!   `FeatureExtractor` for feature math; adds a `causal` module with
//!   templates that match ordered multi-bar causal chains.
//!
//! ## Feature vector (per bar, legacy mode)
//!
//! | Index | Feature |
//! |-------|---------|
//! | 0 | Depth-normalized OFI |
//! | 1 | CVD delta (volume-scaled) |
//! | 2 | Depth imbalance |
//! | 3 | Absorption |
//! | 4 | Liquidity vacuum |
//! | 5 | Volume-delta divergence |
//! | 6 | CVD acceleration (volume-scaled) |
//! | 7 | Aggressor ratio |
//! | 8 | Large-print imbalance |
//! | 9 | Trade intensity |
//!
//! ## Quick start
//!
//! ```rust
//! use forge_anomaly::{AnomalyEngine, EngineConfig, VolumeBar};
//!
//! let mut engine = AnomalyEngine::new(EngineConfig::default());
//! let bar = VolumeBar::default();
//! let output = engine.on_bar(&bar);
//! if let Some(signal) = output.signal {
//!     println!("{} conf={:.2} {}", signal.description, signal.confidence, signal.signal_type);
//! }
//! ```

#![forbid(unsafe_code)]

pub mod backtest;
pub mod causal;
pub mod csv;
pub mod detector;
pub mod engine;
pub mod features;
pub mod null_edge;
pub mod pattern;
pub mod prng;
pub mod regime;
pub mod stats;
pub mod types;

pub use backtest::{EvalRow, ForwardReturns, SignalStats, calibrate_expected_move, evaluate};
pub use causal::{
    AbsorptionReversalParams, CausalDirection, CausalRollingBuf, CausalSignal,
    CausalTemplatesConfig, EngineMode, Step, TemplateOutcome,
};
pub use csv::{load_volume_bars, load_volume_bars_with_fwd};
pub use detector::{DetectorError, MahalanobisDetector};
pub use engine::{AnomalyEngine, EngineOutput};
pub use features::FeatureExtractor;
pub use null_edge::NullEdgeGate;
pub use pattern::PatternCounter;
pub use stats::RollingFeatureWindow;
pub use types::{
    AnomalyEvent, AnomalyKind, AnomalySignal, BarFeatures, EngineConfig,
    FeatureVector, SignalDirection, SignalType, VolumeBar,
};

/// Number of microstructure features in the multivariate detection vector.
pub const FEATURE_DIM: usize = 10;