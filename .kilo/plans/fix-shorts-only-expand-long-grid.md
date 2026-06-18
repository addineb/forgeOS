# Audit: Pre-Sweep Data & Pipeline Integrity

## Audit Results

### 1. RAW vs ENRICHED ROW COUNTS — PASS
All 18 BTC dates: raw rows == enriched rows (exact 1:1 match).
No rows lost or duplicated during enrichment.

### 2. STITCHED CLEAN ROW SUM — PASS
Sum of 14 good-date enriched rows = 33,789 = stitched clean rows.
4 bad dates correctly excluded (2026-03-01, 2026-04-01/02/03).
No off-by-one errors.

### 3. COLUMN SCHEMA CONSISTENCY — PASS WITH NOTES
- Raw CSV: 58 columns
- Enriched CSV: 106 columns (58 raw + 48 enriched)
- Stitched clean: 106 columns (matches enriched)
- Bar struct in main.rs: 58 raw fields + 22 rolling (_25 + _50 windows only) + 8 base enriched + 10 = matches 106 deserializable fields

**NOTE**: Enriched CSVs contain `_10` and `_100` window columns (20 extra) that Bar struct does NOT read. Serde ignores them silently — not a bug, but wasted computation. The enrich script generates 4 windows × 10 features = 40 rolling cols; Bar struct only reads 2 windows × 10 features = 20.

### 4. DUPLICATE TIMESTAMPS — PASS
Zero duplicate timestamps in stitched clean CSV.
Zero intra-date duplicates in any individual enriched file.

### 5. PRICE CONTINUITY — PASS
mid_price range: 59,130.8 - 97,178.9 (reasonable BTC range).
Only 3 bars with >5% mid_price jump (date boundary gaps — expected).
No NaN in mid_price column.

### 6. NaN AUDIT — PASS WITH KNOWN GAPS
- basis_bps: 100% NaN (Binance spot BBO reconstruction not run — known, documented)
- Rolling features (_10/_25/_50/_100 windows): ~32.2% NaN (10,868 of 33,789 rows)
  - This is expected: date-boundary edge effects + early dates without funding/OI
  - Sweepscope correctly checks `!bars[i].field.is_nan()` before all enriched signal matching
- All raw (depthscope) columns: 0% NaN

### 7. DATE-AWARE CV — PASS
- `find_date_boundaries()` correctly splits on `bar.date` field
- Date is derived from `ts` (Unix nanoseconds) when `date` column is empty/missing
- `days_to_ymd()` is a verified algorithm (no chrono dependency)
- PurgedCv date-aware mode: splits dates into folds, not bars — prevents selection bias

### 8. SWEEP BINARY vs SOURCE CODE — PASS
- Grid: 34 entry signals × 2-3 thresholds × 4 TP × 3 SL × 3 hold = 2,871 configs
- `entry_family()` correctly strips _window then _direction suffixes → 10 hypothesis families
- `is_long` logic: `_long`, `_buy`, `_absorb`, `_discount` → long; else short
- OOS trades run independently on each fold's OOS bar slice (not filtered from all-data run)
- Fee deducted correctly: `gross_pnl_bps - fee_bps` (9 bps round-trip)
- Non-overlapping trades: jumps to `exit_idx + 1` after each trade

### 9. V12 SCORECARD — PASS
- 2,871 configs: 49 PROMOTE, 10 PARK, 2,812 RETIRE
- All 49 PROMOTE are short-only (0 long-side promote)
- Top: `oi_surge_short_50` @ 120.3 net bps
- PBO = 0.486 (below 0.5 threshold)
- This is honest — BTC dropped 31.8% across these dates

### 10. NEW DATES DATA — IN PROGRESS
**BTC bookDelta**: 39 dates on Hetzner, 18 processed, 21 need depthscope:
- `2025-07-09` through `2025-09-22`: 6 dates (NO funding/OI — enrich will produce NaN rolling features for these)
- `2025-10-07` through `2025-11-21`: 4 dates (WITH funding/OI)
- `2025-12-06`, `2025-12-21`: 2 dates (WITH funding/OI)
- `2026-01-05`, `2026-01-20`, `2026-02-04`, `2026-02-19`: 4 dates (WITH funding/OI)
- `2026-03-06`, `2026-03-21`: 2 dates (NO funding/OI — probably bad dates)
- `2026-04-20`, `2026-05-05`, `2026-05-20`: 3 dates (WITH funding/OI)

