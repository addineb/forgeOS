#!/bin/bash
# Check trade columns and run validation on June 2026 dates
for d in 2026-06-06 2026-06-07 2026-06-08 2026-06-09 2026-06-10; do
    c=$(head -1 /root/depthscope_out/BTCUSDT_${d}_vb10_enriched.csv | tr ',' '\n' | grep -c trade_count)
    echo "$d: trade_count=$c"
done
