import csv
first = None
last = None
with open('/root/depthscope_out/stitched_vb10_enriched_clean.csv') as f:
    r = csv.DictReader(f)
    for row in r:
        p = float(row['mid_price'])
        if first is None:
            first = p
        last = p
chg = (last / first - 1) * 100
print(f"First mid: {first:.2f}, Last mid: {last:.2f}, Change: {chg:+.1f}%")
print(f"Dates in CSV: 14 (Dec 2025 - Jun 2026, excluding 4 bad dates)")
