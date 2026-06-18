//! Core domain types for the anomaly detection engine.

use forge_core::Side;

/// A single enriched volume-bar snapshot (depthscope-shaped input).
#[derive(Debug, Clone, Copy, Default)]
pub struct VolumeBar {
    /// Bar close timestamp (nanoseconds).
    pub ts: u64,
    /// Monotonic bar index (0-based).
    pub bar_index: u64,
    /// Cumulative traded volume at bar close (base units).
    pub cum_vol: f64,
    /// Volume traded within this bar.
    pub bar_vol: f64,
    /// Mid price at bar close.
    pub mid_price: f64,
    /// Best bid at bar close.
    pub best_bid: f64,
    /// Best ask at bar close.
    pub best_ask: f64,
    /// Spread in basis points.
    pub spread_bps: f64,
    /// Full-book imbalance [-1, 1].
    pub full_imbalance: f64,
    /// Top-5 level imbalance.
    pub top5_imbalance: f64,
    /// Exponentially-weighted imbalance.
    pub weighted_imbalance: f64,
    /// Total resting bid volume.
    pub total_bid_vol: f64,
    /// Total resting ask volume.
    pub total_ask_vol: f64,
    /// Bid volume in top-5 levels.
    pub bid_vol_top5: f64,
    /// Ask volume in top-5 levels.
    pub ask_vol_top5: f64,
    /// Bid volume in top-10 levels.
    pub bid_vol_top10: f64,
    /// Ask volume in top-10 levels.
    pub ask_vol_top10: f64,
    /// Depth breadth on bid side.
    pub depth_breadth_bid: f64,
    /// Depth breadth on ask side.
    pub depth_breadth_ask: f64,
    /// Mean inter-level gap on bid side (bps).
    pub mean_bid_gap_bps: f64,
    /// Mean inter-level gap on ask side (bps).
    pub mean_ask_gap_bps: f64,
    /// CVD delta over the bar window.
    pub cvd_delta: f64,
    /// CVD buy ratio (0-1).
    pub cvd_ratio: f64,
    /// CVD momentum (1st derivative).
    pub cvd_momentum: f64,
    /// CVD acceleration (2nd derivative).
    pub cvd_acceleration: f64,
    /// Total trades in this bar.
    pub trade_count: u64,
    /// Buy-aggressor trade count.
    pub buy_count: u64,
    /// Sell-aggressor trade count.
    pub sell_count: u64,
    /// Aggressor ratio (buy / total, 0.5 = balanced).
    pub aggressor_ratio: f64,
    /// Large buy trade count.
    pub large_buy_count: u64,
    /// Large sell trade count.
    pub large_sell_count: u64,
    /// Large buy trade volume.
    pub large_buy_vol: f64,
    /// Large sell trade volume.
    pub large_sell_vol: f64,
    /// Large trade aggressor ratio.
    pub large_aggressor_ratio: f64,
    /// Largest single trade in this bar.
    pub max_trade_size: f64,
    /// Trade intensity (trades per unit bar volume).
    pub trade_intensity: f64,
}

/// Which microstructure mechanism contributed to the signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnomalyKind {
    Ofi,
    Cvd,
    DepthImbalance,
    Absorption,
    LiquidityVacuum,
    VolDeltaDivergence,
    AggressorImbalance,
    LargePrint,
    TradeIntensity,
    PatternRepeat,
}

/// Whether the signal reads continuation or reversal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalType {
    /// Flow and depth align with recent price direction.
    MomentumContinuation,
    /// Absorption / divergence suggests price will reverse.
    Reversal,
}

impl std::fmt::Display for SignalType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MomentumContinuation => write!(f, "momentum_continuation"),
            Self::Reversal => write!(f, "reversal"),
        }
    }
}

/// Trade direction implied by a signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SignalDirection {
    Long,
    Short,
    Neutral,
}

impl SignalDirection {
    /// Map to forge-core [`Side`] for order placement.
    #[must_use]
    pub fn to_side(self) -> Option<Side> {
        match self {
            Self::Long => Some(Side::Bid),
            Self::Short => Some(Side::Ask),
            Self::Neutral => None,
        }
    }
}

impl std::fmt::Display for SignalDirection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Long => write!(f, "long"),
            Self::Short => write!(f, "short"),
            Self::Neutral => write!(f, "neutral"),
        }
    }
}

/// Per-bar extracted microstructure features (pre-detection).
#[derive(Debug, Clone, Copy, Default)]
pub struct BarFeatures {
    pub ofi: f64,
    pub ofi_normalized: f64,
    pub cvd_delta: f64,
    pub cvd_momentum: f64,
    pub depth_imbalance: f64,
    pub full_depth_imbalance: f64,
    pub absorption: f64,
    pub bid_absorption: f64,
    pub ask_absorption: f64,
    pub liquidity_vacuum: f64,
    pub bid_vacuum: f64,
    pub ask_vacuum: f64,
    pub vol_delta_divergence: f64,
    pub mid_return_bps: f64,
    pub cvd_acceleration_normalized: f64,
    pub aggressor_ratio: f64,
    pub large_buy_vol: f64,
    pub large_sell_vol: f64,
    pub large_print_imbalance: f64,
    pub trade_intensity: f64,
    pub bar_index: u64,
}

