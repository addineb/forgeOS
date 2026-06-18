#!/usr/bin/env python3
import pandas as pd, glob, os
indir = "/root/depthscope_out"
files = sorted(glob.glob(os.path.join(indir, "*_vb10_enriched.csv")))
seen = set()
frames = []
for f in files:
    df = pd.read_csv(f)
    df["ts"] = df["ts"].astype("int64")
    # dedup by file date from filename
    tag = os.path.basename(f).split("_vb10_enriched")[0]
    if tag in seen:
        continue
    seen.add(tag)
    df = df.sort_values("ts").reset_index(drop=True)
    if len(df) > 0:
        df.loc[df.index[-1], ["fwd_ret_15m_bps","fwd_ret_1h_bps","fwd_ret_4h_bps"]] = 0.0
    frames.append(df)
stitched = pd.concat(frames, ignore_index=True)
stitched = stitched.sort_values("ts").reset_index(drop=True)
# dedup exact timestamps
stitched = stitched.drop_duplicates(subset="ts", keep="last").reset_index(drop=True)
out = os.path.join(indir, "stitched_vb10_enriched.csv")
stitched.to_csv(out, index=False)
print("stitched %d bars -> %s  cols=%d" % (len(stitched), out, len(stitched.columns)))
print("rolling cols:", [c for c in stitched.columns if "cum" in c or "avg" in c or "change" in c and "pct" not in c])
