#!/usr/bin/env python3
"""Run depthscope on all ETHUSDT dates."""
import subprocess, sys, os

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

depthscope = "/root/forgeOS/target/release/depthscope"
data_dir = "/root/chd/data/ticks"
outdir = "/root/depthscope_out"

for d in dates:
    # Check if already processed
    outfile = os.path.join(outdir, f"ETHUSDT_{d}_vb10.csv")
    if os.path.exists(outfile) and os.path.getsize(outfile) > 1000:
        print(f"EXISTS {d}, skipping")
        continue
    
    print(f"=== DEPTHSCOPE {d} ===", flush=True)
    result = subprocess.run(
        [depthscope, "--date", d, "--symbol", "ETHUSDT",
         "--data-root", data_dir, "--volume-bar", "10",
         "--output", f"/root/depthscope_out/ETHUSDT_{d}_vb10.csv"],
        capture_output=True, text=True
    )
    print(result.stdout[-500:] if len(result.stdout) > 500 else result.stdout)
    if result.returncode != 0:
        print(f"ERROR on {d}: {result.stderr[-500:]}")
    print()

print("=== DONE ===")
