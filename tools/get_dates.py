#!/usr/bin/env python3
"""Get actual dates in CSV."""
import csv

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

dates = set()
with open("/root/depthscope_out/stitched_vb10_enriched.csv") as f:
    reader = csv.DictReader(f)
    for row in reader:
        ts = int(float(row["ts"]))
        secs = ts // 1_000_000_000
        days = secs // 86400
        y, m, d = days_to_ymd(days)
        dates.add(f"{y:04}-{m:02}-{d:02}")

print(sorted(dates))
