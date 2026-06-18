#!/usr/bin/env python3
"""Check parquet contents for ETH vs BTC"""
import pyarrow.parquet as pq

for sym in ["ETHUSDT", "BTCUSDT"]:
    f = f"/root/chd/data/ticks/{sym}/bookDelta/2026-06-10/12.parquet"
    t = f"/root/chd/data/ticks/{sym}/trade/2026-06-10/12.parquet"
    try:
        b = pq.read_table(f)
        print(f"BOOK {sym}: {len(b)} rows, cols: {b.column_names}")
    except Exception as e:
        print(f"BOOK {sym}: ERROR {e}")
    try:
        b = pq.read_table(t)
        print(f"TRADE {sym}: {len(b)} rows, cols: {b.column_names}")
    except Exception as e:
        print(f"TRADE {sym}: ERROR {e}")
    print()
