#!/bin/bash
echo "=== BTCUSDT bookDelta dates ==="
ls /root/chd/data/ticks/BTCUSDT/bookDelta/ 2>/dev/null
echo "=== BTCUSDT bookDelta 2026-06-09 hours ==="
ls /root/chd/data/ticks/BTCUSDT/bookDelta/2026-06-09/ 2>/dev/null | head -5
echo "=== BTCUSDT trade dates ==="
ls /root/chd/data/ticks/BTCUSDT/trade/ 2>/dev/null
echo "=== BTC hlbook dates ==="
ls /root/chd/data/ticks/BTC/hlbook/ 2>/dev/null
echo "=== BTC hlquote dates ==="
ls /root/chd/data/ticks/BTC/hlquote/ 2>/dev/null
echo "=== ETHUSDT data ==="
ls /root/chd/data/ticks/ETHUSDT/ 2>/dev/null
echo "=== ETH data ==="
ls /root/chd/data/ticks/ETH/ 2>/dev/null
echo "=== All top-level dirs ==="
ls /root/chd/data/ticks/
echo "=== Disk ==="
df -h /root/chd/
echo "=== Sample parquet schema (bookDelta) ==="
python3 -c "
import pyarrow.parquet as pq
t = pq.read_table('/root/chd/data/ticks/BTCUSDT/bookDelta/2026-06-09/00.parquet')
print('Schema:', t.schema)
print('Rows:', t.num_rows)
print('Cols:', t.column_names)
" 2>/dev/null
echo "=== Sample parquet schema (trade) ==="
python3 -c "
import pyarrow.parquet as pq
t = pq.read_table('/root/chd/data/ticks/BTCUSDT/trade/2026-06-09/00.parquet')
print('Schema:', t.schema)
print('Rows:', t.num_rows)
" 2>/dev/null