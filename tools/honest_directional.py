"""Honest non-overlapping directional analysis.
Entry: cvd_delta < threshold → go long
Hold: fixed time (15min, 1hr, 4hr) then exit at market
No re-entry until after exit bar."""
import pandas as pd, numpy as np

df = pd.read_csv('/root/depthscope_out/stitched_vb10.csv')
n_bars = len(df)

# How many bars is each hold period?
# Volume bars are ~10 BTC each. At normal volume that's roughly 1-5 minutes per bar.
# Let's estimate: 40017 bars over 18 dates ≈ 2223 bars/day ≈ over 24h = ~1 bar per 40 seconds
# So: 15min ≈ 22 bars, 1hr ≈ 90 bars, 4hr ≈ 360 bars

for hold_label, hold_bars in [('15min (~22 bars)', 22), ('1hr (~90 bars)', 90), ('4hr (~360 bars)', 360)]:
    print(f'\n=== HOLD {hold_label} ===')
    for thresh in [-3000, -4000, -5000, -6000]:
        trades = []
        i = 0
        while i < n_bars:
            if df.iloc[i]['cvd_delta'] < thresh:
                entry_price = df.iloc[i]['mid_price']
                entry_idx = i
                exit_idx = min(i + hold_bars, n_bars - 1)
                exit_price = df.iloc[exit_idx]['mid_price']
                gross_pct = (exit_price - entry_price) / entry_price * 100
                trades.append({
                    'entry_idx': entry_idx,
                    'exit_idx': exit_idx,
                    'entry_price': entry_price,
                    'exit_price': exit_price,
                    'gross_pct': gross_pct,
                    'net_taker_pct': gross_pct - 0.09,  # round-trip taker
                    'net_maker_pct': gross_pct - 0.05,  # round-trip maker
                })
                i = exit_idx + 1  # skip to after exit (no overlap)
            else:
                i += 1

        if not trades:
            print(f'  cvd_delta < {thresh}: no trades')
            continue

        tdf = pd.DataFrame(trades)
        n = len(tdf)
        gross_avg = tdf['gross_pct'].mean()
        net_taker_avg = tdf['net_taker_pct'].mean()
        net_maker_avg = tdf['net_maker_pct'].mean()
        wr_taker = (tdf['net_taker_pct'] > 0).mean()
        wr_maker = (tdf['net_maker_pct'] > 0).mean()
        avg_win = tdf.loc[tdf['gross_pct'] > 0, 'gross_pct'].mean()
        avg_loss = tdf.loc[tdf['gross_pct'] <= 0, 'gross_pct'].mean()
        rr = abs(avg_win / avg_loss) if avg_loss != 0 else float('inf')
        total_gross = tdf['gross_pct'].sum()
        total_net_taker = tdf['net_taker_pct'].sum()
        total_net_maker = tdf['net_maker_pct'].sum()

        # Big move stats
        big_up = (tdf['gross_pct'] > 1.0).sum()
        big_dn = (tdf['gross_pct'] < -1.0).sum()

        print(f'  cvd_delta < {thresh}: {n} trades (non-overlapping)')
        print(f'    avg gross: {gross_avg:+.3f}%  avg net (taker): {net_taker_avg:+.3f}%  avg net (maker): {net_maker_avg:+.3f}%')
        print(f'    win rate (taker): {wr_taker*100:.1f}%  win rate (maker): {wr_maker*100:.1f}%')
        print(f'    avg win: {avg_win:+.3f}%  avg loss: {avg_loss:+.3f}%  R:R = {rr:.2f}:1')
        print(f'    total gross: {total_gross:+.1f}%  total net (taker): {total_net_taker:+.1f}%  total net (maker): {total_net_maker:+.1f}%')
        print(f'    >+1% moves: {big_up} ({big_up/n*100:.0f}%)  <-1% moves: {big_dn} ({big_dn/n*100:.0f}%)')

