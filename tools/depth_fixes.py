#!/usr/bin/env python3
"""Immediate fixes: (1) identify negative date chunks, (2) CVD threshold optimization."""

import pandas as pd
import numpy as np
import sys

def main():
    path = sys.argv[1] if len(sys.argv) > 1 else "/root/depthscope_out/stitched_vb10.csv"
    df = pd.read_csv(path)
    print(f"Dataset: {len(df)} bars")
    print()

    # === FIX 1: Map chunks to actual dates ===
    print("=" * 70)
    print("FIX 1: IDENTIFY NEGATIVE DATE CHUNKS")
    print("=" * 70)

    # Convert ts (nanoseconds) to approximate UTC date
    df['utc_date'] = pd.to_datetime(df['ts'], unit='ns').dt.date

    # Per-date analysis of CVD Q0-Q9 spread
    print("\nPer-date CVD mean-reversion signal:")
    print(f"{'Date':>12} {'Bars':>6} {'Q0_ret15m':>10} {'Q9_ret15m':>10} {'Spread':>10} {'Net':>10} {'Q0_n':>6} {'Q9_n':>6} {'Q0_win%':>8}")
    
    dates_with_signal = []
    for date, group in df.groupby('utc_date'):
        q10 = group['cvd_delta'].quantile(0.1)
        q90 = group['cvd_delta'].quantile(0.9)
        q0 = group[group['cvd_delta'] <= q10]
        q9 = group[group['cvd_delta'] >= q90]
        if len(q0) < 5 or len(q9) < 5:
            continue
        spread = q0['fwd_ret_15m_bps'].mean() - q9['fwd_ret_15m_bps'].mean()
        net = spread - 9.0
        win_rate = (q0['fwd_ret_15m_bps'] > 0).mean()
        dates_with_signal.append({
            'date': date, 'bars': len(group), 'spread': spread, 'net': net,
            'q0_ret': q0['fwd_ret_15m_bps'].mean(), 'q9_ret': q9['fwd_ret_15m_bps'].mean(),
            'q0_n': len(q0), 'q9_n': len(q9), 'win_rate': win_rate
        })
        print(f"  {str(date):>12} {len(group):>6} {q0['fwd_ret_15m_bps'].mean():>10.2f} "
              f"{q9['fwd_ret_15m_bps'].mean():>10.2f} {spread:>10.2f} {net:>10.2f} "
              f"{len(q0):>6} {len(q9):>6} {win_rate:>8.1%}")

    # Summary
    positive = sum(1 for d in dates_with_signal if d['net'] > 0)
    negative = sum(1 for d in dates_with_signal if d['net'] <= 0)
    print(f"\n  Positive net: {positive}/{len(dates_with_signal)} dates")
    print(f"  Negative/zero net: {negative}/{len(dates_with_signal)} dates")
    
    if negative > 0:
        print(f"\n  *** NEGATIVE DATES (signal fails) ***")
        for d in dates_with_signal:
            if d['net'] <= 0:
                print(f"    {d['date']}: spread={d['spread']:.2f}, net={d['net']:.2f}, bars={d['bars']}")

    # Also check 1h returns per date
    print(f"\nPer-date 1h CVD signal:")
    print(f"{'Date':>12} {'Spread_1h':>10} {'Net_1h':>10}")
    for date, group in df.groupby('utc_date'):
        q10 = group['cvd_delta'].quantile(0.1)
        q90 = group['cvd_delta'].quantile(0.9)
        q0 = group[group['cvd_delta'] <= q10]
        q9 = group[group['cvd_delta'] >= q90]
        if len(q0) < 5 or len(q9) < 5:
            continue
        spread_1h = q0['fwd_ret_1h_bps'].mean() - q9['fwd_ret_1h_bps'].mean()
        print(f"  {str(date):>12} {spread_1h:>10.2f} {spread_1h - 9:>10.2f}")

    # === FIX 2: CVD THRESHOLD OPTIMIZATION ===
    print("\n" + "=" * 70)
    print("FIX 2: CVD THRESHOLD OPTIMIZATION")
    print("=" * 70)

    # Test different entry thresholds for long side (cvd_delta < X)
    print("\nLong side: cvd_delta < threshold")
    print(f"{'Threshold':>10} {'n':>6} {'ret_15m':>10} {'ret_1h':>10} {'win_15m':>8} {'net_15m':>10} {'net_1h':>10}")
    
    thresholds_long = [-1000, -1500, -2000, -2500, -3000, -3500, -4000, -4500, -5000]
    for t in thresholds_long:
        sub = df[df['cvd_delta'] < t]
        if len(sub) < 10:
            continue
        ret_15m = sub['fwd_ret_15m_bps'].mean()
        ret_1h = sub['fwd_ret_1h_bps'].mean()
        win_15m = (sub['fwd_ret_15m_bps'] > 0).mean()
        # Net after 4.5 bps one-way fee (long only, not long-short)
        net_15m = ret_15m - 4.5
        net_1h = ret_1h - 4.5
        print(f"  {t:>10} {len(sub):>6} {ret_15m:>10.2f} {ret_1h:>10.2f} {win_15m:>8.1%} {net_15m:>10.2f} {net_1h:>10.2f}")

    # Short side: cvd_delta > threshold
    print("\nShort side: cvd_delta > threshold")
    print(f"{'Threshold':>10} {'n':>6} {'ret_15m':>10} {'ret_1h':>10} {'win_15m':>8} {'net_15m':>10} {'net_1h':>10}")
    
    thresholds_short = [0, 100, 200, 300, 400, 500, 600, 700, 800]
    for t in thresholds_short:
        sub = df[df['cvd_delta'] > t]
        if len(sub) < 10:
            continue
        # Short: profit when ret is negative
        ret_15m = -sub['fwd_ret_15m_bps'].mean()  # flip sign for short PnL
        ret_1h = -sub['fwd_ret_1h_bps'].mean()
        win_15m = (sub['fwd_ret_15m_bps'] < 0).mean()
        net_15m = ret_15m - 4.5
        net_1h = ret_1h - 4.5
        print(f"  {t:>10} {len(sub):>6} {ret_15m:>10.2f} {ret_1h:>10.2f} {win_15m:>8.1%} {net_15m:>10.2f} {net_1h:>10.2f}")

    # Long-short combined at various thresholds
    print("\nLong-Short combined (long below -X, short above +Y):")
    print(f"{'Long<':>8} {'Short>':>8} {'n_long':>7} {'n_short':>8} {'gross_15m':>10} {'net_15m':>10} {'gross_1h':>10} {'net_1h':>10}")
    
    for long_t in [-2000, -2500, -3000, -3500, -4000]:
        for short_t in [200, 300, 400, 500, 600]:
            longs = df[df['cvd_delta'] < long_t]
            shorts = df[df['cvd_delta'] > short_t]
            if len(longs) < 10 or len(shorts) < 10:
                continue
            # Long PnL = ret, Short PnL = -ret
            long_pnl_15m = longs['fwd_ret_15m_bps'].mean()
            short_pnl_15m = -shorts['fwd_ret_15m_bps'].mean()
            long_pnl_1h = longs['fwd_ret_1h_bps'].mean()
            short_pnl_1h = -shorts['fwd_ret_1h_bps'].mean()
            
            # Weighted by sample size
            total_n = len(longs) + len(shorts)
            gross_15m = (long_pnl_15m * len(longs) + short_pnl_15m * len(shorts)) / total_n
            gross_1h = (long_pnl_1h * len(longs) + short_pnl_1h * len(shorts)) / total_n
            net_15m = gross_15m - 9.0  # round-trip fee
            net_1h = gross_1h - 9.0
            
            print(f"  {long_t:>8} {short_t:>8} {len(longs):>7} {len(shorts):>8} "
                  f"{gross_15m:>10.2f} {net_15m:>10.2f} {gross_1h:>10.2f} {net_1h:>10.2f}")

    # === MAKER VS TAKER FEE COMPARISON ===
    print("\n" + "=" * 70)
    print("MAKER vs TAKER FEE IMPACT")
    print("=" * 70)
    print("  HL taker fee: ~4.5 bps one-way (9 bps round-trip)")
    print("  HL maker fee: ~2.5 bps one-way (5 bps round-trip)")
    print()
    
    # Best long-short combo
    best_long_t = -3000
    best_short_t = 400
    longs = df[df['cvd_delta'] < best_long_t]
    shorts = df[df['cvd_delta'] > best_short_t]
    long_pnl_15m = longs['fwd_ret_15m_bps'].mean()
    short_pnl_15m = -shorts['fwd_ret_15m_bps'].mean()
    total_n = len(longs) + len(shorts)
    gross_15m = (long_pnl_15m * len(longs) + short_pnl_15m * len(shorts)) / total_n
    
    print(f"  Best combo (long<{best_long_t}, short>{best_short_t}):")
    print(f"    Gross: {gross_15m:.2f} bps")
    print(f"    Net (taker): {gross_15m - 9:.2f} bps")
    print(f"    Net (maker): {gross_15m - 5:.2f} bps")
    print(f"    Trades: {len(longs)} long + {len(shorts)} short = {total_n} over 18 dates")
    print(f"    Avg trades/day: {total_n/18:.0f}")

if __name__ == "__main__":
    main()
