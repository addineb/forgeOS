import pyarrow.parquet as pq
t = pq.read_table('/root/chd/data/ticks/BTCUSDT/bookDelta/2026-06-10/00.parquet')
print('BTC book cols:', t.column_names)
t2 = pq.read_table('/root/chd/data/ticks/ETHUSDT/bookDelta/2026-06-10/00.parquet')
print('ETH book cols:', t2.column_names)
