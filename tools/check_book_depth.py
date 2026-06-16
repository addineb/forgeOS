#!/usr/bin/env python3
"""Check book depth: how many levels does the initial snapshot give us?"""
import pyarrow.parquet as pq
import os

bd_dir = '/root/chd/data/ticks/BTCUSDT/bookDelta'

# Check the initial snapshot depth for each hour of 2026-06-09
print("=== Initial snapshot depth per hour (2026-06-09) ===")
for hh in range(24):
    path = os.path.join(bd_dir, '2026-06-09', f'{hh:02d}.parquet')
    if not os.path.exists(path):
        print(f"  {hh:02d}: MISSING")
        continue
    t = pq.read_table(path)
    first_ts = t.column('venueTs')[0].as_py()
    # Count rows at first timestamp
    snap_rows = 0
    bid_levels = 0
    ask_levels = 0
    for i in range(min(5000, t.num_rows)):
        if t.column('venueTs')[i].as_py() != first_ts:
            break
        snap_rows += 1
        side = t.column('side')[i].as_py()
        if side == 'bid':
            bid_levels += 1
        elif side == 'ask':
            ask_levels += 1
    
    # Also check: how many total unique timestamps in first 10 seconds?
    ts_10s = first_ts + 10000  # 10 seconds later
    ts_count = 0
    for i in range(t.num_rows):
        if t.column('venueTs')[i].as_py() > ts_10s:
            break
        ts_count += 1
    
    print(f"  {hh:02d}: first_ts={first_ts}, snap_rows={snap_rows}, bid={bid_levels}, ask={ask_levels}, updates_in_10s={ts_count}")

# Check total unique price levels after 5 minutes of replay
print("\n=== Book depth after 5 minutes of replay (hour 00) ===")
t = pq.read_table(os.path.join(bd_dir, '2026-06-09', '00.parquet'))
first_ts = t.column('venueTs')[0].as_py()
five_min_later = first_ts + 300000  # 5 minutes in ms

# Build book state
bids = {}
asks = {}
updates = 0
for i in range(t.num_rows):
    ts = t.column('venueTs')[i].as_py()
    if ts > five_min_later:
        break
    side = t.column('side')[i].as_py()
    price = t.column('price')[i].as_py()
    qty = t.column('qty')[i].as_py()
    kind = t.column('kind')[i].as_py()
    
    book = bids if side == 'bid' else asks
    if kind == 'remove' or qty == 0.0:
        book.pop(price, None)
    else:
        book[price] = qty
    updates += 1

print(f"  After {updates} updates in 5 minutes:")
print(f"  Bid levels: {len(bids)}, Ask levels: {len(asks)}")
print(f"  Best bid: {max(bids.keys()):.2f} ({bids[max(bids.keys())]:.6f} BTC)")
print(f"  Best ask: {min(asks.keys()):.2f} ({asks[min(asks.keys())]:.6f} BTC)")
print(f"  Spread: {(min(asks.keys()) - max(bids.keys())):.2f} USD = {(min(asks.keys()) - max(bids.keys())) / max(bids.keys()) * 10000:.1f} bps")
print(f"  Bid depth (top 5): {sorted(bids.keys(), reverse=True)[:5]}")
print(f"  Ask depth (top 5): {sorted(asks.keys())[:5]}")

# Check if we have ETH data anywhere
print("\n=== Checking for ETH data ===")
for sym in ['ETHUSDT', 'ETH', 'ethusdt', 'eth']:
    for stream in ['bookDelta', 'trade', 'hlbook', 'hlquote']:
        p = os.path.join('/root/chd/data/ticks', sym, stream)
        if os.path.exists(p):
            dates = os.listdir(p)
            print(f"  {sym}/{stream}: {len(dates)} dates")