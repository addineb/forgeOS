#!/usr/bin/env python3
import pandas as pd, glob, os
indir = "/root/depthscope_out"
files = sorted(glob.glob(os.path.join(indir, "*_vb10_enriched.csv")))
frames = []
for f in files:
    df = pd.read_csv(f)
    df["ts"] = df["ts"].astype("int64")
    df = df.sort_values("ts").reset_index(drop=True)
    # zero out fwd_ret at date boundaries (last bar of each day)
    if len(df) > 0:
        df.loc[df.index[-1], ["fwd_ret_15m_bps","fwd_ret_1h_bps","fwd_ret_4h_bps"]] = 0.0
    frames.append(df)
stitched = pd.concat(frames, ignore_index=True)
stitched = stitched.sort_values("ts").reset_index(drop=True)
out = os.path.join(indir, "stitched_vb10_enriched.csv")
stitched.to_csv(out, index=False)
print("stitched %d bars -> %s  cols=%d" % (len(stitched), out, len(stitched.columns)))
print("new cols:", [c for c in stitched.columns if c in ["funding_rate","mark_index_bps","oi","oi_pct_change","liq_vol_buy","liq_vol_sell","liq_imbalance","basis_bps"]])
print("NaN summary (new cols):")
for c in ["funding_rate","mark_index_bps","oi","oi_pct_change","liq_vol_buy","liq_vol_sell","liq_imbalance","basis_bps"]:
    print("  %s: %d NaN / %d total (%.1f%%)" % (c, stitched[c].isna().sum(), len(stitched), 100*stitched[c].isna().mean()))
