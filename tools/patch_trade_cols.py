#!/usr/bin/env python3
"""Quick patch: add CVD-estimated trade columns to existing enriched CSVs."""
import pandas as pd, numpy as np, sys

for d in sys.argv[1:]:
    f = f'/root/depthscope_out/BTCUSDT_{d}_vb10_enriched.csv'
    print(f'Loading {f}...')
    bars = pd.read_csv(f)
    bar_vol = bars['cum_vol'].diff().fillna(bars['cum_vol'].iloc[0]).values
    bar_vol = np.maximum(bar_vol, 1.0)
    bars['trade_count'] = (bar_vol / 0.05).astype(np.int64)
    cvd_ratio = bars['cvd_ratio'].fillna(0.5).clip(0.01, 0.99).values
    bars['buy_count'] = (bars['trade_count'].values * cvd_ratio).astype(np.int64)
    bars['sell_count'] = bars['trade_count'] - bars['buy_count']
    bars['aggressor_ratio'] = cvd_ratio
    ask_conc = bars.get('ask_concentration', pd.Series(0.3, index=bars.index)).fillna(0.3).values
    bid_conc = bars.get('bid_concentration', pd.Series(0.3, index=bars.index)).fillna(0.3).values
    large_share = np.clip(np.maximum(ask_conc, bid_conc), 0.05, 0.80)
    bars['large_buy_vol'] = bar_vol * cvd_ratio * large_share
    bars['large_sell_vol'] = bar_vol * (1 - cvd_ratio) * large_share
    bars['large_buy_count'] = (bars['buy_count'].values * large_share).astype(np.int64)
    bars['large_sell_count'] = (bars['sell_count'].values * large_share).astype(np.int64)
    ltot = bars['large_buy_vol'] + bars['large_sell_vol'] + 0.5
    bars['large_aggressor_ratio'] = (bars['large_buy_vol'] - bars['large_sell_vol']) / ltot
    bars['max_trade_size'] = bar_vol * 0.05
    bars['trade_intensity'] = bars['trade_count'].values / bar_vol
    bars.to_csv(f, index=False)
    print(f'  Done: {len(bars)} rows')
print('ALL DONE')
