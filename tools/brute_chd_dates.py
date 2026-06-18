#!/usr/bin/env python3
"""Brute-force check CHD for BTC data availability across many months."""
import requests, io, zstandard as zstd
from concurrent.futures import ThreadPoolExecutor, as_completed

BASE = "https://api.cryptohftdata.com"
key = open("/root/.chd_key").read().strip()

# Check one date per week from 2024-01 through 2026-06
import datetime
dates_to_check = []
d = datetime.date(2024, 1, 1)
end = datetime.date(2026, 6, 18)
while d < end:
    dates_to_check.append(d.strftime("%Y-%m-%d"))
    d += datetime.timedelta(days=7)  # every 7 days

def check_date(date):
    url = f"{BASE}/download?file=binance_futures/{date}/00/BTCUSDT_bookDelta.parquet.zst&api_key={key}"
    try:
        r = requests.get(url, timeout=15)
        return (date, r.status_code == 200)
    except:
        return (date, False)

found = []
missing = []
with ThreadPoolExecutor(max_workers=10) as ex:
    futures = {ex.submit(check_date, d): d for d in dates_to_check}
    for f in as_completed(futures):
        date, ok = f.result()
        if ok:
            found.append(date)
        else:
            missing.append(date)

print(f"Dates with data: {len(found)}")
for d in sorted(found):
    print(f"  {d}")
print(f"\nDates without: {len(missing)}")
# Show edge: what's the earliest and latest
if found:
    print(f"\nRange: {sorted(found)[0]} to {sorted(found)[-1]}")
    # Show gaps
    all_dates = sorted(found)
    for i in range(len(all_dates)-1):
        d1 = datetime.date.fromisoformat(all_dates[i])
        d2 = datetime.date.fromisoformat(all_dates[i+1])
        gap = (d2 - d1).days
        if gap > 14:
            print(f"  GAP: {all_dates[i]} -> {all_dates[i+1]} ({gap} days)")
