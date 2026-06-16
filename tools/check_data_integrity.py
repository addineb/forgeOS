#!/usr/bin/env python3
"""Check data integrity: snapshots, gaps, and book reconstruction feasibility."""
import pyarrow.parquet as pq
import os

bd_dir = '/root/chd/data/ticks/BTCUSDT/bookDelta'
tr_dir = '/root/chd/data/ticks/BTCUSDT/trade'

# Check a full day (2026-06-09) for snapshots vs deltas
print("=== Checking 2026-06-09 bookDelta data ===")
for hh in ['00', '01', '02', '03', '04', '05', '06', '07', '08', '09', '10', '11', '12']:
    path = os.path.join(bd_dir, '2026-06-09', f'{hh}.parquet')
    if not os.path.exists(path):
        print(f"  {hh}: MISSING")
        continue
    t = pq.read_table(path)
    kinds = set(t.column('kind').to_pylist())
    sides = set(t.column('side').to_pylist())
    n = t.num_rows
    # Check for snapshots (kind=snapshot or similar)
    has_snapshot = 'snapshot' in kinds or 'Snapshot' in kinds or 'SNAPSHOT' in kinds
    print(f"  {hh}: {n} rows, kinds={kinds}, sides={sides}, has_snapshot={has_snapshot}")

# Check first few rows of hour 00 for initial state
print("\n=== First 10 rows of hour 00 ===")
t = pq.read_table(os.path.join(bd_dir, '2026-06-09', '00.parquet'))
for i in range(min(10, t.num_rows)):
    row = {col: t.column(col)[i].as_py() for col in t.column_names}
    print(f"  {i}: venueTs={row['venueTs']}, side={row['side']}, price={row['price']:.2f}, qty={row['qty']:.6f}, kind={row['kind']}")

# Check if first row is a snapshot (qty > 0 at many levels = initial state)
print("\n=== Checking if hour starts with a snapshot (many levels at once) ===")
t = pq.read_table(os.path.join(bd_dir, '2026-06-09', '00.parquet'))
first_ts = t.column('venueTs')[0].as_py()
first_ts_count = sum(1 for i in range(t.num_rows) if t.column('venueTs')[i].as_py() == first_ts)
print(f"  First venueTs: {first_ts}, rows with same ts: {first_ts_count}")

# Check for time gaps (missing hours)
print("\n=== Checking all dates for hour completeness ===")
for d in sorted(os.listdir(bd_dir)):
    hours = os.listdir(os.path.join(bd_dir, d))
    hour_nums = sorted([h.replace('.parquet', '') for h in hours])
    missing = [f"{i:02d}" for i in range(24) if f"{i:02d}" not in hour_nums]
    if missing:
        print(f"  {d}: {len(hours)}/24 hours, MISSING: {missing}")
    else:
        print(f"  {d}: 24/24 hours COMPLETE")

# Check trade data integrity
print("\n=== Trade data sample (2026-06-09 hour 00) ===")
t = pq.read_table(os.path.join(tr_dir, '2026-06-09', '00.parquet'))
for i in range(min(5, t.num_rows)):
    row = {col: t.column(col)[i].as_py() for col in t.column_names}
    print(f"  {i}: ts={row['ts']}, price={row['price']:.2f}, qty={row['qty']:.6f}, isBuyerMaker={row['isBuyerMaker']}")

# Check if we need to pull fresh data with snapshots
print("\n=== CHD raw data check (what the converter sees) ===")
import zstandard as zstd
import io
import pandas as pd

# Check if the raw CHD download script exists and what it pulls
conv_path = '/root/forgeOS/tools/chd-to-parquet.py'
if os.path.exists(conv_path):
    print(f"  Converter exists at {conv_path}")
else:
    print(f"  Converter NOT at {conv_path}")

# Check available disk space
print(f"\n=== Disk space ===")
os.system("df -h /root/chd/")