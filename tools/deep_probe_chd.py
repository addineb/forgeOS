#!/usr/bin/env python3
"""Deep probe CHD for all available BTC dates."""
import requests, datetime
from concurrent.futures import ThreadPoolExecutor, as_completed

BASE = "https://api.cryptohftdata.com"
key = open("/root/.chd_key").read().strip()

# CHD data is at binance_futures (what we used for depthscope L2 data)
# Let's check a few known-good dates first to verify
known_good = ["2026-06-10", "2026-02-01", "2025-12-15", "2025-12-01"]
print("=== Verifying known dates ===")
for d in known_good:
    url = f"{BASE}/download?file=binance_futures/{d}/00/BTCUSDT_bookDelta.parquet.zst&api_key={key}"
    r = requests.get(url, timeout=15)
    print(f"  {d}: HTTP {r.status_code}")

# Now scan every 15 days from 2024-01-01 to 2026-06-18
print("\n=== Scanning all dates ===")
dates_to_check = []
d = datetime.date(2024, 1, 1)
end = datetime.date(2026, 6, 18)
while d <= end:
    dates_to_check.append(d.strftime("%Y-%m-%d"))
    d += datetime.timedelta(days=15)

def check_date(date):
    # Try binance_futures (what we use)
    url = f"{BASE}/download?file=binance_futures/{date}/00/BTCUSDT_bookDelta.parquet.zst&api_key={key}"
    try:
        r = requests.get(url, timeout=15)
        if r.status_code == 200:
            return (date, "futures")
    except:
        pass
    # Try binance_spot
    url = f"{BASE}/download?file=binance_spot/{date}/00/BTCUSDT_orderbook.parquet.zst&api_key={key}"
    try:
        r = requests.get(url, timeout=15)
        if r.status_code == 200:
            return (date, "spot")
    except:
        pass
    return (date, None)

found = []
with ThreadPoolExecutor(max_workers=15) as ex:
    futures = {ex.submit(check_date, d): d for d in dates_to_check}
    for f in as_completed(futures):
        date, source = f.result()
        if source:
            found.append((date, source))

found.sort()
print(f"Dates with BTC data: {len(found)} total")
for d, src in found:
    marker = " *** NEW" if d not in {"2025-12-01","2025-12-15","2026-01-15","2026-02-01","2026-03-01","2026-04-01","2026-04-02","2026-04-03","2026-05-01","2026-06-02","2026-06-03","2026-06-04","2026-06-05","2026-06-06","2026-06-07","2026-06-08","2026-06-09","2026-06-10"} else ""
    print(f"  {d} ({src}){marker}")

print(f"\nAlready pulled: 18")
print(f"NEW dates to pull: {sum(1 for d,_ in found if d not in {'2025-12-01','2025-12-15','2026-01-15','2026-02-01','2026-03-01','2026-04-01','2026-04-02','2026-04-03','2026-05-01','2026-06-02','2026-06-03','2026-06-04','2026-06-05','2026-06-06','2026-06-07','2026-06-08','2026-06-09','2026-06-10'})}")
