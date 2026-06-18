# forge-anomaly

Volume-bar anomaly detection engine for ForgeOS. Identifies repetitive order-book behavioral patterns that consistently precede momentum continuations or reversals.

Self-contained crate: depends only on `forge-core` plus `ndarray`, `ndarray-stats`, and `linfa` for multivariate statistics.

## Crate layout

```
src/
├── lib.rs          Public API
├── types.rs        VolumeBar, AnomalySignal, EngineConfig
├── features.rs     OFI, CVD, imbalance, absorption, vacuum, divergence
├── stats.rs        RollingFeatureWindow (mean, covariance, z-scores)
├── detector.rs     Mahalanobis + Isolation Forest + AnomalyDetector
├── pattern.rs      Repetitive pattern counter
├── null_edge.rs    Shuffled-control null-edge gate
├── engine.rs       AnomalyEngine orchestrator
├── csv.rs          Depthscope CSV loader
└── bin/anomalyscope.rs
```

## Pipeline

```
VolumeBar
    → FeatureExtractor        (per-bar microstructure features)
    → RollingFeatureWindow    (rolling mean / covariance / z-scores)
    → AnomalyDetector         (Mahalanobis + Isolation Forest)
    → PatternCounter          (repetitive co-occurrence)
    → NullEdgeGate            (shuffled control + rate limit)
    → AnomalySignal           (confidence, type, description)
```

## Feature vector (7 dimensions)

| # | Feature | Description |
|---|---------|-------------|
| 0 | OFI | CKS order-flow imbalance, depth-normalized |
| 1 | CVD | Aggressive flow delta, volume-scaled |
| 2 | Depth imbalance | Top-N resting volume skew |
| 3 | Absorption | Aggressive flow failing to break touch |
| 4 | Liquidity vacuum | Depth withdrawal + gap widening |
| 5 | Vol-delta divergence | Price vs CVD disagreement |
| 6 | Mid return | Bar-to-bar price change (bps) |

## Detection methods

| Method | Description |
|--------|-------------|
| `Mahalanobis` | Primary. Multivariate distance from rolling centroid. |
| `IsolationForest` | Alternative. Random-partition tree ensemble. |
| `Both` | Conservative: both must fire. |
| `Either` | Sensitive: either may fire. |

## Signal output

```rust
pub struct AnomalySignal {
    pub signal_type: SignalType,   // MomentumContinuation | Reversal
    pub direction: SignalDirection, // Long | Short
    pub confidence: f64,           // [0, 1]
    pub description: String,       // human-readable read
    pub mahalanobis_dist: f64,
    pub isolation_score: f64,
    pub pattern_count: u32,
    pub expected_move_bps: f64,  // fee-adjusted (default ≥ 12 bps)
    pub hold_bars: u32,            // fractal-scaled
    pub passed_null_edge: bool,
    pub events: Vec<AnomalyEvent>,
}
```

## Usage

```rust
use forge_anomaly::{AnomalyEngine, DetectionMethod, EngineConfig, VolumeBar};

let cfg = EngineConfig {
    method: DetectionMethod::Mahalanobis,
    lookback_bars: 50,
    mahalanobis_threshold: 4.0,
    fee_bps: 9.0,
    edge_margin_bps: 3.0,
    ..Default::default()
};

let mut engine = AnomalyEngine::new(cfg);

// Feed volume bars sequentially (from CSV or live pipeline)
let output = engine.on_bar(&bar);
if let Some(sig) = output.signal {
    println!("{} {} conf={:.2}", sig.signal_type, sig.direction, sig.confidence);
    println!("{}", sig.description);
}
```

## CSV replay

```bash
cargo run -p forge-anomaly --bin anomalyscope -- \
  --input BTCUSDT_2026-02-01_vb10_enriched.csv \
  --output anomaly_signals.csv \
  --method mahalanobis \
  --lookback-bars 50
```

## Configuration defaults

| Parameter | Default | Notes |
|-----------|---------|-------|
| `lookback_bars` | 50 | Rolling window (bars, not seconds) |
| `mahalanobis_threshold` | 4.0 | Distance threshold |
| `isolation_threshold` | 0.65 | Score threshold [0, 1] |
| `fee_bps` | 9.0 | Round-trip fee floor |
| `edge_margin_bps` | 3.0 | Safety margin above fees |
| `min_pattern_count` | 3 | Repetitions before pattern fires |
| `base_hold_bars` | 3 | Minimum hold (fractal-scaled up) |
| `max_hold_bars` | 48 | Maximum hold |

## Tests

```bash
cargo test -p forge-anomaly
```

## Dependencies

- `forge-core` — `Side`, fixed-point primitives
- `ndarray` + `ndarray-stats` — rolling covariance, summary statistics
- `linfa` — Dataset interop for Isolation Forest training data
- `csv` + `serde` — depthscope CSV ingestion