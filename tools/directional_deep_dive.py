"""Deep dive into CVD delta extreme as a DIRECTIONAL signal.
We want to HOLD THE MOVE, not scalp bps."""
import pandas as pd, numpy as np

df = pd.read_csv('/root/depthscope_out/stitched_vb10.csv')

# Focus on the strongest signal: cvd_delta < -5000
mask = df['cvd_delta'] < -5000
n = mask.sum()
print(f'=== CVD DELTA < -5000: {n} entry bars ===')
print()

# Full return distribution at different horizons
for col, label in [('fwd_ret_15m_bps','15min'), ('fwd_ret_1h_bps','1hr'), ('fwd_ret_4h_bps','4hr')]:
    rets = df.loc[mask, col] / 100  # to pct
    print(f'{label} forward return:')
    print(f'  mean={rets.mean():+.3f}%  median={rets.median():+.3f}%  std={rets.std():.3f}%')
    print(f'  win rate (ret>0): {(rets>0).mean()*100:.1f}%')
    print(f'  >+0.5%: {(rets>0.5).sum()} ({(rets>0.5).mean()*100:.1f}%)')
    print(f'  >+1.0%: {(rets>1.0).sum()} ({(rets>1.0).mean()*100:.1f}%)')
    print(f'  >+2.0%: {(rets>2.0).sum()} ({(rets>2.0).mean()*100:.1f}%)')
    print(f'  <-0.5%: {(rets<-0.5).sum()} ({(rets<-0.5).mean()*100:.1f}%)')
    print(f'  <-1.0%: {(rets<-1.0).sum()} ({(rets<-1.0).mean()*100:.1f}%)')
    print(f'  worst: {rets.min():+.3f}%   best: {rets.max():+.3f}%')
    print()

# Per-date breakdown
print('=== PER-DATE BREAKDOWN (cvd_delta < -5000, 1h fwd) ===')
df['date'] = pd.to_datetime(df['ts'], unit='ns').dt.date
for date, grp in df.loc[mask].groupby('date'):
    rets = grp['fwd_ret_1h_bps'] / 100
    print(f'  {date}: {len(grp)} trades, mean={rets.mean():+.3f}%, win={(rets>0).mean()*100:.0f}%, >+1%:{(rets>1).sum()}, <-1%:{(rets<-1).sum()}')

print()

# What about the SHORT side? CVD delta > +X
print('=== CVD DELTA POSITIVE (sell pressure) - SHORT SIGNAL ===')
for thresh in [2000, 3000, 4000, 5000]:
    mask_s = df['cvd_delta'] > thresh
    n_s = mask_s.sum()
    if n_s < 10: continue
    rets = df.loc[mask_s, 'fwd_ret_1h_bps'] / 100
    print(f'  cvd_delta > +{thresh}: {n_s} bars, mean 1h={rets.mean():+.3f}%, <-1%:{(rets<-1).sum()} ({(rets<-1).mean()*100:.0f}%), >+1%:{(rets>1).sum()} ({(rets>1).mean()*100:.0f}%)')

print()

# The KEY question: can we separate the 49% big-up from the 8% big-down?
# What's different about the losers?
print('=== WHAT SEPARATES WINNERS FROM LOSERS? (cvd_delta < -5000, 1h fwd) ===')
mask = df['cvd_delta'] < -5000
subset = df.loc[mask].copy()
subset['ret_1h_pct'] = subset['fwd_ret_1h_bps'] / 100
winners = subset[subset['ret_1h_pct'] > 0.5]
big_winners = subset[subset['ret_1h_pct'] > 1.0]
losers = subset[subset['ret_1h_pct'] < -0.5]
big_losers = subset[subset['ret_1h_pct'] < -1.0]

for name, group in [('ALL', subset), ('WIN >0.5%', winners), ('BIG WIN >1%', big_winners), ('LOSE <-0.5%', losers), ('BIG LOSE <-1%', big_losers)]:
    if len(group) == 0: continue
    print(f'  {name} (n={len(group)}):')
    print(f'    cvd_delta: mean={group["cvd_delta"].mean():.0f}  spread={group["spread_bps"].mean():.2f}  imbalance={group["full_imbalance"].mean():.3f}')
    print(f'    cvd_ratio: mean={group["cvd_ratio"].mean():.3f}  cvd_momentum={group["cvd_momentum"].mean():.0f}  cvd_accel={group["cvd_acceleration"].mean():.0f}')
    print(f'    total_bid_vol={group["total_bid_vol"].mean():.1f}  total_ask_vol={group["total_ask_vol"].mean():.1f}')
    print(f'    ask_concentration={group["ask_concentration"].mean():.4f}  bid_concentration={group["bid_concentration"].mean():.4f}')
    print(f'    active_wall_count={group["active_wall_count"].mean():.1f}  wall_cancel_ratio={group["wall_cancel_ratio"].mean():.3f}')
    print()

# Simple strategy simulation: long when cvd_delta < -5000, hold 1hr
print('=== SIMPLE STRATEGY: Long on cvd_delta < -5000, hold 1hr ===')
mask = df['cvd_delta'] < -5000
rets = df.loc[mask, 'fwd_ret_1h_bps'] / 100
fee_pct = 0.09  # taker fee in %
net_rets = rets - fee_pct
total_trades = len(rets)
gross_pnl = rets.sum()
net_pnl = net_rets.sum()
win_rate = (net_rets > 0).mean()
avg_win = net_rets[net_rets > 0].mean() if (net_rets > 0).any() else 0
avg_loss = net_rets[net_rets <= 0].mean() if (net_rets <= 0).any() else 0
rr = abs(avg_win / avg_loss) if avg_loss != 0 else float('inf')
print(f'  Trades: {total_trades}')
print(f'  Gross P&L: {gross_pnl:+.2f}%')
print(f'  Net P&L (taker 0.09%): {net_pnl:+.2f}%')
print(f'  Net P&L per trade: {net_pnl/total_trades:+.4f}%')
print(f'  Win rate: {win_rate*100:.1f}%')
print(f'  Avg win: {avg_win:+.3f}%  Avg loss: {avg_loss:+.3f}%  R:R = {rr:.2f}:1')
print(f'  Expectancy per trade: {net_rets.mean()*100:+.3f}% = {net_rets.mean()*10000:+.1f} bps')

# With maker fee
fee_maker = 0.05
net_rets_maker = rets - fee_maker
print(f'  --- Maker (0.05%): ---')
print(f'  Net P&L: {net_rets_maker.sum():+.2f}%')
print(f'  Net per trade: {net_rets_maker.mean()*100:+.4f}% = {net_rets_maker.mean()*10000:+.1f} bps')
print(f'  Win rate: {(net_rets_maker > 0).mean()*100:.1f}%')
