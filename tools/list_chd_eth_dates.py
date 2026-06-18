#!/usr/bin/env python3
"""List available CHD dates for ETHUSDT bookDelta."""
import requests, re

BASE = "https://api.cryptohftdata.com"
key = open("/root/.chd_key").read().strip().split("=")[-1].strip().strip('"').strip("'")

for product in ["ETHUSDT", "BTCUSDT"]:
    print(f"=== {product} ===")
    r = requests.get(
        f"{BASE}/list-files?prefix=ticks/{product}/bookDelta/",
        params={"api_key": key},
        timeout=30
    )
    if r.status_code == 200:
        data = r.json()
        if isinstance(data, list):
            dates = set()
            for item in data:
                m = re.search(r"(\d{4}-\d{2}-\d{2})", str(item))
                if m:
                    dates.add(m.group(1))
            print(f"  Dates found: {len(dates)}")
            for d in sorted(dates):
                print(f"    {d}")
        else:
            print(f"  Response: {str(data)[:500]}")
    else:
        print(f"  HTTP {r.status_code}: {r.text[:300]}")
    print()
