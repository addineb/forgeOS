"""Check what forward returns look like in % terms and which signals predict BIG moves."""
import pandas as pd, numpy as np

df = pd.read_csv('/root/depthscope_out/stitched_vb10.csv')

print('=== FORWARD RETURN DISTRIBUTION (percentage) ===')
for col in ['fwd_ret_15m_bps','fwd_ret_1h_bps','fwd_ret_4h_bps']:
    pct = df[col] / 100  # bps to pct
    print(f'{col}:')
    print(f'  mean={pct.mean():.3f}%  std={pct.std():.3f}%')
    print(f'  |ret|>0.5%: {(pct.abs()>0.5).sum()} ({(pct.abs()>0.5).mean()*100:.1f}%)')
    print(f'  |ret|>1.0%: {(pct.abs()>1.0).sum()} ({(pct.abs()>1.0).mean()*100:.1f}%)')
    print(f'  |ret|>2.0%: {(pct.abs()>2.0).sum()} ({(pct.abs()>2.0).mean()*100:.1f}%)')
    print(f'  |ret|>3.0%: {(pct.abs()>3.0).sum()} ({(pct.abs()>3.0).mean()*100:.1f}%)')
    print()

# CVD delta extreme vs big moves
print('=== CVD DELTA EXTREME vs BIG MOVES (1h fwd, %) ===')
for thresh in [-2000, -3000, -4000, -5000, -6000, -8000]:
    mask = df['cvd_delta'] < thresh
    n = mask.sum()
    if n == 0: continue
    rets = df.loc[mask, 'fwd_ret_1h_bps'] / 100
    big_up = (rets > 1.0).sum()
    big_dn = (rets < -1.0).sum()
    print(f'  cvd_delta < {thresh}: {n} bars, mean 1h={rets.mean():+.3f}%, >+1%:{big_up} ({big_up/n*100:.0f}%), <-1%:{big_dn} ({big_dn/n*100:.0f}%)')

# CVD delta extreme SHORT (positive = selling pressure)
print()
print('=== CVD DELTA POSITIVE (sell pressure) vs BIG MOVES (1h fwd, %) ===')
for thresh in [2000, 3000, 4000, 5000, 6000, 8000]:
    mask = df['cvd_delta'] > thresh
    n = mask.sum()
    if n == 0: continue
    rets = df.loc[mask, 'fwd_ret_1h_bps'] / 100
    big_up = (rets > 1.0).sum()
    big_dn = (rets < -1.0).sum()
    print(f'  cvd_delta > {thresh}: {n} bars, mean 1h={rets.mean():+.3f}%, >+1%:{big_up} ({big_up/n*100:.0f}%), <-1%:{big_dn} ({big_dn/n*100:.0f}%)')

# Imbalance extreme
print()
print('=== FULL IMBALANCE EXTREME vs BIG MOVES (1h fwd, %) ===')
for thresh in [0.3, 0.4, 0.5, 0.6, 0.7]:
    for direction in ['bid_heavy', 'ask_heavy']:
        if direction == 'bid_heavy':
            mask = df['full_imbalance'] > thresh
            label = f'imbalance > +{thresh}'
        else:
            mask = df['full_imbalance'] < -thresh
            label = f'imbalance < -{thresh}'
        n = mask.sum()
        if n == 0: continue
        rets = df.loc[mask, 'fwd_ret_1h_bps'] / 100
        big_up = (rets > 1.0).sum()
        big_dn = (rets < -1.0).sum()
        print(f'  {label}: {n} bars, mean 1h={rets.mean():+.3f}%, >+1%:{big_up} ({big_up/n*100:.0f}%), <-1%:{big_dn} ({big_dn/n*100:.0f}%)')

# COMBO: CVD extreme + imbalance confirming
print()
print('=== COMBO: CVD + IMBALANCE confirming (1h fwd, %) ===')
for cvd_t in [-3000, -4000, -5000]:
    for imb_t in [0.2, 0.3, 0.4]:
        mask = (df['cvd_delta'] < cvd_t) & (df['full_imbalance'] > imb_t)
        n = mask.sum()
        if n < 5: continue
        rets = df.loc[mask, 'fwd_ret_1h_bps'] / 100
        print(f'  cvd<{cvd_t} & imb>+{imb_t}: {n} bars, mean 1h={rets.mean():+.3f}%, >+1%:{(rets>1.0).sum()} ({(rets>1.0).sum()/n*100:.0f}%), <-1%:{(rets<-1.0).sum()} ({(rets<-1.0).sum()/n*100:.0f}%)')

# COMBO: CVD extreme + imbalance opposing (absorption)
print()
print('=== COMBO: CVD + IMBALANCE opposing / absorption (1h fwd, %) ===')
for cvd_t in [-3000, -4000, -5000]:
    for imb_t in [-0.2, -0.3, -0.4]:
        mask = (df['cvd_delta'] < cvd_t) & (df['full_imbalance'] < imb_t)
        n = mask.sum()
        if n < 5: continue
        rets = df.loc[mask, 'fwd_ret_1h_bps'] / 100
        print(f'  cvd<{cvd_t} & imb<{imb_t}: {n} bars, mean 1h={rets.mean():+.3f}%, >+1%:{(rets>1.0).sum()} ({(rets>1.0).sum()/n*100:.0f}%), <-1%:{(rets<-1.0).sum()} ({(rets<-1.0).sum()/n*100:.0f}%)')

# CVD momentum (acceleration) - directional thrust
print()
print('=== CVD ACCELERATION extreme vs BIG MOVES (1h fwd, %) ===')
for q in [0.95, 0.99]:
    thresh_up = df['cvd_acceleration'].quantile(q)
    thresh_dn = df['cvd_acceleration'].quantile(1-q)
    mask_up = df['cvd_acceleration'] > thresh_up
    mask_dn = df['cvd_acceleration'] < thresh_dn
    n_up = mask_up.sum()
    n_dn = mask_dn.sum()
    rets_up = df.loc[mask_up, 'fwd_ret_1h_bps'] / 100
    rets_dn = df.loc[mask_dn, 'fwd_ret_1h_bps'] / 100
    print(f'  accel > p{q} ({thresh_up:.0f}): {n_up} bars, mean 1h={rets_up.mean():+.3f}%')
    print(f'  accel < p{1-q} ({thresh_dn:.0f}): {n_dn} bars, mean 1h={rets_dn.mean():+.3f}%')

# Spread widening (volatility clue)
print()
print('=== SPREAD WIDENING vs BIG MOVES (1h fwd, %) ===')
for q in [0.90, 0.95, 0.99]:
    thresh = df['spread_bps'].quantile(q)
    mask = df['spread_bps'] > thresh
    n = mask.sum()
    rets = df.loc[mask, 'fwd_ret_1h_bps'] / 100
    print(f'  spread > p{q} ({thresh:.1f} bps): {n} bars, mean 1h={rets.mean():+.3f}%, std={rets.std():.3f}%')
