#!/usr/bin/env python3
"""Quick depth-pattern analysis: null-edge gate and feature overview."""

import pandas as pd
import numpy as np
import sys

def main():
    path = sys.argv[1] if len(sys.argv) > 1 else "/root/depthscope_out/stitched_vb10.csv"
    df = pd.read_csv(path)
    print(f"Shape: {df.shape}")
    print(f"Columns ({len(df.columns)}): {list(df.columns)}")
    print()

    # === NULL-EDGE GATE ===
    print("=== NULL-EDGE GATE ===")
    print("Coinflip (random long/short) must lose ~fees")
    for col in ['fwd_ret_15m_bps', 'fwd_ret_1h_bps', 'fwd_ret_4h_bps']:
        ret = df[col]
        nonzero = (ret != 0)
        mean_ret = ret[nonzero].mean()
        std_ret = ret[nonzero].std()
        # Taker fee on HL: ~9 bps round-trip = 4.5 bps one-way
        print(f"  {col}: mean={mean_ret:.2f} bps, std={std_ret:.1f} bps, mean/fee={mean_ret/4.5:.2f}")
        print(f"    skew={ret[nonzero].skew():.3f}, median={ret[nonzero].median():.2f} bps")
        # Sharpe-like: mean / std
        print(f"    sharpe (annualized, 6 bars/hr): {mean_ret / std_ret * np.sqrt(6 * 24 * 365):.2f}")
    print()

    # === FEATURE STATS ===
    print("=== KEY FEATURE STATS ===")
    key_features = [
        'imbalance_top5', 'imbalance_top10', 'imbalance_top20',
        'bid_ask_ratio_0_1pct', 'bid_ask_ratio_1_5pct', 'bid_ask_ratio_5_10pct',
        'cvd_delta', 'cvd_momentum_1', 'cvd_momentum_2',
        'wall_bid_count', 'wall_ask_count',
        'vp_poc_distance_bps', 'vp_bid_ask_ratio',
        'depth_bid_0_1pct', 'depth_ask_0_1pct',
        'depth_bid_1_5pct', 'depth_ask_1_5pct',
    ]
    for col in key_features:
        if col in df.columns:
            print(f"  {col}: mean={df[col].mean():.4f}, std={df[col].std():.4f}, "
                  f"min={df[col].min():.4f}, max={df[col].max():.4f}")
    print()

    # === SIMPLE CORRELATION WITH FORWARD RETURNS ===
    print("=== FEATURE-RETURN CORRELATIONS ===")
    feature_cols = [c for c in df.columns if c not in ['ts', 'fwd_ret_15m_bps', 'fwd_ret_1h_bps', 'fwd_ret_4h_bps']]
    for ret_col in ['fwd_ret_15m_bps', 'fwd_ret_1h_bps', 'fwd_ret_4h_bps']:
        print(f"\n  --- {ret_col} ---")
        corrs = []
        for fc in feature_cols:
            if df[fc].dtype in [np.float64, np.int64, float, int]:
                r = df[fc].corr(df[ret_col])
                if not np.isnan(r):
                    corrs.append((abs(r), fc, r))
        corrs.sort(reverse=True)
        for _, fc, r in corrs[:15]:
            print(f"    {fc}: r={r:.4f}")

    # === QUANTILE ANALYSIS: does imbalance predict returns? ===
    print("\n=== QUANTILE ANALYSIS: imbalance_top20 -> fwd_ret_15m ===")
    if 'imbalance_top20' in df.columns:
        df['q20'] = pd.qcut(df['imbalance_top20'], 10, labels=False, duplicates='drop')
        for q in range(10):
            sub = df[df['q20'] == q]
            print(f"  Q{q}: imbalance={sub['imbalance_top20'].mean():.4f}, "
                  f"ret_15m={sub['fwd_ret_15m_bps'].mean():.2f}, "
                  f"ret_1h={sub['fwd_ret_1h_bps'].mean():.2f}, "
                  f"ret_4h={sub['fwd_ret_4h_bps'].mean():.2f}, "
                  f"n={len(sub)}")

    # === QUANTILE ANALYSIS: bid_ask_ratio_0_1pct ===
    print("\n=== QUANTILE ANALYSIS: bid_ask_ratio_0_1pct -> fwd_ret ===")
    if 'bid_ask_ratio_0_1pct' in df.columns:
        df['q_bar'] = pd.qcut(df['bid_ask_ratio_0_1pct'], 10, labels=False, duplicates='drop')
        for q in range(10):
            sub = df[df['q_bar'] == q]
            print(f"  Q{q}: ratio={sub['bid_ask_ratio_0_1pct'].mean():.4f}, "
                  f"ret_15m={sub['fwd_ret_15m_bps'].mean():.2f}, "
                  f"ret_1h={sub['fwd_ret_1h_bps'].mean():.2f}, "
                  f"ret_4h={sub['fwd_ret_4h_bps'].mean():.2f}, "
                  f"n={len(sub)}")

    # === QUANTILE ANALYSIS: CVD delta ===
    print("\n=== QUANTILE ANALYSIS: cvd_delta -> fwd_ret ===")
    if 'cvd_delta' in df.columns:
        df['q_cvd'] = pd.qcut(df['cvd_delta'], 10, labels=False, duplicates='drop')
        for q in range(10):
            sub = df[df['q_cvd'] == q]
            print(f"  Q{q}: cvd_delta={sub['cvd_delta'].mean():.4f}, "
                  f"ret_15m={sub['fwd_ret_15m_bps'].mean():.2f}, "
                  f"ret_1h={sub['fwd_ret_1h_bps'].mean():.2f}, "
                  f"ret_4h={sub['fwd_ret_4h_bps'].mean():.2f}, "
                  f"n={len(sub)}")

    # === SPREAD ANALYSIS: extreme imbalance ===
    print("\n=== EXTREME IMBALANCE ANALYSIS ===")
    if 'imbalance_top20' in df.columns:
        for threshold in [0.3, 0.4, 0.5, 0.6]:
            bid_heavy = df[df['imbalance_top20'] > threshold]
            ask_heavy = df[df['imbalance_top20'] < -threshold]
            if len(bid_heavy) > 10 and len(ask_heavy) > 10:
                print(f"  |imbalance| > {threshold}: "
                      f"bid_heavy(n={len(bid_heavy)}, ret_15m={bid_heavy['fwd_ret_15m_bps'].mean():.2f}), "
                      f"ask_heavy(n={len(ask_heavy)}, ret_15m={ask_heavy['fwd_ret_15m_bps'].mean():.2f}), "
                      f"spread={bid_heavy['fwd_ret_15m_bps'].mean() - ask_heavy['fwd_ret_15m_bps'].mean():.2f} bps")

if __name__ == "__main__":
    main()