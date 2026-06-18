# Plan: Fix Shorts-Only + Data Pull

## Problem
All 16 PROMOTE configs are short-only. A strategy that can only short is a directional bet, not an edge. The grid has 4 long entries vs 9 short entries, and the long thresholds are naive sign-flips.

## Step 1: Diagnose Why Longs Failed
SSH to Hetzner and inspect the v10 scorecard for long-side configs:
- Check if long signals are firing (round_trips > 0) or not triggering at all
- If firing: are they losing (net_bps < 0) or just not clearing DSR?
- This tells us whether to fix thresholds, add more long signals, or both

```bash
ssh root@167.233.57.140 "head -1 /root/depthscope_out/sweepscope_v10_date_aware_scorecard.csv; grep long /root/depthscope_out/sweepscope_v10_date_aware_scorecard.csv; grep _buy /root/depthscope_out/sweepscope_v10_date_aware_scorecard.csv"
```

## Step 2: Expand Long-Side Grid Coverage
The current grid has 4 long entries. The codebase already supports many more long match arms in main.rs that aren't in the grid:
- `oi_unwind_long` (mirrors `oi_unwind_short`)
- `funding_crowd_long_25/50` (contrarian: sustained negative funding → crowded shorts → long)
- `mi_discount_long_25/50` (sustained perp discount → long)
- `cvd_push_long_25/50` (heavy cumulative buying → momentum long)
- `bid_skew_sust_long_50` (50-bar version of persistent bid skew)
- `cvd_mom_cum_long_50` (50-bar version)
- `oi_surge_long_50` (already in grid but needs better thresholds)
- `liq_cascade_buy_25/50` (cumulative buy liquidation → cascade reversal long)
- `liq_flow_buy_25/50` (sustained buy-side liq flow)
- `basis_tight_long` (if basis_bps available — blocked on spot BBO)

Add ALL of these to grid.rs with proper thresholds (not naive sign-flips).

## Step 3: Fix Long-Side Thresholds
Current long thresholds are naive sign-flips of short thresholds. This is wrong because:
- OI surge long (+1/+2) might fire too rarely or too often on BTC
- CVD momentum long (+100/+200) has different distribution than short side
- Bid skew sustained long (0.60) might need a different level

**Approach**: Add wider threshold ranges for long signals to let the sweep find the right level:
- `oi_surge_long_25`: [0.5, 1.0, 2.0] (was [1.0, 2.0])
- `cvd_mom_cum_long_25`: [50.0, 100.0, 200.0] (was [100.0, 200.0])
- `bid_skew_sust_long_25`: [0.40, 0.60, 0.80] (was [0.60])
- `liq_cascade_buy_25`: [5.0, 10.0, 20.0] (was [10.0, 20.0])
- New signals: start with reasonable ranges

## Step 4: Pull More Data
Current: 14 clean dates (Feb-May 2026). Need more for statistical power.

### 4a: Pull missing recent dates from CHD
Dates we don't have yet (or were bad):
- 2026-03-01 (bad: no HL funding/OI)
- 2026-04-01/02/03 (bad: no HL funding/OI)
- 2026-06-11 through 2026-06-17 (missing entirely)
- Any dates in Jan 2026 or Jun 4-10 that we don't have

### 4b: Check what dates CHD has available
Use `enrich_depthscope.py` to probe dates — or just try pulling a range and see what works.

### 4c: Run depthscope + enrich for new dates
For each new date:
```bash
# On Hetzner
./target/release/depthscope --data-root /root/chd/data/ticks --symbol BTCUSDT --date YYYY-MM-DD --volume-bar 10 --output /root/depthscope_out/BTCUSDT_YYYY-MM-DD_vb10.csv
python3 tools/enrich_depthscope.py --date YYYY-MM-DD
```

### 4d: Re-stitch clean CSV
Re-run `tools/filter_bad_dates.py` + `tools/stitch_enriched.py` with expanded date set.

## Step 5: Enable Basis Feature (Optional, Deferred)
`basis_bps` is 100% NaN because `--no-basis` was passed. Enabling it requires running `pull_binance_bbo()` in `enrich_depthscope.py` which reconstructs Binance spot BBO from L2 diffs — very slow (24 hours of L2 data per date). Defer unless long-side still can't find edges.

## Step 6: Run v11 Sweep
With expanded long grid + more data:
```bash
# On Hetzner
./target/release/sweepscope \
  --input /root/depthscope_out/stitched_vb10_enriched_clean.csv \
  --output /root/depthscope_out/sweepscope_v11_scorecard.csv \
  --folds 5 --min-trades 15 --fee-bps 9.0
```

Evaluate: are any long configs now promoting? If still short-only, the honest answer may be "BTC was bearish in this period" — but with more dates (including bullish periods) this should balance out.

## Step 7: Commit
Commit sweepscope v11 with expanded long grid + any new data pipeline changes.

## Execution Order
1. Diagnose long failure from v10 scorecard (read-only, 1 min)
2. Expand grid.rs long entries + fix thresholds (code change)
3. Pull new dates from CHD + enrich (Hetzner, parallelizable per date)
4. Re-stitch clean CSV (Hetzner)
5. Build + run v11 sweep (Hetzner)
6. Evaluate + commit
