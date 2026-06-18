import csv
signals = {}
with open('/root/depthscope_out/sweepscope_v12_scorecard.csv') as f:
    r = csv.DictReader(f)
    for row in r:
        if row['verdict'] == 'PROMOTE':
            entry = row['entry']
            thresh = float(row['threshold'])
            net = float(row['net_pnl_bps'])
            if entry not in signals:
                signals[entry] = []
            signals[entry].append((thresh, net))

for s in sorted(signals.keys()):
    thresholds = sorted(set(t for t,_ in signals[s]))
    best = max(signals[s], key=lambda x: x[1])
    dirs = []
    for t in thresholds:
        if t > 0: dirs.append('long')
        elif t < 0: dirs.append('short')
        else: dirs.append('neutral')
    print(f"{s}: thresholds={thresholds} dirs={dirs} best={best[1]:.1f}bps")

print(f"\nTotal configs promoted: {sum(len(v) for v in signals.values())}")
