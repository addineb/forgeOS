#!/usr/bin/env python3
import pyarrow.parquet as pq

# Check bookDelta schema and sample data
bd = pq.read_table('/root/chd/data/ticks/BTCUSDT/bookDelta/2026-06-09/00.parquet')
print("=== bookDelta schema ===")
print(bd.schema)
print("Columns:", bd.column_names)
print("Rows:", bd.num_rows)
# Show kind values
kind_col = bd.column('kind')
unique_kinds = set(kind_col.to_pylist())
print("Kind values:", unique_kinds)
# Show first 3 rows
print("\nFirst 3 rows:")
for i in range(min(3, bd.num_rows)):
    row = {col: bd.column(col)[i].as_py() for col in bd.column_names}
    print(row)

# Check trade schema
tr = pq.read_table('/root/chd/data/ticks/BTCUSDT/trade/2026-06-09/00.parquet')
print("\n=== trade schema ===")
print(tr.schema)
print("Columns:", tr.column_names)
print("Rows:", tr.num_rows)
print("\nFirst 3 rows:")
for i in range(min(3, tr.num_rows)):
    row = {col: tr.column(col)[i].as_py() for col in tr.column_names}
    print(row)

# Check what dates we have full coverage for
import os
bd_dir = '/root/chd/data/ticks/BTCUSDT/bookDelta'
print("\n=== bookDelta date coverage ===")
for d in sorted(os.listdir(bd_dir)):
    hours = os.listdir(os.path.join(bd_dir, d))
    total_size = sum(os.path.getsize(os.path.join(bd_dir, d, h)) for h in hours)
    print(f"  {d}: {len(hours)} hours, {total_size/1e6:.0f}MB")

tr_dir = '/root/chd/data/ticks/BTCUSDT/trade'
print("\n=== trade date coverage ===")
for d in sorted(os.listdir(tr_dir)):
    hours = os.listdir(os.path.join(tr_dir, d))
    total_size = sum(os.path.getsize(os.path.join(tr_dir, d, h)) for h in hours)
    print(f"  {d}: {len(hours)} hours, {total_size/1e6:.0f}MB")