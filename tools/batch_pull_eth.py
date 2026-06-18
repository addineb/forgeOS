#!/usr/bin/env python3
"""Pull ETHUSDT CHD data for all dates we have BTC data for."""
import subprocess, sys, os

# Read key from file and set env
key = open("/root/.chd_key").read().strip()
os.environ["CRYPTOHFTDATA_API_KEY"] = key

dates = [
    "2025-12-01","2025-12-15",
    "2026-01-15",
    "2026-02-01",
    "2026-03-01",
    "2026-04-01","2026-04-02","2026-04-03",
    "2026-05-01",
    "2026-06-02","2026-06-03","2026-06-04","2026-06-05",
    "2026-06-06","2026-06-07","2026-06-08","2026-06-09","2026-06-10",
]

script = "/root/forgeOS/tools/chd-to-parquet.py"
data_dir = "/root/chd/data"

for d in dates:
    # Check if already pulled
    check = f"{data_dir}/ticks/ETHUSDT/bookDelta/{d}"
    if os.path.exists(check):
        existing = len([f for f in os.listdir(check) if f.endswith('.parquet')])
        if existing >= 24:
            print(f"EXISTS {d} ({existing} hours), skipping")
            continue
    
    print(f"=== PULLING {d} ===", flush=True)
    result = subprocess.run(
        [sys.executable, script, "--date", d, "--symbol", "ETHUSDT",
         "--coin", "ETH", "--hl-symbol", "ETH", "--data-dir", data_dir,
         "--streams", "trade,bookDelta,hlquote,funding,oi"],
        capture_output=True, text=True
    )
    print(result.stdout)
    if result.returncode != 0:
        print(f"ERROR on {d}: {result.stderr[-500:]}")
    print()

print("=== DONE ===")
