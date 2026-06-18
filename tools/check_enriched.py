#!/usr/bin/env python3
import pandas as pd
df = pd.read_csv("/root/depthscope_out/BTCUSDT_2026-02-01_vb10_enriched.csv")
cols = ["funding_rate","mark_index_bps","oi","oi_pct_change",
        "liq_vol_buy","liq_vol_sell","liq_imbalance","basis_bps"]
print(df[cols].describe().to_string())
print("\nNaN counts:")
print(df[cols].isna().sum().to_string())
print("\nFirst 5 rows (new cols):")
print(df[cols].head().to_string())
