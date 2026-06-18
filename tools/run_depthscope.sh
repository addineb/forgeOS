#!/bin/bash
set -e
source ~/.cargo/env
cd /root/forgeOS

for DATE in 2025-12-01 2025-12-15 2026-01-15 2026-02-01 2026-03-01 2026-04-01 2026-04-02 2026-04-03 2026-05-01 2026-06-02 2026-06-03 2026-06-04 2026-06-05 2026-06-06 2026-06-07 2026-06-08 2026-06-09 2026-06-10; do
    echo "=== Running $DATE volume-bar 10 BTC ==="
    ./target/release/depthscope --data-root /root/chd/data/ticks --symbol BTCUSDT --date $DATE --volume-bar 10 --warmup-s 300 --output /root/depthscope_out/BTCUSDT_${DATE}_vb10.csv 2>&1 | tail -5
    echo ""
done

echo "=== All done ==="
ls -la /root/depthscope_out/