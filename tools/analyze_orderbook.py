#!/usr/bin/env python3
"""Analyze CHD orderbook data - check depth per event_time."""
import sys, io, zstandard, pandas as pd
import requests

KEY = "bf90cc0213eb0d5d949343df0afef3a5741c2a758e91b0b0268a754223a32d86"
BASE = "https://api.cryptohftdata.com"

def dl(path):
    url = f"{BASE}/download?file={path}&api_key={KEY}"
    r = requests.get(url, timeout=120)
    if r.status_code == 404:
        return None
    r.raise_for_status()
    raw = r.content
    try:
        raw = zstandard.ZstdDecompressor().decompress(raw)
    except Exception:
        pass
    return pd.read_parquet(io.BytesIO(raw))

print("=== Binance Futures BTCUSDT orderbook ===")
df = dl("binance_futures/2026-06-09/09/BTCUSDT_orderbook.parquet.zst")
if df is not None:
    df['quantity'] = pd.to_numeric(df['quantity'], errors='coerce')
    df['price'] = pd.to_numeric(df['price'], errors='coerce')
    
    print(f"Total rows: {len(df)}")
    print(f"Columns: {list(df.columns)}")
    print(f"event_type values: {df['event_type'].unique()}")
    
    sample_times = sorted(df['event_time'].unique())[:5]
    for t in sample_times:
        sub = df[df['event_time'] == t]
        bids = sub[(sub['side'] == 'bid') & (sub['quantity'] > 0)]
        asks = sub[(sub['side'] == 'ask') & (sub['quantity'] > 0)]
        removes_bid = sub[(sub['side'] == 'bid') & (sub['quantity'] == 0)]
        removes_ask = sub[(sub['side'] == 'ask') & (sub['quantity'] == 0)]
        print(f"  ts={t}: {len(bids)} bid updates, {len(asks)} ask updates, {len(removes_bid)} bid removes, {len(removes_ask)} ask removes")
    
    print(f"\nfirst_update_id range: {df['first_update_id'].min()} - {df['first_update_id'].max()}")
    print(f"final_update_id range: {df['final_update_id'].min()} - {df['final_update_id'].max()}")
    print(f"\nUnique event_times: {df['event_time'].nunique()}")
    print(f"Time span: {(df['event_time'].max() - df['event_time'].min()) / 1000:.1f} seconds")
    
    # Book reconstruction test
    print("\n=== Book reconstruction test ===")
    book = {"bid": {}, "ask": {}}
    events = df.sort_values(['event_time', 'received_time']).reset_index(drop=True)
    applied = 0
    for i, row in events.head(50000).iterrows():
        side = row['side']
        price = row['price']
        qty = row['quantity']
        if qty == 0:
            book[side].pop(price, None)
        else:
            book[side][price] = qty
        applied += 1
    
    top_bids = sorted(book["bid"].items(), key=lambda x: -x[0])[:10]
    top_asks = sorted(book["ask"].items(), key=lambda x: x[0])[:10]
    print(f"After {applied} deltas:")
    print(f"  Bid levels: {len(book['bid'])}, Ask levels: {len(book['ask'])}")
    print(f"  Top 5 bids: {top_bids[:5]}")
    print(f"  Top 5 asks: {top_asks[:5]}")
    if top_bids and top_asks:
        mid = (top_bids[0][0] + top_asks[0][0]) / 2
        spread = top_asks[0][0] - top_bids[0][0]
        print(f"  Mid: {mid:.2f}, Spread: {spread:.2f} ({spread/mid*10000:.1f} bps)")