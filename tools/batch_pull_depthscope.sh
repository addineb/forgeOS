#!/bin/bash
set -e
source /root/.cargo/env
cd /root/forgeOS

DATA_DIR=/root/chd/data/ticks
DEPTH_OUT=/root/depthscope_out

for DATE in 2026-06-11 2026-06-12 2026-06-13 2026-06-14 2026-06-15 2026-06-16 2026-06-17; do
    echo "========================================"
    echo "=== $DATE ==="
    echo "========================================"

    # 1. Pull raw Binance tick data (trades + orderbook deltas)
    echo "[1/3] Pulling CHD raw data..."
    python3 tools/chd-to-parquet.py \
        --date "$DATE" \
        --streams trade,bookDelta \
        --data-dir "$DATA_DIR" \
        --hours all 2>&1 | tail -10

    # 2. Run depthscope to produce volume-bar CSVs
    echo "[2/3] Running depthscope..."
    ./target/release/depthscope \
        --data-root "$DATA_DIR" \
        --symbol BTCUSDT \
        --date "$DATE" \
        --volume-bar 10 \
        --warmup-s 300 \
        --output "$DEPTH_OUT/BTCUSDT_${DATE}_vb10.csv" 2>&1 | tail -5

    # 3. Run enrichment (funding + OI + liquidations + rolling features)
    echo "[3/3] Enriching..."
    python3 tools/enrich_depthscope.py \
        --date "$DATE" \
        --indir "$DEPTH_OUT" \
        --no-basis 2>&1 | tail -5

    echo "=== $DATE complete ==="
done

echo "========================================"
echo "=== ALL DATES DONE ==="
echo "========================================"
