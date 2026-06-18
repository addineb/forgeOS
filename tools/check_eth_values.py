#!/usr/bin/env python3
"""Check data values in ETH parquet"""
import pyarrow.parquet as pq
import pyarrow as pa

f = "/root/chd/data/ticks/ETHUSDT/bookDelta/2026-06-10/12.parquet"
t = pq.read_table(f)
print(f"BOOK: {len(t)} rows")
print(f"  Columns: {t.column_names}")
print(f"  Sample venueTs: {t.column('venueTs').to_pylist()[:5]}")
print(f"  Sample captureTs: {t.column('captureTs').to_pylist()[:5]}")
print(f"  Sample side: {t.column('side').to_pylist()[:5]}")
print(f"  Sample price: {t.column('price').to_pylist()[:5]}")
print(f"  Sample qty: {t.column('qty').to_pylist()[:5]}")
print(f"  Sample kind: {t.column('kind').to_pylist()[:5]}")

# Check for nulls
for col in t.column_names:
    nulls = t.column(col).null_count
    if nulls > 0:
        print(f"  {col}: {nulls} nulls")

# Check types
for col in t.column_names:
    print(f"  {col}: {t.column(col).type}")

f2 = "/root/chd/data/ticks/ETHUSDT/trade/2026-06-10/12.parquet"
t2 = pq.read_table(f2)
print(f"\nTRADE: {len(t2)} rows")
print(f"  Columns: {t2.column_names}")
for col in t2.column_names:
    print(f"  {col}: {t2.column(col).type}")
    try:
        print(f"    Sample: {t2.column(col).to_pylist()[:3]}")
    except:
        print(f"    (could not sample)")
