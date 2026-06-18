#!/usr/bin/env python3
"""Check what dates are in the stitched clean CSV vs what enriched CSVs exist."""
import csv, os, glob

def days_to_ymd(days):
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

# Dates in the stitched clean CSV
clean_dates = set()
with open("/root/depthscope_out/stitched_vb10_enriched_clean.csv") as f:
    r = csv.DictReader(f)
    for row in r:
        ts = int(float(row["ts"]))
        secs = ts // 1_000_000_000
        days_since = secs // 86400
        y, m, d = days_to_ymd(days_since)
        clean_dates.add(f"{y:04}-{m:02}-{d:02}")
    print(f"Total bars in clean CSV: {r.line_num - 1}")

# Enriched CSVs that exist
enriched_files = glob.glob("/root/depthscope_out/*_enriched.csv")
dates_with_enriched = set()
for f in enriched_files:
    base = os.path.basename(f)
    # pattern: BTCUSDT_YYYY-MM-DD_vb10_enriched.csv
    parts = base.replace("BTCUSDT_", "").split("_vb10")
    if parts and len(parts) > 0:
        dates_with_enriched.add(parts[0])

print(f"\nDates in clean CSV ({len(clean_dates)}):")
for d in sorted(clean_dates):
    print(f"  {d}")

print(f"\nAll enriched CSVs exist for ({len(dates_with_enriched)}):")
for d in sorted(dates_with_enriched):
    print(f"  {d} {'<- MISSING from clean' if d not in clean_dates else ''}")

missing_from_clean = dates_with_enriched - clean_dates
if missing_from_clean:
    print(f"\n*** {len(missing_from_clean)} dates have enriched data but NOT in clean CSV:")
    for d in sorted(missing_from_clean):
        print(f"  {d}")
else:
    print("\nAll enriched dates are in clean CSV.")
