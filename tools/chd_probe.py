#!/usr/bin/env python3
"""Probe CHD for missing dates Jun 11-17"""
import requests
BASE = "https://api.cryptohftdata.com"
key = open("/root/.chd_key").read().strip().split("=")[-1].strip().strip('"').strip("'")
for dt in ["2026-06-11","2026-06-12","2026-06-13","2026-06-14","2026-06-15","2026-06-16","2026-06-17"]:
    # Probe Binance trades (hour 12 is reasonable middle-of-day)
    url = f"{BASE}/download?file=binance_spot/{dt}/12/BTCUSDT_trades.parquet.zst&api_key={key}"
    r = requests.get(url, timeout=60)
    # Probe HL funding
    url2 = f"{BASE}/download?file=hyperliquid_futures/{dt}/12/BTC_mark_price.parquet.zst&api_key={key}"
    r2 = requests.get(url2, timeout=60)
    # Probe HL OI
    url3 = f"{BASE}/download?file=hyperliquid_futures/{dt}/12/BTC_open_interest.parquet.zst&api_key={key}"
    r3 = requests.get(url3, timeout=60)
    # Probe Binance futures liquidations
    url4 = f"{BASE}/download?file=binance_futures/{dt}/12/BTCUSDT_liquidations.parquet.zst&api_key={key}"
    r4 = requests.get(url4, timeout=60)
    print(f"{dt}: trades={r.status_code} funding={r2.status_code} oi={r3.status_code} liq={r4.status_code}", flush=True)
