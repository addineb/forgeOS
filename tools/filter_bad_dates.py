#!/usr/bin/env python3
"""Filter bad dates from stitched CSV and re-stitch clean version."""
import csv, os, sys

def days_to_ymd(days):
    """Convert days since Unix epoch to (year, month, day)."""
    days += 719468
    era = days // 146097 if days >= 0 else (days - 146096) // 146097
    day_of_era = days - era * 146097
    year_of_era = (day_of_era - day_of_era // 1460 + day_of_era // 36524 - day_of_era // 146096) // 365
    day_of_year = day_of_era - (365 * year_of_era + year_of_era // 4 - year_of_era // 100)
    month = (5 * day_of_year + 2) // 153
    day = day_of_year - (153 * month + 2) // 5 + 1
    year = year_of_era + era * 400 + (0 if month < 10 else 1)
    month = month + 3 if month < 10 else month - 9
    return year, month, day

# Bad dates to exclude (no HL funding/OI data)
BAD_DATES = {"2026-03-01", "2026-04-01", "2026-04-02", "2026-04-03"}

INPUT = "/root/depthscope_out/stitched_vb10_enriched.csv"
OUTPUT = "/root/depthscope_out/stitched_vb10_enriched_clean.csv"

bad_count = 0
good_count = 0

with open(INPUT) as fin, open(OUTPUT, "w", newline="") as fout:
    reader = csv.DictReader(fin)
    writer = csv.DictWriter(fout, fieldnames=reader.fieldnames)
    writer.writeheader()
    for row in reader:
        # Extract date from ts (Unix nanoseconds -> UTC date)
        ts = int(float(row["ts"]))
        secs = ts // 1_000_000_000
        days_since_epoch = secs // 86400
        y, m, d = days_to_ymd(days_since_epoch)
        date_str = f"{y:04}-{m:02}-{d:02}"
        if date_str in BAD_DATES:
            bad_count += 1
        else:
            writer.writerow(row)
            good_count += 1

print(f"Bad dates excluded: {bad_count} bars")
print(f"Good dates kept: {good_count} bars")
print(f"Output: {OUTPUT}")
