#!/usr/bin/env python3
"""Deep dive into CVD mean-reversion signal and depth features."""

import pandas as pd
import numpy as np
import sys

def main():
    path = sys.argv[1] if len(sys.argv) > 1 else "/root/depthscope_out/stitched_vb10.csv"
    df = pd.read_csv(path)
    print(f"Dataset: {len(df)} volume bars, {df['ts'].iloc[0]} - {df['ts'].iloc[-1]}")
    print()

    # === CVD DELTA: the strongest signal ===
    print("=== CVD DELTA MEAN-REVERSION DEEP DIVE ===")
    print(f"  cvd_delta: mean={df['cvd_delta'].mean():.2f}, std={df['cvd_delta'].std():.2f}")
    print(f"  cvd_ratio: mean={df['cvd_ratio'].mean():.4f}, std={df['cvd_ratio'].std():.4f}")
    print()

    # Decile analysis with more detail
    df['q_cvd'] = pd.qcut(df['cvd_delta'], 10, labels=False, duplicates='drop')
    print("CVD Delta Decile -> Forward Returns (bps):")
    print(f"{'Decile':>7} {'cvd_delta':>12} {'n':>6} {'ret_15m':>10} {'ret_1h':>10} {'ret_4h':>10} {'win_rate_15m':>13}")
    for q in range(10):
        sub = df[df['q_cvd'] == q]
        win_rate = (sub['fwd_ret_15m_bps'] > 0).mean()
        print(f"  Q{q:>2} {sub['cvd_delta'].mean():>12.1f} {len(sub):>6} "
              f"{sub['fwd_ret_15m_bps'].mean():>10.2f} {sub['fwd_ret_1h_bps'].mean():>10.2f} "
              f"{sub['fwd_ret_4h_bps'].mean():>10.2f} {win_rate:>13.1%}")

    # Long-short: go long Q0, short Q9
    q0 = df[df['q_cvd'] == 0]
    q9 = df[df['q_cvd'] == 9]
    print(f"\n  Long Q0 / Short Q9 spread:")
    for ret_col, label in [('fwd_ret_15m_bps', '15m'), ('fwd_ret_1h_bps', '1h'), ('fwd_ret_4h_bps', '4h')]:
        spread = q0[ret_col].mean() - q9[ret_col].mean()
        net = spread - 9.0  # 9 bps round-trip taker fee
        print(f"    {label}: gross={spread:.2f} bps, net (after 9bps fee)={net:.2f} bps")

    # === EXTREME CVD: top/bottom 5% ===
    print("\n=== EXTREME CVD (top/bottom 5%) ===")
    p5 = df['cvd_delta'].quantile(0.05)
    p95 = df['cvd_delta'].quantile(0.95)
    extreme_sell = df[df['cvd_delta'] <= p5]
    extreme_buy = df[df['cvd_delta'] >= p95]
    print(f"  Bottom 5% (cvd_delta <= {p5:.0f}): n={len(extreme_sell)}, "
          f"ret_15m={extreme_sell['fwd_ret_15m_bps'].mean():.2f}, "
          f"ret_1h={extreme_sell['fwd_ret_1h_bps'].mean():.2f}")
    print(f"  Top 5% (cvd_delta >= {p95:.0f}): n={len(extreme_buy)}, "
          f"ret_15m={extreme_buy['fwd_ret_15m_bps'].mean():.2f}, "
          f"ret_1h={extreme_buy['fwd_ret_1h_bps'].mean():.2f}")
    spread_15m = extreme_sell['fwd_ret_15m_bps'].mean() - extreme_buy['fwd_ret_15m_bps'].mean()
    spread_1h = extreme_sell['fwd_ret_1h_bps'].mean() - extreme_buy['fwd_ret_1h_bps'].mean()
    print(f"  Spread: 15m={spread_15m:.2f} bps (net={spread_15m-9:.2f}), 1h={spread_1h:.2f} bps (net={spread_1h-9:.2f})")

    # === IMBALANCE FEATURES (using actual column names) ===
    print("\n=== IMBALANCE FEATURES ===")
    for col in ['full_imbalance', 'top5_imbalance', 'weighted_imbalance']:
        if col in df.columns:
            df[f'q_{col}'] = pd.qcut(df[col], 10, labels=False, duplicates='drop')
            print(f"\n  {col} decile analysis:")
            print(f"  {'Decile':>7} {col:>20} {'ret_15m':>10} {'ret_1h':>10} {'ret_4h':>10}")
            for q in range(10):
                sub = df[df[f'q_{col}'] == q]
                print(f"    Q{q:>2} {sub[col].mean():>20.4f} {sub['fwd_ret_15m_bps'].mean():>10.2f} "
                      f"{sub['fwd_ret_1h_bps'].mean():>10.2f} {sub['fwd_ret_4h_bps'].mean():>10.2f}")

    # === DEPTH BAND RATIOS ===
    print("\n=== DEPTH BAND ANALYSIS ===")
    # Compute bid/ask ratio from volume columns
    if 'bid_vol_top20' in df.columns and 'ask_vol_top20' in df.columns:
        df['depth_ratio_20'] = df['bid_vol_top20'] / df['ask_vol_top20'].replace(0, np.nan)
        df['q_dr20'] = pd.qcut(df['depth_ratio_20'].dropna(), 10, labels=False, duplicates='drop')
        print("  bid_vol/ask_vol (top20) decile analysis:")
        for q in range(10):
            sub = df[df['q_dr20'] == q]
            if len(sub) > 0:
                print(f"    Q{q:>2}: ratio={sub['depth_ratio_20'].mean():.3f}, "
                      f"ret_15m={sub['fwd_ret_15m_bps'].mean():.2f}, ret_1h={sub['fwd_ret_1h_bps'].mean():.2f}")

    # === CVD + IMBALANCE COMBO ===
    print("\n=== CVD + IMBALANCE COMBO ===")
    # Long: heavy selling (Q0-1) + bid-heavy imbalance
    # Short: heavy buying (Q8-9) + ask-heavy imbalance
    df['sell_signal'] = (df['cvd_delta'] < df['cvd_delta'].quantile(0.2)).astype(int)
    df['buy_signal'] = (df['cvd_delta'] > df['cvd_delta'].quantile(0.8)).astype(int)

    if 'full_imbalance' in df.columns:
        df['bid_heavy'] = (df['full_imbalance'] > df['full_imbalance'].quantile(0.6)).astype(int)
        df['ask_heavy'] = (df['full_imbalance'] < df['full_imbalance'].quantile(0.4)).astype(int)

        # Long: sell flow + bid-heavy book (absorption)
        long_mask = (df['sell_signal'] == 1) & (df['bid_heavy'] == 1)
        # Short: buy flow + ask-heavy book
        short_mask = (df['buy_signal'] == 1) & (df['ask_heavy'] == 1)

        print(f"  Long (sell flow + bid-heavy book): n={long_mask.sum()}, "
              f"ret_15m={df.loc[long_mask, 'fwd_ret_15m_bps'].mean():.2f}, "
              f"ret_1h={df.loc[long_mask, 'fwd_ret_1h_bps'].mean():.2f}")
        print(f"  Short (buy flow + ask-heavy book): n={short_mask.sum()}, "
              f"ret_15m={df.loc[short_mask, 'fwd_ret_15m_bps'].mean():.2f}, "
              f"ret_1h={df.loc[short_mask, 'fwd_ret_1h_bps'].mean():.2f}")
        if long_mask.sum() > 0 and short_mask.sum() > 0:
            spread = df.loc[long_mask, 'fwd_ret_15m_bps'].mean() - df.loc[short_mask, 'fwd_ret_15m_bps'].mean()
            print(f"  Long-Short spread: 15m={spread:.2f} bps (net={spread-9:.2f}), "
                  f"1h={df.loc[long_mask, 'fwd_ret_1h_bps'].mean() - df.loc[short_mask, 'fwd_ret_1h_bps'].mean():.2f}")

    # === WALL FEATURES ===
    print("\n=== WALL FEATURES ===")
    if 'active_wall_count' in df.columns:
        df['q_walls'] = pd.qcut(df['active_wall_count'], 5, labels=False, duplicates='drop')
        print("  active_wall_count quintile analysis:")
        for q in range(5):
            sub = df[df['q_walls'] == q]
            print(f"    Q{q}: walls={sub['active_wall_count'].mean():.1f}, "
                  f"ret_15m={sub['fwd_ret_15m_bps'].mean():.2f}, ret_1h={sub['fwd_ret_1h_bps'].mean():.2f}")

    # === DATE-BY-DATE STABILITY CHECK ===
    print("\n=== DATE-BY-DATE STABILITY: CVD Q0-Q9 spread ===")
    df['date'] = (df['ts'] // 86400000000000).astype(int)  # rough date grouping
    # Better: extract date from filename order
    dates = df['ts'].values
    # Group by approximate date
    df['bar_idx'] = range(len(df))
    # Use rolling windows of ~2000 bars (roughly 1 date)
    n = len(df)
    chunk_size = 2500
    print(f"  {'Chunk':>7} {'Bars':>6} {'Q0_ret15m':>10} {'Q9_ret15m':>10} {'Spread':>10} {'Net':>10}")
    for start in range(0, n, chunk_size):
        end = min(start + chunk_size, n)
        chunk = df.iloc[start:end]
        if len(chunk) < 100:
            continue
        q0 = chunk[chunk['cvd_delta'] <= chunk['cvd_delta'].quantile(0.1)]
        q9 = chunk[chunk['cvd_delta'] >= chunk['cvd_delta'].quantile(0.9)]
        if len(q0) < 5 or len(q9) < 5:
            continue
        s = q0['fwd_ret_15m_bps'].mean() - q9['fwd_ret_15m_bps'].mean()
        print(f"  {start//chunk_size:>7} {len(chunk):>6} {q0['fwd_ret_15m_bps'].mean():>10.2f} "
              f"{q9['fwd_ret_15m_bps'].mean():>10.2f} {s:>10.2f} {s-9:>10.2f}")

if __name__ == "__main__":
    main()