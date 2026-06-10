#!/usr/bin/env python3
# chd-to-parquet.py - cryptohftdata -> our captured Parquet tick layout.
# Stages: trade (flat) | bookDelta (flat, Binance diff passthrough) |
#         hlquote (flat, reconstructed BBO from Hyperliquid orderbook).
# Schemas byte-match lib/tick-storage.ts so the replay harness reads them as-is.
import os, sys, io, argparse, requests
import pyarrow as pa, pyarrow.parquet as pq
import pandas as pd, zstandard as zstd

BASE = "https://api.cryptohftdata.com"
KEY = os.environ["CRYPTOHFTDATA_API_KEY"]

TRADE_SCHEMA = pa.schema([
    ("ts", pa.int64()), ("localTs", pa.int64()), ("symbol", pa.string()),
    ("price", pa.float64()), ("qty", pa.float64()), ("isBuyerMaker", pa.bool_()),
])
BOOKDELTA_SCHEMA = pa.schema([
    ("venueTs", pa.int64()), ("captureTs", pa.int64()), ("side", pa.string()),
    ("price", pa.float64()), ("qty", pa.float64()), ("kind", pa.string()),
])
HLQUOTE_SCHEMA = pa.schema([
    ("ts", pa.int64()), ("coin", pa.string()), ("bid", pa.float64()),
    ("ask", pa.float64()), ("mid", pa.float64()), ("source", pa.string()),
])

def dl(file_path):
    url = "%s/download?file=%s&api_key=%s" % (BASE, file_path, KEY)
    r = requests.get(url, timeout=180)
    if r.status_code == 404:
        return None
    r.raise_for_status()
    raw = r.content
    try:
        raw = zstd.ZstdDecompressor().decompress(raw)
    except Exception:
        pass
    return pd.read_parquet(io.BytesIO(raw))

def outdir(data_dir, key, stream, date):
    d = os.path.join(data_dir, "ticks", key, stream, date)
    os.makedirs(d, exist_ok=True)
    return d

