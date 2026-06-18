#!/usr/bin/env python3
"""Brute-force check all possible ETH paths on CHD."""
import requests

BASE = "https://api.cryptohftdata.com"
key = open("/root/.chd_key").read().strip().split("=")[-1].strip().strip('"').strip("'")

test_date = "2026-05-01"

# All possible path patterns
paths = [
    # Binance futures bookDelta
    f"binance_futures/{test_date}/00/ETHUSDT_bookDelta.parquet.zst",
    f"binance_futures/{test_date}/00/ETH_bookDelta.parquet.zst",
    # Binance spot bookDelta  
    f"binance_spot/{test_date}/00/ETHUSDT_bookDelta.parquet.zst",
    f"binance_spot/{test_date}/00/ETH_bookDelta.parquet.zst",
    # HL quote
    f"hyperliquid_futures/{test_date}/00/ETHUSDT_quotes.parquet.zst",
    f"hyperliquid_futures/{test_date}/00/ETH_quotes.parquet.zst",
    # HL funding
    f"hyperliquid_futures/{test_date}/00/ETH_mark_price.parquet.zst",
    # HL OI
    f"hyperliquid_futures/{test_date}/00/ETH_open_interest.parquet.zst",
    # Binance liquidations
    f"binance_futures/{test_date}/00/ETHUSDT_liquidations.parquet.zst",
    # trades
    f"binance_futures/{test_date}/00/ETHUSDT_trades.parquet.zst",
]

for p in paths:
    url = f"{BASE}/download?file={p}&api_key={key}"
    r = requests.get(url, timeout=30)
    status = "OK" if r.status_code == 200 else f"{r.status_code}"
    print(f"{status:>4}  {p}")