**ETH bookDelta**: 39 dates on Hetzner, 1 processed, 38 need depthscope
- Currently running: `batch_depthscope_eth_v2.py` on date 2025-12-15 (2nd of 38)
- ETA: ~19 hours remaining (30min/date)

### 11. ETH FUNDING/OI — COVERAGE ISSUE
ETH has funding data for dates that BTC doesn't (e.g., 2025-12-01, 2026-03-01).
ETH `funding` dir has 26 dates vs BTC's 13. This means ETH enrichment will have more non-NaN data for funding/OI signals, which is good for ETH sweeps.

### 12. IDENTIFIED ISSUES

#### ISSUE 1: `_10` and `_100` rolling features computed but unused
- Enrich generates 4 windows (10/25/50/100) × 10 features = 40 rolling columns
- Bar struct only has _25 and _50 fields (20 rolling columns)
- 20 columns computed + written to CSV but never read
- **Fix**: Either add _10/_100 to Bar struct + grid, or stop generating them in enrich
- **Impact**: Wasted disk + compute, not a correctness bug

#### ISSUE 2: 8 new BTC dates have NO funding/OI (Jul-Sep 2025 + Mar 2026)
- These dates can still be depthscoped (orderbook data is present)
- But enriched rolling features (funding_avg, oi_change, etc.) will be NaN
- Sweepscope NaN checks will correctly skip signal evaluation for these bars
- **Impact**: ~30% of bars from these dates will have NaN enrichment = fewer eligible signals
- **Recommendation**: Still process these dates — depth-only signals (skew, imbalance, CVD) still work

#### ISSUE 3: `date` column missing from enriched CSVs
- The `stitch_enriched.py` tool doesn't add a `date` column
- sweepscope's `load_csv()` derives date from `ts` when `bar.date.is_empty()` (line 495-501)
- This works correctly — but it's a hidden dependency
- **Impact**: No bug, but fragile. If ts interpretation changes, date-aware CV breaks silently.

#### ISSUE 4: `batch_depthscope_eth_v2.py` only processes 1/38 ETH dates so far
- Running for ~1 hour, currently on date 2/38
- ETA ~19 hours for all ETH dates
- **Recommendation**: Let it run, but BTC sweep should proceed first (more impactful)

#### ISSUE 5: Enrich script hardcodes "BTC" for funding/OI/liquidation API paths
- `pull_funding()` uses `BTC_mark_price.parquet.zst`
- `pull_oi()` uses `BTC_open_interest.parquet.zst`
- `pull_liquidations()` uses `BTCUSDT_liquidations.parquet.zst`
- **Impact**: Enrich won't work for ETH without modifying these paths
- **Fix needed before ETH enrichment**: Add `--symbol` flag to enrich script

### 13. EXECUTION PLAN: V13 SWEEP

All commands run on Hetzner (root@167.233.57.140). SSH from local uses PowerShell.
Inline Python on SSH ALWAYS fails due to PowerShell escaping — always SCP scripts as files first.
`source /root/.cargo/env` needed before any cargo command.

#### Step 1: Run depthscope on 21 new BTC dates (~44min total, ~2min each)

Write + SCP a batch script:
```bash
#!/bin/bash
source /root/.cargo/env
DATES="2025-07-09 2025-07-24 2025-08-08 2025-08-23 2025-09-07 2025-09-22 2025-10-07 2025-10-22 2025-11-06 2025-11-21 2025-12-06 2025-12-21 2026-01-05 2026-01-20 2026-02-04 2026-02-19 2026-03-06 2026-03-21 2026-04-20 2026-05-05 2026-05-20"
for d in $DATES; do
  echo "[depthscope] $d ..."
  /root/forgeOS/target/release/depthscope \
    --data-root /root/chd/data/ticks \
    --symbol BTCUSDT \
    --date $d \
    --volume-bar 10 \
    --output /root/depthscope_out/BTCUSDT_${d}_vb10.csv
  echo "[depthscope] $d done"
done
```
SCP to `/root/batch_depthscope_btc_new.sh`, then `ssh root@167.233.57.140 "bash /root/batch_depthscope_btc_new.sh"`.

