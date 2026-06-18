import csv
sigs=set()
with open('/root/depthscope_out/sweepscope_v12_scorecard.csv') as f:
    r=csv.DictReader(f)
    for row in r:
        if row['verdict']=='PROMOTE':
            sigs.add(row['entry'])
for s in sorted(sigs):
    print(s)
print(f'\nTotal unique signals: {len(sigs)}')
