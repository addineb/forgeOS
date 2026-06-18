#!/usr/bin/env python3
"""Check NaN density in clean stitched CSV."""
import csv

nan_counts = {}
total = 0
with open("/root/depthscope_out/stitched_vb10_enriched_clean.csv") as f:
    reader = csv.DictReader(f)
    header = reader.fieldnames
    for row in reader:
        total += 1
        for col in header:
            val = row.get(col, "")
            if val == "" or val == "NaN" or val == "nan":
                nan_counts[col] = nan_counts.get(col, 0) + 1

print(f"Total rows: {total}")
print("NaN density (columns with >1% NaN):")
for col, count in sorted(nan_counts.items(), key=lambda x: -x[1]):
    pct = 100 * count / total
    if pct > 1:
        print(f"  {col}: {count} ({pct:.1f}%)")
