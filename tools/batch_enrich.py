#!/usr/bin/env python3
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
indir = "/root/depthscope_out"
for d in dates:
    src = os.path.join(indir, "BTCUSDT_%s_vb10.csv" % d)
    if not os.path.exists(src):
        print("SKIP %s (no source CSV)" % d)
        continue
    out = os.path.join(indir, "BTCUSDT_%s_vb10_enriched.csv" % d)
    if os.path.exists(out):
        print("EXISTS %s, skipping" % d)
        continue
    print("=== %s ===" % d)
    r = subprocess.run([sys.executable, "/root/forgeOS/tools/enrich_depthscope.py",
                        "--date", d, "--indir", indir, "--no-basis"],
                       capture_output=True, text=True)
    print(r.stdout)
    if r.returncode != 0:
        print("ERROR: %s" % r.stderr[-500:])
