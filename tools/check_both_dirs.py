import csv

with open('/root/depthscope_out/sweepscope_v12_scorecard.csv') as f:
    r = csv.DictReader(f)
    by_family = {}
    
    for row in r:
        entry = row['entry']
        # Extract family base name: strip window and direction suffixes
        base = entry
        for sfx in ['_25', '_50', '_100']:
            if base.endswith(sfx):
                base = base[:-len(sfx)]
                break
        dir_sfx = None
        for sfx in ['_short', '_sell', '_long', '_buy', '_absorb', '_discount']:
            if base.endswith(sfx):
                dir_sfx = sfx
                base = base[:-len(sfx)]
                break
        # Determine direction
        if dir_sfx in ('_short', '_sell'):
            dir_type = 'SHORT'
        elif dir_sfx in ('_long', '_buy', '_absorb', '_discount'):
            dir_type = 'LONG'
        else:
            dir_type = 'UNK'
        
        net = float(row['net_pnl_bps'])
        verdict = row['verdict']
        trades = int(row['trades'])
        wr = float(row['win_rate'])
        
        if base not in by_family:
            by_family[base] = {}
        if dir_type not in by_family[base]:
            by_family[base][dir_type] = {'max_net': -999, 'promoted': False, 'trades': 0, 'wr': 0}
        
        by_family[base][dir_type]['max_net'] = max(by_family[base][dir_type]['max_net'], net)
        by_family[base][dir_type]['trades'] = max(by_family[base][dir_type]['trades'], trades)
        by_family[base][dir_type]['wr'] = max(by_family[base][dir_type]['wr'], wr)
        if verdict == 'PROMOTE':
            by_family[base][dir_type]['promoted'] = True

print(f"{'FAMILY':<28s} {'SHORT net':>10s} {'prom':>5s} {'LONG net':>10s} {'prom':>5s} {'WORKS?':>8s}")
print('-' * 75)
for base in sorted(by_family.keys()):
    short = by_family[base].get('SHORT', {'max_net': -999, 'promoted': False, 'trades': 0, 'wr': 0})
    long = by_family[base].get('LONG', {'max_net': -999, 'promoted': False, 'trades': 0, 'wr': 0})
    
    s_net = short['max_net']
    l_net = long['max_net']
    s_prom = 'YES' if short['promoted'] else 'no'
    l_prom = 'YES' if long['promoted'] else 'no'
    
    if s_prom == 'YES' and l_prom == 'YES':
        works = 'BOTH!'
    elif s_prom == 'YES':
        works = 'short'
    elif l_prom == 'YES':
        works = 'long'
    else:
        works = 'neither'
    
    print(f"{base:<28s} {s_net:>10.1f} {s_prom:>5s} {l_net:>10.1f} {l_prom:>5s} {works:>8s}")

# Count total
prom_count = sum(1 for p in by_family.values() if p.get('SHORT', {}).get('promoted', False) or p.get('LONG', {}).get('promoted', False))
both_count = sum(1 for p in by_family.values() if p.get('SHORT', {}).get('promoted', False) and p.get('LONG', {}).get('promoted', False))
print(f"\nPromoted in any direction: {prom_count}")
print(f"Promoted in BOTH: {both_count}")
print(f"Only short: {sum(1 for p in by_family.values() if p.get('SHORT', {}).get('promoted', False) and not p.get('LONG', {}).get('promoted', False))}")
print(f"Only long: {sum(1 for p in by_family.values() if not p.get('SHORT', {}).get('promoted', False) and p.get('LONG', {}).get('promoted', False))}")
