#!/bin/bash
# Validate all June 2026 dates
for d in 2026-06-02 2026-06-03 2026-06-04 2026-06-05 2026-06-06 2026-06-07 2026-06-08 2026-06-09 2026-06-10; do
    echo "=== $d ==="
    /root/forgeOS/target/release/validate \
        --input "/root/depthscope_out/BTCUSDT_${d}_vb10_enriched.csv" \
        --date "$d" \
        --output "/root/forgeOS/results/results_${d}.txt" 2>&1 | grep -E 'total signals|pattern-based|avg confidence'
    echo ""
done
echo "=== AGGREGATE ==="
grep -h 'total signals\|pattern-based' /root/forgeOS/results/results_2026-06-*.txt | grep 'total signals' | awk '{s+=$3} END {print "total signals: " s}'
grep -h 'total signals\|pattern-based' /root/forgeOS/results/results_2026-06-*.txt | grep 'pattern-based' | awk '{p+=$3} END {print "pattern-based: " p}'
