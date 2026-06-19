#!/usr/bin/env bash
# Batch enrichment with --no-trades for trade estimates from CVD
set -e
cd /root/forgeOS
for d in 2026-06-02 2026-06-03 2026-06-04 2026-06-05; do
    echo "=== $d ==="
    python3 tools/enrich_depthscope.py --date "$d" --no-trades
    echo ""
done
echo "DONE"
