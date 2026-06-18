#!/bin/bash
# Compare raw data sizes for failing vs good dates
echo "=== FAILING DATES ==="
for D in 2026-04-02 2026-04-03 2026-06-06; do
    echo "Date: $D"
    echo "  bookDelta MB: $(du -sm /root/chd/data/ticks/BTCUSDT/bookDelta/$D/ 2>/dev/null | awk '{print $1}')"
    echo "  trade MB: $(du -sm /root/chd/data/ticks/BTCUSDT/trade/$D/ 2>/dev/null | awk '{print $1}')"
    echo "  bookDelta files: $(ls /root/chd/data/ticks/BTCUSDT/bookDelta/$D/*.parquet 2>/dev/null | wc -l)"
    echo "  trade files: $(ls /root/chd/data/ticks/BTCUSDT/trade/$D/*.parquet 2>/dev/null | wc -l)"
done

echo ""
echo "=== GOOD DATES ==="
for D in 2025-12-01 2026-06-05 2026-02-01; do
    echo "Date: $D"
    echo "  bookDelta MB: $(du -sm /root/chd/data/ticks/BTCUSDT/bookDelta/$D/ 2>/dev/null | awk '{print $1}')"
    echo "  trade MB: $(ls -l /root/chd/data/ticks/BTCUSDT/bookDelta/$D/*.parquet 2>/dev/null | awk '{sum+=$5}END{print sum/1024/1024}')"
    echo "  trade MB: $(du -sm /root/chd/data/ticks/BTCUSDT/trade/$D/ 2>/dev/null | awk '{print $1}')"
    echo "  bookDelta files: $(ls /root/chd/data/ticks/BTCUSDT/bookDelta/$D/*.parquet 2>/dev/null | wc -l)"
    echo "  trade files: $(ls /root/chd/data/ticks/BTCUSDT/trade/$D/*.parquet 2>/dev/null | wc -l)"
done

echo ""
echo "=== DEPTHSCOPE RUN STATS ==="
for D in 2026-04-02 2026-04-03 2026-06-06 2025-12-01 2026-06-05; do
    CSV="/root/depthscope_out/BTCUSDT_${D}_vb10.csv"
    if [ -f "$CSV" ]; then
        ROWS=$(wc -l < "$CSV")
        echo "  $D: $ROWS rows in depthscope output"
    fi
done