impl BarFeatures {
    #[must_use]
    pub fn to_vector(&self) -> FeatureVector {
        [
            self.ofi_normalized,
            self.cvd_delta,
            self.depth_imbalance,
            self.absorption,
            self.liquidity_vacuum,
            self.vol_delta_divergence,
            self.cvd_acceleration_normalized,
            self.aggressor_ratio,
            self.large_print_imbalance,
            self.trade_intensity,
        ]
    }
}

/// Fixed-size feature vector for multivariate detectors.
pub type FeatureVector = [f64; crate::FEATURE_DIM];

/// A single detected anomaly event on one bar.
#[derive(Debug, Clone)]
pub struct AnomalyEvent {
    pub bar_index: u64,
    pub ts: u64,
    pub kind: AnomalyKind,
    pub direction: SignalDirection,
    /// Per-feature z-score magnitude that triggered this event.
    pub z_score: f64,
    pub raw_value: f64,
    pub confidence: f64,
}

/// Composite trade signal emitted by the engine.
#[derive(Debug, Clone)]
pub struct AnomalySignal {
    pub bar_index: u64,
    pub ts: u64,
    pub signal_type: SignalType,
    pub direction: SignalDirection,
    pub confidence: f64,
    pub description: String,
    pub mahalanobis_dist: f64,
    pub isolation_score: f64,
    pub pattern_count: u32,
    pub expected_move_bps: f64,
    pub hold_bars: u32,
    pub passed_null_edge: bool,
    pub regime: Option<crate::regime::MarketRegime>,
    pub events: Vec<AnomalyEvent>,
}

/// Which multivariate detector(s) to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DetectionMethod {
    /// Mahalanobis distance only (default, fast).
    #[default]
    Mahalanobis,
    /// Isolation Forest only.
    IsolationForest,
    /// Both must agree (conservative).
    Both,
    /// Either detector may trigger (sensitive).
    Either,
}

/// Engine configuration. All windows are in *bars*.
#[derive(Debug, Clone)]
pub struct EngineConfig {
    /// Rolling lookback for statistics and detectors (bars).
    pub lookback_bars: usize,
    /// Mahalanobis distance threshold (chi-squared-like; ~3-5 typical).
    pub mahalanobis_threshold: f64,
    /// Isolation Forest anomaly score threshold [0, 1].
    pub isolation_threshold: f64,
    /// Covariance regularization (added to diagonal, relative to variance).
    pub cov_regularization: f64,
    /// Number of trees in the isolation forest.
    pub isolation_trees: usize,
    /// Detection method selection.
    pub method: DetectionMethod,
    /// Minimum composite confidence to emit a signal.
    pub min_confidence: f64,
    /// Round-trip fee floor (bps).
    pub fee_bps: f64,
    /// Safety margin above fees (bps).
    pub edge_margin_bps: f64,
    /// Top-N levels for depth imbalance.
    pub depth_top_n: usize,
    /// OFI depth-normalization enabled.
    pub ofi_normalized: bool,
    /// Minimum pattern repetitions for reinforcement.
    pub min_pattern_count: u32,
    /// Pattern lookback window (bars).
    pub pattern_lookback_bars: usize,
    /// Base hold duration (bars).
    pub base_hold_bars: u32,
    /// Maximum hold duration (bars).
    pub max_hold_bars: u32,
    /// Null-edge: shuffled permutations per bar for control comparison.
    pub null_edge_permutations: u32,
    /// Null-edge margin: real score must exceed shuffled mean by this factor.
    pub null_edge_margin: f64,
    /// Null-edge PRNG seed for reproducible shuffle control.
    pub null_edge_seed: u64,
    /// Maximum signals per 100 bars (overfitting guard).
    pub max_signals_per_100_bars: f64,
    /// Regime lookback (bars).
    pub regime_lookback: usize,
    /// Regime volatility threshold (bps std) for Volatile classification.
    pub regime_vol_threshold: f64,
    /// Regime autocorrelation threshold for Trending classification.
    pub regime_autocorr_threshold: f64,
    /// Calibrated coefficient: maha_dist contribution to expected move (bps).
    pub expected_move_maha_coeff: f64,
    /// Calibrated coefficient: iso_score contribution to expected move (bps).
    pub expected_move_iso_coeff: f64,
    /// Benjamini-Hochberg FDR alpha level for per-feature significance (0.05 typical).
    pub fdr_alpha: f64,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            lookback_bars: 50,
            mahalanobis_threshold: 4.0,
            isolation_threshold: 0.65,
            cov_regularization: 1.0,
            isolation_trees: 64,
            method: DetectionMethod::Mahalanobis,
            min_confidence: 0.55,
            fee_bps: 9.0,
            edge_margin_bps: 3.0,
            depth_top_n: 5,
            ofi_normalized: true,
            min_pattern_count: 3,
            pattern_lookback_bars: 20,
            base_hold_bars: 3,
            max_hold_bars: 48,
            null_edge_permutations: 100,
            null_edge_margin: 0.25,
            null_edge_seed: 0xA5A5_5A5A_C3C3_3C3C,
            max_signals_per_100_bars: 8.0,
            regime_lookback: 50,
            regime_vol_threshold: 15.0,
            regime_autocorr_threshold: 0.2,
            expected_move_maha_coeff: 3.0,
            expected_move_iso_coeff: 20.0,
            fdr_alpha: 0.05,
        }
    }
}