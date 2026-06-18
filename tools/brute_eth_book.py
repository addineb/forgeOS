#!/usr/bin/env python3
"""Brute force check binance_futures bookDelta for many dates."""
import requests

BASE = "https://api.cryptohftdata.com"
key = open("/root/.chd_key").read().strip().split("=")[-1].strip().strip('"').strip("'")

dates = ["2026-02-01", "2026-05-01", "2026-06-10", "2025-12-01"]

# Check BTCUSDT bookDelta exists for reference
for d in dates:
    url = f"{BASE}/download?file=binance_futures/{d}/00/BTCUSDT_bookDelta.parquet.zst&api_key={key}"
    r = requests.get(url, timeout=30)
    print(f"BTC {d}: {r.status_code}")

print()

# Check ETH variations
for d in dates:
    for variant in ["ETHUSDT_bookDelta", "ETH_bookDelta", "ETHUSDT_orderbook", "ETHUSDT_depth", "ETHUSDT_book", "ETH_book"]:
        url = f"{BASE}/download?file=binance_futures/{d}/00/{variant}.parquet.zst&api_key={key}"
        r = requests.get(url, timeout=30)
        if r.status_code == 200:
            print(f"FOUND: binance_futures/{d}/00/{variant}.parquet.zst")

# Also try spot variations
for d in dates:
    for variant in ["ETHUSDT_bookDelta", "ETH_bookDelta", "ETHUSDT_orderbook", "ETHUSDT_depth", "ETHUSDT_book", "ETH_book"]:
        url = f"{BASE}/download?file=binance_spot/{d}/00/{variant}.parquet.zst&api_key={key}"
        r = requests.get(url, timeout=30)
        if r.status_code == 200:
            print(f"FOUND: binance_spot/{d}/00/{variant}.parquet.zst")
