#!/usr/bin/env python3
"""Quick check: how many zero forward returns in the stitched CSV?"""
import pandas as pd
df = pd.read_csv("/root/depthscope_out/stitched_vb10.csv")
print(f"Rows: {len(df)}")
for col in ["fwd_ret_15m_bps", "fwd_ret_1h_bps", "fwd_ret_4h_bps"]:
    z = (df[col] == 0).sum()
    print(f"  {col}: {z} zeros ({100*z/len(df):.1f}%)")

# Check last 10 rows
print("\nLast 10 rows (ts, mid_price, fwd_ret_4h_bps):")
print(df[["ts", "mid_price", "fwd_ret_4h_bps"]].tail(10).to_string())
