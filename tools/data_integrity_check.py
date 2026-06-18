#!/usr/bin/env python3
"""Data integrity check for the 3 failing dates vs normal dates."""

import pandas as pd
import numpy as np
import sys

def main():
    path = sys.argv[1] if len(sys.argv) > 1 else "/root/depthscope_out/stitched_vb10.csv"
    df = pd.read_csv(path)
    df['utc_date'] = pd.to_datetime(df['ts'], unit='ns').dt.date

    failing = [pd.Timestamp('2026-04-02').date(), pd.Timestamp('2026-04-03').date(), pd.Timestamp('2026-06-06').date()]
    good = [pd.Timestamp('2025-12-01').date(), pd.Timestamp('2026-06-05').date(), pd.Timestamp('2026-02-01').date()]

    print("=" * 70)
    print("DATA INTEGRITY CHECK: FAILING vs GOOD DATES")
    print("=" * 70)

    # 1. Basic stats comparison
    print("\n1. BASIC STATS COMPARISON")
    print(f"{'Date':>12} {'Bars':>6} {'Mid_start':>12} {'Mid_end':>12} {'Range%':>8} {'Avg_spread':>11} {'Avg_bid_lvl':>12} {'Avg_ask_lvl':>12} {'Avg_cvd_delta':>14}")
    
    for date in failing + good:
        sub = df[df['utc_date'] == date]
        if len(sub) == 0:
            print(f"  {date}: NO DATA")
            continue
        mid_start = sub['mid_price'].iloc[0]
        mid_end = sub['mid_price'].iloc[-1]
        range_pct = (mid_end - mid_start) / mid_start * 100
        avg_spread = sub['spread_bps'].mean()
        avg_bid_lvl = sub['bid_levels'].mean()
        avg_ask_lvl = sub['ask_levels'].mean()
        avg_cvd = sub['cvd_delta'].mean()
        print(f"  {str(date):>12} {len(sub):>6} {mid_start:>12.1f} {mid_end:>12.1f} {range_pct:>8.2f}% "
              f"{avg_spread:>11.2f} {avg_bid_lvl:>12.1f} {avg_ask_lvl:>12.1f} {avg_cvd:>14.2f}")

    # 2. Check for data gaps (time between consecutive bars)
    print("\n2. TIME GAP ANALYSIS (seconds between consecutive bars)")
    print(f"{'Date':>12} {'Mean_gap':>10} {'Median_gap':>12} {'Max_gap':>10} {'Gaps>600s':>11}")
    
    for date in failing + good:
        sub = df[df['utc_date'] == date].sort_values('ts').reset_index(drop=True)
        if len(sub) < 2:
            continue
        gaps_s = np.diff(sub['ts'].values) / 1e9  # nanoseconds to seconds
        mean_gap = gaps_s.mean()
        median_gap = np.median(gaps_s)
        max_gap = gaps_s.max()
        big_gaps = (gaps_s > 600).sum()  # gaps > 10 min
        print(f"  {str(date):>12} {mean_gap:>10.1f} {median_gap:>12.1f} {max_gap:>10.1f} {big_gaps:>11}")

    # 3. CVD delta distribution comparison
    print("\n3. CVD DELTA DISTRIBUTION")
    print(f"{'Date':>12} {'Mean':>10} {'Std':>10} {'Min':>10} {'Max':>10} {'Skew':>8} {'%negative':>10}")
    
    for date in failing + good:
        sub = df[df['utc_date'] == date]
        cvd = sub['cvd_delta']
        pct_neg = (cvd < 0).mean()
        print(f"  {str(date):>12} {cvd.mean():>10.1f} {cvd.std():>10.1f} {cvd.min():>10.1f} {cvd.max():>10.1f} "
              f"{cvd.skew():>8.3f} {pct_neg:>10.1%}")

    # 4. Forward return distribution
    print("\n4. FORWARD RETURN DISTRIBUTION (15m)")
    print(f"{'Date':>12} {'Mean':>10} {'Std':>10} {'Min':>10} {'Max':>10} {'Skew':>8} {'%positive':>10}")
    
    for date in failing + good:
        sub = df[df['utc_date'] == date]
        ret = sub['fwd_ret_15m_bps']
        pct_pos = (ret > 0).mean()
        print(f"  {str(date):>12} {ret.mean():>10.2f} {ret.std():>10.2f} {ret.min():>10.2f} {ret.max():>10.2f} "
              f"{ret.skew():>8.3f} {pct_pos:>10.1%}")

    # 5. Price trend analysis — was it a trending day?
    print("\n5. PRICE TREND ANALYSIS")
    print(f"{'Date':>12} {'Trend_bps':>11} {'Abs_trend':>10} {'Max_dd_bps':>12} {'Trending?':>10}")
    
    for date in failing + good:
        sub = df[df['utc_date'] == date].sort_values('ts').reset_index(drop=True)
        if len(sub) < 2:
            continue
        # Overall trend in bps
        start_mid = sub['mid_price'].iloc[0]
        end_mid = sub['mid_price'].iloc[-1]
        trend_bps = (end_mid - start_mid) / start_mid * 10000
        # Max drawdown from running high
        running_max = sub['mid_price'].cummax()
        dd = (sub['mid_price'] - running_max) / running_max * 10000
        max_dd = dd.min()
        trending = "YES" if abs(trend_bps) > 200 else "no"
        print(f"  {str(date):>12} {trend_bps:>11.1f} {abs(trend_bps):>10.1f} {max_dd:>12.1f} {trending:>10}")

    # 6. CVD signal specifically on failing dates — does it invert?
    print("\n6. CVD SIGNAL ON FAILING DATES (decile analysis)")
    for date in failing:
        sub = df[df['utc_date'] == date].copy()
        if len(sub) < 100:
            continue
        sub['q'] = pd.qcut(sub['cvd_delta'], 5, labels=False, duplicates='drop')
        print(f"\n  {date} (n={len(sub)}):")
        print(f"  {'Q':>4} {'cvd_delta':>12} {'ret_15m':>10} {'ret_1h':>10} {'win_15m':>8}")
        for q in range(5):
            qs = sub[sub['q'] == q]
            win = (qs['fwd_ret_15m_bps'] > 0).mean()
            print(f"  Q{q:>2} {qs['cvd_delta'].mean():>12.1f} {qs['fwd_ret_15m_bps'].mean():>10.2f} "
                  f"{qs['fwd_ret_1h_bps'].mean():>10.2f} {win:>8.1%}")

    # 7. Check for duplicate timestamps or out-of-order data
    print("\n7. DATA QUALITY FLAGS")
    for date in failing + good:
        sub = df[df['utc_date'] == date].sort_values('ts').reset_index(drop=True)
        dups = sub['ts'].duplicated().sum()
        out_of_order = (np.diff(sub['ts'].values) < 0).sum()
        zero_mid = (sub['mid_price'] == 0).sum()
        zero_spread = (sub['spread_bps'] == 0).sum()
        low_levels = ((sub['bid_levels'] < 50) | (sub['ask_levels'] < 50)).sum()
        print(f"  {date}: dup_ts={dups}, out_of_order={out_of_order}, zero_mid={zero_mid}, "
              f"zero_spread={zero_spread}, low_levels={low_levels}")

if __name__ == "__main__":
    main()
