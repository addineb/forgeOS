#!/usr/bin/env python3
"""
Stitch forward returns across dates.

depthscope computes forward returns per-date, so the 4hr return at 20:00 UTC
is 0 because there's no next-day data loaded. This script:
1. Reads all per-date CSVs in chronological order
2. Merges them into one big DataFrame
3. Recomputes forward returns using the full timeline
4. Writes the stitched output

Usage:
    python stitch_forward_returns.py /root/depthscope_out/ /root/depthscope_out/stitched_vb10.csv
"""

import sys
import glob
import os
import pandas as pd
import numpy as np


def load_csvs(directory: str) -> pd.DataFrame:
    """Load all depthscope CSVs from directory, sorted by filename (date)."""
    files = sorted(glob.glob(os.path.join(directory, "BTCUSDT_*_vb10.csv")))
    if not files:
        # Try time-bar files too
        files = sorted(glob.glob(os.path.join(directory, "BTCUSDT_*_tb60s.csv")))
    if not files:
        print(f"No CSV files found in {directory}")
        sys.exit(1)

    print(f"Loading {len(files)} files...")
    dfs = []
    for f in files:
        df = pd.read_csv(f)
        date_str = os.path.basename(f).split("_")[1]
        print(f"  {date_str}: {len(df)} rows")
        dfs.append(df)

    combined = pd.concat(dfs, ignore_index=True)
    combined = combined.sort_values("ts").reset_index(drop=True)
    print(f"Total: {len(combined)} rows, time range: {combined['ts'].iloc[0]} - {combined['ts'].iloc[-1]}")
    return combined


def recompute_forward_returns(df: pd.DataFrame) -> pd.DataFrame:
    """Recompute forward returns using the full stitched timeline.
    Fully vectorized with numpy searchsorted — O(n log n)."""
    # Forward return horizons in nanoseconds
    horizons = [
        ("fwd_ret_15m_bps", 15 * 60 * 1_000_000_000),
        ("fwd_ret_1h_bps", 60 * 60 * 1_000_000_000),
        ("fwd_ret_4h_bps", 4 * 60 * 60 * 1_000_000_000),
    ]

    ts = df["ts"].values.astype(np.int64)
    mid = df["mid_price"].values.astype(np.float64)
    n = len(df)
    idx = np.arange(n, dtype=np.int64)

    for col, horizon_ns in horizons:
        print(f"  Computing {col}...")
        target_ts = ts + horizon_ns

        # searchsorted: first index j where ts[j] >= target_ts[i]
        right = np.searchsorted(ts, target_ts, side="left")
        left = np.maximum(right - 1, 0)

        # Clamp to valid range [0, n-1]
        right = np.minimum(right, n - 1)

        # Pick closer of left/right (but must be > i and < n)
        diff_left = np.abs(ts[left] - target_ts)
        diff_right = np.abs(ts[right] - target_ts)

        # Prefer right (first snapshot at or after target) unless left is closer
        pick_right = diff_right <= diff_left
        best = np.where(pick_right, right, left)

        # Zero out where best <= i (no future snapshot) or best >= n
        valid = (best > idx) & (best < n)
        best_clamped = np.where(valid, best, 0)  # safe index for array lookup

        fwd_mid = mid[best_clamped]
        fwd = np.where(valid & (mid > 0), (fwd_mid - mid) / mid * 10000.0, 0.0)
        df[col] = fwd

    return df


def main():
    if len(sys.argv) < 3:
        print("Usage: python stitch_forward_returns.py <input_dir> <output_csv>")
        sys.exit(1)

    input_dir = sys.argv[1]
    output_csv = sys.argv[2]

    # Load all CSVs
    df = load_csvs(input_dir)

    # Drop old forward return columns (they're per-date, we'll recompute)
    fwd_cols = ["fwd_ret_15m_bps", "fwd_ret_1h_bps", "fwd_ret_4h_bps"]
    for col in fwd_cols:
        if col in df.columns:
            df.drop(columns=[col], inplace=True)

    # Recompute with full timeline
    print("Recomputing forward returns across dates...")
    df = recompute_forward_returns(df)

    # Write output
    df.to_csv(output_csv, index=False)
    print(f"Written {len(df)} rows to {output_csv}")

    # Quick stats
    for col in fwd_cols:
        nonzero = (df[col] != 0).sum()
        print(f"  {col}: {nonzero}/{len(df)} non-zero ({100*nonzero/len(df):.1f}%)")


if __name__ == "__main__":
    main()