**IMPORTANT**: `--data-root` MUST be `/root/chd/data/ticks` (NOT `/root/chd/data`).

#### Step 2: Enrich all new BTC dates (~2-3 min each, ~45min total)

Write + SCP a batch script:
```bash
#!/bin/bash
DATES="2025-07-09 2025-07-24 2025-08-08 2025-08-23 2025-09-07 2025-09-22 2025-10-07 2025-10-22 2025-11-06 2025-11-21 2025-12-06 2025-12-21 2026-01-05 2026-01-20 2026-02-04 2026-02-19 2026-03-06 2026-03-21 2026-04-20 2026-05-05 2026-05-20"
for d in $DATES; do
  echo "[enrich] $d ..."
  python3 /root/enrich_depthscope.py --date $d --no-basis
  echo "[enrich] $d done"
done
```

NOTE: `--no-basis` skips Binance spot BBO reconstruction (basis_bps stays NaN). Dates without funding/OI (Jul-Sep 2025, Mar 2026) will still process — rolling features will be NaN for those, which is fine.

#### Step 3: Re-stitch clean CSV

Use `tools/stitch_enriched.py` + `tools/filter_bad_dates.py`. Bad dates to exclude:
- Original 4: `2026-03-01`, `2026-04-01`, `2026-04-02`, `2026-04-03`
- New 2 (no funding/OI): `2026-03-06`, `2026-03-21`

Total: 39 dates - 6 bad = 33 good dates. Expected ~55K-65K bars.

Command:
```bash
python3 /root/stitch_enriched.py \
  --indir /root/depthscope_out \
  --pattern "BTCUSDT_*_vb10_enriched.csv" \
  --output /root/depthscope_out/stitched_vb10_enriched_all.csv

python3 /root/filter_bad_dates.py \
  --input /root/depthscope_out/stitched_vb10_enriched_all.csv \
  --output /root/depthscope_out/stitched_vb10_enriched_clean_v2.csv \
  --bad-dates 2026-03-01,2026-04-01,2026-04-02,2026-04-03,2026-03-06,2026-03-21
```

**Verify**: Check new stitched CSV has ~33 dates and row count matches sum of good-date enriched rows.

#### Step 4: Run v13 sweep on expanded BTC data

```bash
source /root/.cargo/env
/root/forgeOS/target/release/sweepscope \
  --input /root/depthscope_out/stitched_vb10_enriched_clean_v2.csv \
  --output /root/depthscope_out/sweepscope_v13_scorecard.csv \
  --per-date /root/depthscope_out/sweepscope_v13_per_date.csv \
  --folds 5 \
  --min-trades 15 \
  --fee-bps 9.0
```

**Key question to evaluate**: Do any signal families now PROMOTE in BOTH directions (long AND short)?

#### Step 5 (parallel): Fix enrich script for ETH — add `--symbol` flag

Edit `tools/enrich_depthscope.py`:
- Add `--symbol` arg (default `BTCUSDT`)
- Change `pull_funding()`: `BTC_mark_price` → `{symbol_root}_mark_price`
- Change `pull_oi()`: `BTC_open_interest` → `{symbol_root}_open_interest`
- Change `pull_liquidations()`: `BTCUSDT_liquidations` → `{symbol}_liquidations`
- Change input filename: `BTCUSDT_{date}_vb10.csv` → `{symbol}_{date}_vb10.csv`

Symbol root mapping: `BTCUSDT` → `BTC`, `ETHUSDT` → `ETH` (strip `USDT`).

#### Step 6 (after ETH depthscope completes ~19h): ETH enrich + stitch + sweep

When batch_depthscope_eth_v2.py finishes all 38 ETH dates:
```bash
# Enrich all ETH dates
for d in $(ls /root/depthscope_out/ETHUSDT_*_vb10.csv | sed 's/.*_\([0-9-]*\)_vb10.*/\1/'); do
  python3 /root/enrich_depthscope.py --date $d --symbol ETHUSDT --no-basis
done

# Stitch + filter + sweep (same pattern as BTC)
```

ETH bad dates may differ from BTC — need to check which ETH dates have 0 funding/OI.

#### Deferred
- Basis feature (100% NaN, needs slow Binance spot BBO reconstruction)
- _10/_100 window features (computed but unused — cosmetic, not a bug)
