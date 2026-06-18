#!/usr/bin/env python3
"""Quick check of sweepscope scorecard - show top configs with 30+ trades."""
import csv, sys

path = sys.argv[1] if len(sys.argv) > 1 else '/root/depthscope_out/sweepscope_scorecard.csv'
with open(path) as f:
    reader = csv.DictReader(f)
    rows = [r for r in reader if int(r['trades']) >= 30]

rows.sort(key=lambda r: float(r['net_pnl_bps']), reverse=True)
print(f"Configs with 30+ trades: {len(rows)}")
print()
for r in rows[:15]:
    print(f"id={r['id']:>4} entry={r['entry']:<20} thresh={r['threshold']:>4} TP={r['tp_bps']:>5} SL={r['sl_bps']:>5} hold={r['hold_bars']:>3} "
          f"trades={r['trades']:>3} net={float(r['net_pnl_bps']):>7.1f} sharpe={r['sharpe']:>5} dsr={r['dsr']:>5} pbo={r['pbo']:>5} "
          f"oos_net={r['oos_net_bps']:>7} verdict={r['verdict']}")
