# Phase 2: Real Data Validation Results

## Data

BTCUSDT spot, 10 BTC volume bars, depthscope CSV format.
Default engine config: lookback=50, maha_thresh=4.0, min_conf=0.55, fee=9bps.

## Results Summary

| Date       | Bars  | Signals | Rate  | Reversal | Momentum | Long | Short | Avg Maha | Avg Move |
|------------|-------|---------|-------|----------|----------|------|-------|----------|----------|
| 2026-06-02 | 3,065 | 8       | 0.3%  | 5        | 3        | 1    | 7     | 37.2     | 111.5 bps |
| 2026-06-03 | 2,704 | 10      | 0.4%  | 3        | 7        | 6    | 4     | 88.2     | 265.4 bps |
| 2026-06-05 | 5,007 | 5       | 0.1%  | 2        | 3        | 3    | 2     | 32.0     | 96.1 bps  |

## 1. Signal count per day

5-10 signals per day from 2,700-5,000 bars. This is **reasonable to slightly sparse**.
- Rate of 0.1-0.4% is conservative — unlikely to over-trade.
- The engine is clearly selective. Most multivariate deviations don't exceed the 4.0 threshold.
- Increasing lookback from 50 to 100 or lowering maha_thresh to 3.0 would likely increase signal count.

## 2. Signal meaningfulness

**Mixed — some signals are clearly regime-shift captures, others are questionable.**

Strong signals (maha > 30):
- `bar 1200, 2026-06-03 09:52, Momentum/Long, maha=156.9` — clearly captures a major order flow regime change. 470 bps expected move is extreme.
- `bar 2648, 2026-06-03 23:36, Reversal/Short, maha=506.8` — massive regime shift at end of day. 1520 bps expected move is unrealistic but reflects how far the feature vector was from the rolling mean.

Weak signals (maha < 10):
- `bar 648, 2026-06-03 05:12, Momentum/Short, maha=8.55, move=25.6 bps` — barely above threshold. After fees (9 + 3 margin = 12 bps), net is only ~13 bps. Marginal signal.

**Confidence is always 1.000 — this is a bug or design issue.** The composite confidence formula in `calc_confidence()` caps at 1.0 and may be saturating because the pattern_boost + agreement components push it to 1.0 too easily when multiple anomaly kinds fire. This makes confidence useless as a filter.

## 3. Pattern-based signals

**ZERO pattern-based signals on all 3 dates.** The `[PATTERN]` tag never appears.

This means the sequence-based detector in pattern.rs is not firing. Likely causes:
- `MIN_AVG_Z = 1.60` filters out too many sequences
- Cooldown prevents counting before enough bars have passed
- The `is_repetitive()` hybrid scoring threshold (0.45) may still be too high
- The sequence hash is too specific (kind + direction + strength) — real anomalies with slightly different strengths won't match

This is the **number one issue** to investigate next. The pattern detector has extensive logic but produces zero results in real usage.

## 4. Obvious issues

1. **Confidence saturation**: Every signal has confidence = 1.000. The `calc_confidence()` function needs to produce meaningful variation. Currently it's a pass-through with no discriminatory power.

2. **Directional bias varies**: 2026-06-02 is heavily short (7/8), 2026-06-03 is balanced-to-long (6/10), 2026-06-05 is balanced (3/2 long). This is good — the engine isn't mechanically biasing one direction.

3. **Anomaly kind distribution**: CVD, LiquidityVacuum, OFI, and Absorption dominate (50-180 counts each). VolDeltaDivergence, DepthImbalance, AggressorImbalance, LargePrint, TradeIntensity appear far less often (0-26 counts). The feature extractor may be computing these features too conservatively.

4. **Extreme Mahalanobis outliers**: Two signals with maha > 150 (one at 506). These are regime-shift captures — the feature vector is in a completely different regime from the rolling 50-bar window. Whether these are tradeable is unclear.

5. **Zero AggressorImbalance/LargePrint/TradeIntensity anomalies**: These columns have 0 anomalies on all dates. The feature extraction or z-score thresholding is suppressing these entirely. Check `z_fdr` filtering in `detect_anomalies()`.

## 5. Overall assessment

**The engine is functionally sound but needs tuning before production use.**

✅ What works:
- Core detection pipeline runs without errors on real data
- Mahalanobis distances computed correctly, outliers detected
- Signal composition produces valid outputs with expected_move, hold_bars, etc.
- Signal rate is conservative (not over-trading)
- Directional balance adapts to market conditions

❌ What needs work:
1. **Confidence scoring** — always 1.0, needs recalibration
2. **Pattern detector** — produces zero signals on real data, needs thresholds revisited
3. **Missing anomaly kinds** — AggressorImbalance, LargePrint, TradeIntensity never fire
4. **Extreme Mahalanobis values** — maha=500 suggests the rolling window isn't handling regime shifts gracefully
5. **Per-date signal count**: 5-10 signals/day is acceptable for research but may be too sparse for systematic trading