def conv_trades(exchange, symbol, date, hh, data_dir):
    df = dl("%s/%s/%s/%s_trades.parquet.zst" % (exchange, date, hh, symbol))
    if df is None or len(df) == 0: return 0
    tbl = pa.table({
        "ts": df["trade_time"].astype("int64").to_numpy(),
        "localTs": (df["received_time"].astype("int64") // 1_000_000).to_numpy(),
        "symbol": pa.array([symbol]*len(df), pa.string()),
        "price": df["price"].astype("float64").to_numpy(),
        "qty": df["quantity"].astype("float64").to_numpy(),
        "isBuyerMaker": df["is_buyer_maker"].astype("bool").to_numpy(),
    }, schema=TRADE_SCHEMA)
    pq.write_table(tbl, os.path.join(outdir(data_dir, symbol, "trade", date), hh+".parquet"))
    return len(df)

def conv_bookdelta(exchange, symbol, date, hh, data_dir):
    df = dl("%s/%s/%s/%s_orderbook.parquet.zst" % (exchange, date, hh, symbol))
    if df is None or len(df) == 0: return 0
    qty = df["quantity"].astype("float64")
    side = df["side"].astype(str)
    kind = pd.Series(["remove" if q == 0 else "change" for q in qty])
    tbl = pa.table({
        "venueTs": df["event_time"].astype("int64").to_numpy(),
        "captureTs": (df["received_time"].astype("int64") // 1_000_000).to_numpy(),
        "side": pa.array(side.tolist(), pa.string()),
        "price": df["price"].astype("float64").to_numpy(),
        "qty": qty.to_numpy(),
        "kind": pa.array(kind.tolist(), pa.string()),
    }, schema=BOOKDELTA_SCHEMA)
    pq.write_table(tbl, os.path.join(outdir(data_dir, symbol, "bookDelta", date), hh+".parquet"))
    return len(df)

def conv_hlquote(exchange, symbol, coin, date, hh, data_dir, book):
    # book = persistent {"bid":{px:qty}, "ask":{px:qty}} carried across hours.
    df = dl("%s/%s/%s/%s_orderbook.parquet.zst" % (exchange, date, hh, symbol))
    if df is None or len(df) == 0: return 0, book
    ets = df["event_time"].astype("int64").tolist()
    rts = (df["received_time"].astype("int64") // 1_000_000).tolist()
    ev  = df["event_type"].astype(str).tolist()
    sd  = df["side"].astype(str).tolist()
    px  = df["price"].astype("float64").tolist()
    qt  = df["quantity"].astype("float64").tolist()
    rows_ts=[]; rows_bid=[]; rows_ask=[]; rows_mid=[]
    in_snap=False; cur_et=None
    def bbo():
        b = max(book["bid"]) if book["bid"] else None
        a = min(book["ask"]) if book["ask"] else None
        return b, a
    last_emit=None
    for i in range(len(ets)):
        if ev[i] == "snapshot":
            if not in_snap:
                book["bid"].clear(); book["ask"].clear(); in_snap=True
        else:
            in_snap=False
        s = "bid" if sd[i] == "bid" else "ask"
        if qt[i] == 0:
            book[s].pop(px[i], None)
        else:
            book[s][px[i]] = qt[i]
        # emit on event_time boundary
        if cur_et is not None and ets[i] != cur_et:
            b,a = bbo()
            if b and a:
                m=(b+a)/2.0
                if last_emit != (b,a):
                    rows_ts.append(cur_et); rows_bid.append(b); rows_ask.append(a); rows_mid.append(m)
                    last_emit=(b,a)
        cur_et = ets[i]
    if not rows_ts: return 0, book
    tbl = pa.table({
        "ts": pa.array(rows_ts, pa.int64()),
        "coin": pa.array([coin]*len(rows_ts), pa.string()),
        "bid": pa.array(rows_bid, pa.float64()),
        "ask": pa.array(rows_ask, pa.float64()),
        "mid": pa.array(rows_mid, pa.float64()),
        "source": pa.array(["chd"]*len(rows_ts), pa.string()),
    }, schema=HLQUOTE_SCHEMA)
    pq.write_table(tbl, os.path.join(outdir(data_dir, coin, "hlquote", date), hh+".parquet"))
    return len(rows_ts), book

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--date", required=True)
    ap.add_argument("--symbol", default="BTCUSDT")
    ap.add_argument("--exchange", default="binance_spot")
    ap.add_argument("--hl-exchange", default="hyperliquid_futures")
    ap.add_argument("--hl-symbol", default="BTC")
    ap.add_argument("--coin", default="BTC")
    ap.add_argument("--data-dir", default="./data")
    ap.add_argument("--streams", default="trade,bookDelta,hlquote")
    ap.add_argument("--hours", default="all")
    a = ap.parse_args()
    streams = a.streams.split(",")
    hours = [f"{h:02d}" for h in range(24)] if a.hours == "all" else a.hours.split(",")
    tot = {"trade":0,"bookDelta":0,"hlquote":0}
    hlbook = {"bid":{}, "ask":{}}
    for hh in hours:
        if "trade" in streams:
            n = conv_trades(a.exchange, a.symbol, a.date, hh, a.data_dir); tot["trade"]+=n
        if "bookDelta" in streams:
            n = conv_bookdelta(a.exchange, a.symbol, a.date, hh, a.data_dir); tot["bookDelta"]+=n
        if "hlquote" in streams:
            n, hlbook = conv_hlquote(a.hl_exchange, a.hl_symbol, a.coin, a.date, hh, a.data_dir, hlbook); tot["hlquote"]+=n
        print(f"[chd] {a.date} {hh} trade+bookDelta+hlquote done", flush=True)
    print(f"[chd] TOTAL {tot}")

if __name__ == "__main__":
    main()