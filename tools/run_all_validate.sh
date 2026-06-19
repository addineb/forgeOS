#!/bin/bash
# Batch validate enriched CSVs
set -e
for d in 2026-06-02 2026-06-03 2026-06-04 2026-06-05; do
    echo "=== $d ==="
    /root/forgeOS/target/release/validate \
        --input "/root/depthscope_out/BTCUSDT_${d}_vb10_enriched.csv" \
        --date "$d" \
        --output "/root/forgeOS/results/results_${d}.txt" 2>&1 | tail -25
    echo ""
done
echo "ALL DONE"
