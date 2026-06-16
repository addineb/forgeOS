#!/usr/bin/env python3
"""Test CHD API for Binance depth data availability."""
import requests, io, sys
import zstandard as zstd
import pandas as pd

KEY = "bf90cc0213eb0d5d949343df0afef3a5741c2a758e91b0b0268a754223a32d86"
BASE = "https://api.cryptohftdata.com"

def test_file(path):
    url = f"{BASE}/download?file={path}&api_key={KEY}"
    print(f"Testing: {path}")
    try:
        r = requests.get(url, timeout=60)
        print(f"  Status: {r.status_code}")
        if r.status_code == 200:
            try:
                raw = zstd.ZstdDecompressor().decompress(r.content)
                df = pd.read_parquet(io.BytesIO(raw))
                print(f"  Rows: {len(df)}, Cols: {list(df.columns)}")
                print(f"  Sample:\n{df.head(2)}")
            except Exception as e:
                print(f"  Parse error: {e}")
        elif r.status_code == 404:
            print(f"  Not available (404)")
        else:
            print(f"  Error: {r.text[:200]}")
    except Exception as e:
        print(f"  Request error: {e}")
    print()

# Test various depth data types on CHD
# Binance futures - depth snapshots
test_file("binance_futures/2026-06-09/09/BTCUSDT_depth20.parquet.zst")
test_file("binance_futures/2026-06-09/09/BTCUSDT_depth100.parquet.zst")

# Binance spot
test_file("binance_spot/2026-06-09/09/BTCUSDT_depth20.parquet.zst")

# Try different naming patterns
test_file("binance_futures/2026-06-09/09/BTCUSDT_bookDepth.parquet.zst")
test_file("binance_futures/2026-06-09/09/BTCUSDT_depth.parquet.zst")
test_file("binance_futures/2026-06-09/09/BTCUSDT_snapshot.parquet.zst")

# Also test what we already know works
test_file("binance_futures/2026-06-09/09/BTCUSDT_trades.parquet.zst")
test_file("binance_futures/2026-06-09/09/BTCUSDT_orderbook.parquet.zst")