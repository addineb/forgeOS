#!/usr/bin/env python3
"""Test CHD download for ETHUSDT on one date."""
import requests, io, zstandard as zstd, pandas as pd

BASE = "https://api.cryptohftdata.com"
KEY_FILE = "/root/.chd_key"

def load_key():
    return open(KEY_FILE).read().strip().split("=")[-1].strip().strip('"').strip("'")

key = load_key()

# Test: try to download bookDelta for ETHUSDT
# CHD path pattern: exchange/YYYY-MM-DD/HH/symbol_stream.parquet.zst
# For binance_futures: binance_futures/YYYY-MM-DD/HH/BTCUSDT_bookDelta.parquet.zst
# For ETHUSDT: binance_futures/YYYY-MM-DD/HH/ETHUSDT_bookDelta.parquet.zst

test_date = "2026-06-10"

# Try both binance_futures and binance_spot bookDelta
for exch in ["binance_futures"]:
    path = f"{exch}/{test_date}/00/ETHUSDT_bookDelta.parquet.zst"
    url = f"{BASE}/download?file={path}&api_key={key}"
    print(f"Trying: {exch} ...", end=" ")
    r = requests.get(url, timeout=60)
    print(f"HTTP {r.status_code}")
    if r.status_code == 200:
        raw = r.content
        try:
            raw = zstd.ZstdDecompressor().decompress(raw)
        except:
            pass
        df = pd.read_parquet(io.BytesIO(raw))
        print(f"  Columns: {list(df.columns)}")
        print(f"  Rows: {len(df)}")
        print(f"  Sample: \n{df.head(2)}")
        print("  SUCCESS!")
    else:
        print(f"  Response: {r.text[:200]}")
