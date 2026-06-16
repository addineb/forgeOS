#!/usr/bin/env python3
"""Verify book reconstruction quality: check warm-up period and book sanity."""
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

print("=== Book reconstruction verification ===")
df = dl("binance_futures/2026-06-09/09/BTCUSDT_orderbook.parquet.zst")
if df is None:
    print("Failed to download data")
    sys.exit(1)

df['quantity'] = pd.to_numeric(df['quantity'], errors='coerce')
df['price'] = pd.to_numeric(df['price'], errors='coerce')

# Sort by event_time then received_time
events = df.sort_values(['event_time', 'received_time']).reset_index(drop=True)
print(f"Total events: {len(events)}")

# Reconstruct book step by step, track quality metrics
book = {"bid": {}, "ask": {}}
crossed_count = 0  # times best_bid >= best_ask (should be 0 after warm-up)
best_bid_history = []
best_ask_history = []
spread_history = []
bid_level_history = []
ask_level_history = []

# Check at intervals
check_points = [100, 500, 1000, 5000, 10000, 50000, 100000, 500000, 1000000, len(events)]

for i, row in events.iterrows():
    side = row['side']
    price = row['price']
    qty = row['quantity']
    if qty == 0:
        book[side].pop(price, None)
    else:
        book[side][price] = qty
    
    # Check book sanity at checkpoints
    if i + 1 in check_points:
        bids = sorted(book["bid"].keys(), reverse=True)
        asks = sorted(book["ask"].keys())
        best_bid = bids[0] if bids else None
        best_ask = asks[0] if asks else None
        
        if best_bid and best_ask:
            spread = best_ask - best_bid
            crossed = best_bid >= best_ask
            if crossed:
                crossed_count += 1
            
            best_bid_history.append(best_bid)
            best_ask_history.append(best_ask)
            spread_history.append(spread)
            bid_level_history.append(len(bids))
            ask_level_history.append(len(asks))
            
            print(f"  After {i+1:>10,} deltas: best_bid={best_bid:.1f}, best_ask={best_ask:.1f}, "
                  f"spread={spread:.2f} ({spread/best_ask*10000:.1f} bps), "
                  f"bid_levels={len(bids)}, ask_levels={len(asks)}, "
                  f"CROSSED={crossed}")

# Also check: do trades prices match the book?
print("\n=== Cross-check with trades ===")
trades = dl("binance_futures/2026-06-09/09/BTCUSDT_trades.parquet.zst")
if trades is not None:
    trades['price'] = pd.to_numeric(trades['price'], errors='coerce')
    trades['quantity'] = pd.to_numeric(trades['quantity'], errors='coerce')
    print(f"Trade count: {len(trades)}")
    print(f"Trade price range: {trades['price'].min():.1f} - {trades['price'].max():.1f}")
    print(f"First 5 trade prices: {trades['price'].head(5).tolist()}")
    print(f"Last 5 trade prices: {trades['price'].tail(5).tolist()}")
    
    # Compare: after full reconstruction, does the book's mid match trade prices?
    if best_bid_history:
        print(f"\nBook mid after warm-up: {(best_bid_history[-1] + best_ask_history[-1]) / 2:.1f}")
        print(f"Trade price range in same hour: {trades['price'].min():.1f} - {trades['price'].max():.1f}")
        
        # Check if book mid is within trade range
        mid = (best_bid_history[-1] + best_ask_history[-1]) / 2
        if trades['price'].min() <= mid <= trades['price'].max():
            print("✓ Book mid is WITHIN trade price range (good)")
        else:
            print("✗ Book mid is OUTSIDE trade price range (BAD - data issue!)")

# Check for update_id gaps (missing data)
print("\n=== Update ID gap check ===")
if 'final_update_id' in events.columns:
    # Sort by first_update_id to check for gaps
    unique_updates = events.drop_duplicates(subset=['first_update_id']).sort_values('first_update_id')
    ids = unique_updates['first_update_id'].values
    gaps = 0
    big_gaps = 0
    for j in range(1, min(len(ids), 100000)):
        diff = ids[j] - ids[j-1]
        if diff > 1:
            gaps += 1
            if diff > 100:
                big_gaps += 1
    print(f"Checked {min(len(ids), 100000)} consecutive update IDs")
    print(f"  Gaps (id diff > 1): {gaps}")
    print(f"  Big gaps (id diff > 100): {big_gaps}")
    if big_gaps > 0:
        print("  ⚠ Big gaps in update IDs suggest missing data between hours")
    else:
        print("  ✓ No big gaps in update IDs within this hour")