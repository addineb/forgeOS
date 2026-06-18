import csv
signals = {}
with open('/root/depthscope_out/sweepscope_v12_scorecard.csv') as f:
    r = csv.DictReader(f)
    for row in r:
        if row['verdict'] == 'PROMOTE':
            entry = row['entry']
            thresh = float(row['threshold'])
            net = float(row['net_pnl_bps'])
            trades = int(row['trades'])
            wr = float(row['win_rate'])
            dsr = float(row['dsr'])
            if entry not in signals:
                signals[entry] = []
            signals[entry].append((thresh, net, trades, wr, dsr))

print("PROMOTE signals sorted by best net PnL:")
for s in sorted(signals.keys(), key=lambda x: max(t[1] for t in signals[x]), reverse=True):
    best = max(signals[s], key=lambda x: x[1])
    dirs = sorted(set(t[0] for t in signals[s]))
    dir_labels = ['short' if t < 0 else 'long' if t > 0 else 'neutral' for t in dirs]
    print(f"  {s:30s} thresh={dirs} dir={dir_labels} best_net={best[1]:.1f} tr={best[2]} wr={best[3]:.1%} dsr={best[4]:.3f}")

print(f"\nTotal: {sum(len(v) for v in signals.values())}")