# Now the SHORT side
print('\n\n=== SHORT SIDE: cvd_delta > threshold, hold 1hr ===')
hold_bars = 90
for thresh in [3000, 4000, 5000, 6000]:
    trades = []
    i = 0
    while i < n_bars:
        if df.iloc[i]['cvd_delta'] > thresh:
            entry_price = df.iloc[i]['mid_price']
            entry_idx = i
            exit_idx = min(i + hold_bars, n_bars - 1)
            exit_price = df.iloc[exit_idx]['mid_price']
            gross_pct = (entry_price - exit_price) / entry_price * 100  # short P&L
            trades.append({
                'entry_idx': entry_idx,
                'exit_idx': exit_idx,
                'gross_pct': gross_pct,
                'net_taker_pct': gross_pct - 0.09,
                'net_maker_pct': gross_pct - 0.05,
            })
            i = exit_idx + 1
        else:
            i += 1

    if not trades:
        print(f'  cvd_delta > +{thresh}: no trades')
        continue

    tdf = pd.DataFrame(trades)
    n = len(tdf)
    gross_avg = tdf['gross_pct'].mean()
    net_taker_avg = tdf['net_taker_pct'].mean()
    wr_taker = (tdf['net_taker_pct'] > 0).mean()
    avg_win = tdf.loc[tdf['gross_pct'] > 0, 'gross_pct'].mean()
    avg_loss = tdf.loc[tdf['gross_pct'] <= 0, 'gross_pct'].mean()
    rr = abs(avg_win / avg_loss) if avg_loss != 0 else float('inf')

    print(f'  cvd_delta > +{thresh}: {n} trades')
    print(f'    avg gross: {gross_avg:+.3f}%  avg net (taker): {net_taker_avg:+.3f}%')
    print(f'    win rate (taker): {wr_taker*100:.1f}%  R:R = {rr:.2f}:1')

# BOTH DIRECTIONS combined
print('\n\n=== BOTH DIRECTIONS: cvd_delta extreme, hold 1hr ===')
for long_t, short_t in [(-3000, 3000), (-4000, 4000), (-5000, 5000), (-6000, 6000)]:
    trades = []
    i = 0
    while i < n_bars:
        if df.iloc[i]['cvd_delta'] < long_t:
            entry_price = df.iloc[i]['mid_price']
            exit_idx = min(i + hold_bars, n_bars - 1)
            exit_price = df.iloc[exit_idx]['mid_price']
            gross_pct = (exit_price - entry_price) / entry_price * 100
            trades.append({'gross_pct': gross_pct, 'net_taker': gross_pct - 0.09, 'dir': 'long'})
            i = exit_idx + 1
        elif df.iloc[i]['cvd_delta'] > short_t:
            entry_price = df.iloc[i]['mid_price']
            exit_idx = min(i + hold_bars, n_bars - 1)
            exit_price = df.iloc[exit_idx]['mid_price']
            gross_pct = (entry_price - exit_price) / entry_price * 100
            trades.append({'gross_pct': gross_pct, 'net_taker': gross_pct - 0.09, 'dir': 'short'})
            i = exit_idx + 1
        else:
            i += 1

    if not trades:
        continue
    tdf = pd.DataFrame(trades)
    n = len(tdf)
    n_long = (tdf['dir'] == 'long').sum()
    n_short = (tdf['dir'] == 'short').sum()
    gross_avg = tdf['gross_pct'].mean()
    net_taker_avg = tdf['net_taker'].mean()
    wr = (tdf['net_taker'] > 0).mean()
    avg_win = tdf.loc[tdf['gross_pct'] > 0, 'gross_pct'].mean()
    avg_loss = tdf.loc[tdf['gross_pct'] <= 0, 'gross_pct'].mean()
    rr = abs(avg_win / avg_loss) if avg_loss != 0 else float('inf')

    print(f'  long<{long_t} / short>{short_t}: {n} trades ({n_long}L/{n_short}S)')
    print(f'    avg gross: {gross_avg:+.3f}%  avg net (taker): {net_taker_avg:+.3f}%')
    print(f'    win rate: {wr*100:.1f}%  R:R = {rr:.2f}:1')
    print(f'    total net (taker): {tdf["net_taker"].sum():+.1f}%')
