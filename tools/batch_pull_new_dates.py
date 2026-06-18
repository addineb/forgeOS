#!/usr/bin/env python3
"""Pull all newly discovered BTC+ETH dates from CHD. Existing dates skipped."""
import subprocess, sys, os

# New dates from CHD probe that we DON'T already have
NEW_DATES = [
    "2025-07-09","2025-07-24",
    "2025-08-08","2025-08-23",
    "2025-09-07","2025-09-22",
    "2025-10-07","2025-10-22",
    "2025-11-06","2025-11-21",
    "2025-12-06","2025-12-21",
    "2026-01-05","2026-01-20",
    "2026-02-04","2026-02-19",
    "2026-03-06","2026-03-21",
    "2026-04-20",
    "2026-05-05","2026-05-20",
    "2026-06-04",  # might already exist
]

os.environ["CRYPTOHFTDATA_API_KEY"] = open("/root/.chd_key").read().strip()
script = "/root/forgeOS/tools/chd-to-parquet.py"
data_dir = "/root/chd/data"
# Default exchange = binance_spot (source of our existing data)

for symbol, coin, hl_sym in [("BTCUSDT", "BTC", "BTC"), ("ETHUSDT", "ETH", "ETH")]:
    print(f"\n=== {symbol} ===")
    for d in NEW_DATES:
        check = f"{data_dir}/ticks/{symbol}/bookDelta/{d}/00.parquet"
        if os.path.exists(check):
            print(f"EXISTS {d}")
            continue
        
        print(f"PULLING {d}", flush=True)
        result = subprocess.run(
            [sys.executable, script, "--date", d, "--symbol", symbol,
             "--coin", coin, "--hl-symbol", hl_sym, "--data-dir", data_dir,
             "--streams", "trade,bookDelta,hlquote,funding,oi"],
            capture_output=True, text=True
        )
        # Print last line
        outlines = [l for l in result.stdout.split('\n') if l.strip()]
        if outlines:
            print(f"  {outlines[-1][:120]}")
        if result.returncode != 0:
            print(f"  ERROR: {result.stderr[-200:]}")
        print()

print("\n=== ALL DONE ===")
