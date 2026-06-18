#!/usr/bin/env python3
"""Run depthscope on all ETHUSDT dates - optimized (warmup 30s).
   ETH has 3-5x book depth of BTC. Each date ~3-5h. Total ~72h for all 18 dates.
   Run it in background and come back when done.
"""
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
    outfile = os.path.join(outdir, f"ETHUSDT_{d}_vb10.csv")
    if os.path.exists(outfile) and os.path.getsize(outfile) > 1000:
        print(f"EXISTS {d}, skipping", flush=True)
        continue
    
    print(f"=== DEPTHSCOPE {d} ===", flush=True)
    # Reduced warmup: 30s. ETH book has levels from the start.
    result = subprocess.run(
        [depthscope, "--date", d, "--symbol", "ETHUSDT",
         "--data-root", data_dir, "--volume-bar", "10",
         "--warmup-s", "30",
         "--output", f"/root/depthscope_out/ETHUSDT_{d}_vb10.csv"],
        capture_output=True, text=True
    )
    stdout = result.stdout[-500:] if len(result.stdout) > 500 else result.stdout
    print(stdout, flush=True)
    if result.returncode != 0:
        print(f"ERROR on {d}: {result.stderr[-500:]}", flush=True)
    print(flush=True)

print("=== ALL DONE ===", flush=True)
