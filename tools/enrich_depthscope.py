#!/usr/bin/env python3
"""enrich_depthscope.py — add funding, OI, liquidation, and basis features
to per-date depthscope volume-bar CSVs.

Usage:
  python3 enrich_depthscope.py --date 2026-02-01 [-- indir] [--outdir]

Reads  BTCUSDT_{date}_vb10.csv from indir (default /root/depthscope_out).
Writes BTCUSDT_{date}_vb10_enriched.csv to outdir (default = indir).

New columns:
  funding_rate       — HL hourly funding rate, forward-filled to bar ts
  mark_index_bps     — (mark - index) / index * 1e4, per-bar
  oi_pct             — open interest in coin units, forward-filled
  oi_pct_change      — bar-to-bar % change in oi_pct
  liq_vol_buy        — total buy-side liquidation volume in bar window
  liq_vol_sell       — total sell-side liquidation volume in bar window
  liq_imbalance      — (liq_buy - liq_sell) / (liq_buy + liq_sell + 1e-10)
  basis_bps          — (HL_best_ask - Binance_best_bid) / mid * 1e4
"""
import os, sys, io, argparse, warnings
import numpy as np, pandas as pd
try: import zstandard as zstd
except: zstd=None
import requests

BASE = "https://api.cryptohftdata.com"
KEY_FILE = "/root/.chd_key"

def load_key():
    return open(KEY_FILE).read().strip().split("=")[-1].strip().strip('"').strip("'")

def dl(path, key):
    u = "%s/download?file=%s&api_key=%s" % (BASE, path, key)
    r = requests.get(u, timeout=300)
    if r.status_code == 404: return None
    if r.status_code != 200: return None
    raw = r.content
    if zstd:
        try: raw = zstd.ZstdDecompressor().decompress(raw)
        except: pass
    try: return pd.read_parquet(io.BytesIO(raw))
    except: return None

def pull_funding(date, key):
    rows = []
    for hh in range(24):
        p = "hyperliquid_futures/%s/%02d/BTC_mark_price.parquet.zst" % (date, hh)
        df = dl(p, key)
        if df is None or len(df) == 0: continue
        ts = df["event_time"].astype("int64")
        fr = pd.to_numeric(df["funding_rate"], errors="coerce").fillna(0.0)
        mk = pd.to_numeric(df["mark_price"], errors="coerce").fillna(0.0)
        ix = pd.to_numeric(df["index_price"], errors="coerce").fillna(0.0)
        chunk = pd.DataFrame({"ts_ns": ts.values, "funding_rate": fr.values,
                              "mark": mk.values, "index": ix.values})
        chunk = chunk.drop_duplicates(subset="ts_ns", keep="last").sort_values("ts_ns")
        rows.append(chunk)
    if not rows: return None
    out = pd.concat(rows, ignore_index=True)
    out = out.drop_duplicates(subset="ts_ns", keep="last").sort_values("ts_ns").reset_index(drop=True)
    return out

def pull_oi(date, key):
    rows = []
    for hh in range(24):
        p = "hyperliquid_futures/%s/%02d/BTC_open_interest.parquet.zst" % (date, hh)
        df = dl(p, key)
        if df is None or len(df) == 0: continue
        ts = df["timestamp"].astype("int64")
        oi = pd.to_numeric(df["sum_open_interest"], errors="coerce").fillna(0.0)
        chunk = pd.DataFrame({"ts_ns": ts.values, "oi": oi.values})
        chunk = chunk.drop_duplicates(subset="ts_ns", keep="last").sort_values("ts_ns")
        rows.append(chunk)
    if not rows: return None
    out = pd.concat(rows, ignore_index=True)
    out = out.drop_duplicates(subset="ts_ns", keep="last").sort_values("ts_ns").reset_index(drop=True)
    return out

def pull_liquidations(date, key):
    rows = []
    for hh in range(24):
        p = "binance_futures/%s/%02d/BTCUSDT_liquidations.parquet.zst" % (date, hh)
        df = dl(p, key)
        if df is None or len(df) == 0: continue
        ts = df["event_time"].astype("int64") * 1_000_000  # ms -> ns
        side = df["side"].astype(str)
        qty = pd.to_numeric(df["quantity"], errors="coerce").fillna(0.0)
        chunk = pd.DataFrame({"ts_ns": ts.values, "side": side.values, "qty": qty.values})
        rows.append(chunk)
    if not rows: return None
    out = pd.concat(rows, ignore_index=True).sort_values("ts_ns").reset_index(drop=True)
    return out

def pull_binance_bbo(date, key):
    """Reconstruct Binance L2 BBO from diff feed for basis."""
    book = {"bid": {}, "ask": {}}
    rows_ts = []; rows_bid = []; rows_ask = []
    for hh in range(24):
        p = "binance_spot/%s/%02d/BTCUSDT_orderbook.parquet.zst" % (date, hh)
        df = dl(p, key)
        if df is None or len(df) == 0: continue
        ets = df["event_time"].astype("int64").tolist()       # ms
        sd  = df["side"].astype(str).tolist()
        px  = df["price"].astype("float64").tolist()
        qt  = df["quantity"].astype("float64").tolist()
        cur_et = None
        last_emit = None
        for i in range(len(ets)):
            s = "bid" if sd[i] == "bid" else "ask"
            if qt[i] == 0:
                book[s].pop(px[i], None)
            else:
                book[s][px[i]] = qt[i]
            if cur_et is not None and ets[i] != cur_et:
                b = max(book["bid"]) if book["bid"] else None
                a = min(book["ask"]) if book["ask"] else None
                if b and a and (b, a) != last_emit:
                    rows_ts.append(cur_et * 1_000_000)  # ms -> ns
                    rows_bid.append(b)
                    rows_ask.append(a)
                    last_emit = (b, a)
            cur_et = ets[i]
    if not rows_ts: return None
    out = pd.DataFrame({"ts_ns": rows_ts, "spot_bid": rows_bid, "spot_ask": rows_ask})
    out = out.drop_duplicates(subset="ts_ns", keep="last").sort_values("ts_ns").reset_index(drop=True)
    return out

def merge_fwd_fill(bars, aux, ts_col, val_cols):
    """Forward-fill aux data onto bar timestamps using searchsorted."""
    if aux is None or len(aux) == 0:
        for c in val_cols: bars[c] = np.nan
        return bars
    idx = np.searchsorted(aux["ts_ns"].values, bars["ts"].values, side="right") - 1
    idx = np.clip(idx, 0, len(aux) - 1)
    for c in val_cols:
        bars[c] = aux[c].values[idx]
    return bars

def merge_liquidations(bars, liq):
    """Aggregate liquidation qty per bar window (since last bar)."""
    if liq is None or len(liq) == 0:
        bars["liq_vol_buy"] = 0.0
        bars["liq_vol_sell"] = 0.0
        bars["liq_imbalance"] = 0.0
        return bars
    bar_ts = bars["ts"].values
    buy_vol = np.zeros(len(bar_ts))
    sell_vol = np.zeros(len(bar_ts))
    liq_ts = liq["ts_ns"].values
    liq_side = liq["side"].values
    liq_qty = liq["qty"].values
    idx = np.searchsorted(liq_ts, bar_ts, side="right") - 1
    prev_idx = np.searchsorted(liq_ts, np.concatenate([[0], bar_ts[:-1]]), side="right")
    for i in range(len(bar_ts)):
        lo = prev_idx[i]
        hi = idx[i] + 1 if idx[i] >= 0 else 0
        if lo >= hi: continue
        mask_buy = liq_side[lo:hi] == "BUY"
        mask_sell = liq_side[lo:hi] == "SELL"
        buy_vol[i] = liq_qty[lo:hi][mask_buy].sum()
        sell_vol[i] = liq_qty[lo:hi][mask_sell].sum()
    bars["liq_vol_buy"] = buy_vol
    bars["liq_vol_sell"] = sell_vol
    tot = buy_vol + sell_vol + 1e-10
    bars["liq_imbalance"] = (buy_vol - sell_vol) / tot
    return bars

def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--date", required=True)
    ap.add_argument("--indir", default="/root/depthscope_out")
    ap.add_argument("--outdir", default=None)
    ap.add_argument("--no-basis", action="store_true", help="skip Binance spot reconstruction (slow)")
    a = ap.parse_args()
    if a.outdir is None: a.outdir = a.indir

    key = load_key()
    infile = os.path.join(a.indir, "BTCUSDT_%s_vb10.csv" % a.date)
    if not os.path.exists(infile):
        print("MISSING %s" % infile); return
    bars = pd.read_csv(infile)
    bars["ts"] = bars["ts"].astype("int64")
    print("[enrich] %s: %d bars loaded" % (a.date, len(bars)))

    # Funding
    print("[enrich] pulling funding...")
    fund = pull_funding(a.date, key)
    if fund is not None:
        fund["mark_index_bps"] = (fund["mark"] - fund["index"]) / fund["index"] * 1e4
        bars = merge_fwd_fill(bars, fund, "ts_ns", ["funding_rate", "mark_index_bps"])
        print("[enrich] funding: %d ticks, merged" % len(fund))
    else:
        bars["funding_rate"] = np.nan
        bars["mark_index_bps"] = np.nan
        print("[enrich] funding: NO DATA")

    # OI
    print("[enrich] pulling OI...")
    oidf = pull_oi(a.date, key)
    if oidf is not None:
        bars = merge_fwd_fill(bars, oidf, "ts_ns", ["oi"])
        bars["oi_pct_change"] = bars["oi"].pct_change().fillna(0.0) * 100
        print("[enrich] OI: %d ticks, merged" % len(oidf))
    else:
        bars["oi"] = np.nan
        bars["oi_pct_change"] = np.nan
        print("[enrich] OI: NO DATA")

    # Liquidations
    print("[enrich] pulling liquidations...")
    liq = pull_liquidations(a.date, key)
    bars = merge_liquidations(bars, liq)
    if liq is not None:
        print("[enrich] liquidations: %d events, merged" % len(liq))
    else:
        print("[enrich] liquidations: NO DATA")

    # Basis (Binance spot BBO)
    if not a.no_basis:
        print("[enrich] pulling Binance spot book for basis...")
        spot = pull_binance_bbo(a.date, key)
        if spot is not None and len(spot) > 0:
            bars = merge_fwd_fill(bars, spot, "ts_ns", ["spot_bid", "spot_ask"])
            mid = bars["mid_price"] if "mid_price" in bars.columns else (bars["best_bid"] + bars["best_ask"]) / 2
            bars["basis_bps"] = (bars["best_ask"] - bars["spot_bid"]) / mid * 1e4
            print("[enrich] basis: %d spot BBO ticks, merged" % len(spot))
        else:
            bars["basis_bps"] = np.nan
            print("[enrich] basis: NO DATA")
    else:
        bars["basis_bps"] = np.nan

    # === Rolling-window features (macro-structure: sustained flow, not snapshots) ===
    # Windows: 10=~5min, 25=~15min, 50=~30min, 100=~1h at vb10 bar cadence
    for w in [10, 25, 50, 100]:
        # Cumulative liquidation flow over window
        bars["liq_sell_cum_%d" % w] = bars["liq_vol_sell"].rolling(w, min_periods=1).sum()
        bars["liq_buy_cum_%d" % w] = bars["liq_vol_buy"].rolling(w, min_periods=1).sum()
        tot = bars["liq_sell_cum_%d" % w] + bars["liq_buy_cum_%d" % w] + 1e-10
        bars["liq_flow_imb_%d" % w] = (bars["liq_buy_cum_%d" % w] - bars["liq_sell_cum_%d" % w]) / tot

        # OI % change over window (current vs N bars ago)
        bars["oi_change_%d" % w] = bars["oi"].pct_change(w).fillna(0.0) * 100

        # Average funding over window
        bars["funding_avg_%d" % w] = bars["funding_rate"].rolling(w, min_periods=1).mean()

        # Average mark-index basis over window
        bars["mark_index_avg_%d" % w] = bars["mark_index_bps"].rolling(w, min_periods=1).mean()

        # Cumulative CVD delta over window (sustained buying/selling pressure)
        bars["cvd_cum_%d" % w] = bars["cvd_delta"].rolling(w, min_periods=1).sum()

        # Average depth skew over window (persistent supply/demand)
        bars["ask_skew_avg_%d" % w] = bars["ask_depth_skew"].rolling(w, min_periods=1).mean()
        bars["bid_skew_avg_%d" % w] = bars["bid_depth_skew"].rolling(w, min_periods=1).mean()

        # Cumulative CVD momentum over window
        bars["cvd_mom_cum_%d" % w] = bars["cvd_momentum"].rolling(w, min_periods=1).sum()

    outfile = os.path.join(a.outdir, "BTCUSDT_%s_vb10_enriched.csv" % a.date)
    bars.to_csv(outfile, index=False)
    new_cols = [c for c in bars.columns if c not in ["ts","cum_vol","full_imbalance",
        "top5_imbalance","weighted_imbalance","spread_bps","bid_levels","ask_levels",
        "total_bid_vol","total_ask_vol","ask_concentration","bid_concentration",
        "best_ask_gap_bps","best_bid_gap_bps","mean_ask_gap_bps","mean_bid_gap_bps",
        "cvd_delta","cvd_ratio","cvd_count_imbalance","cvd_momentum","cvd_acceleration",
        "poc_price","va_high","va_low","concentration","mid_to_poc_bps",
        "active_wall_count","wall_cancel_ratio","avg_wall_lifetime_s",
        "bid_wall_vol","ask_wall_vol","ask_vol_top1","ask_vol_top3","ask_vol_top5",
        "ask_vol_top10","ask_vol_top20","ask_vol_top50","ask_vol_top100",
        "bid_vol_top1","bid_vol_top3","bid_vol_top5","bid_vol_top10","bid_vol_top20",
        "bid_vol_top50","bid_vol_top100","ask_conc_ratio","bid_conc_ratio",
        "ask_depth_skew","bid_depth_skew","cross_ask_ratio",
        "depth_breadth_ask","depth_breadth_bid","mid_price","best_bid","best_ask",
        "fwd_ret_15m_bps","fwd_ret_1h_bps","fwd_ret_4h_bps"]]
    print("[enrich] DONE %s -> %s  new_cols=%s" % (a.date, outfile, new_cols))
    print("[enrich] shape: %s" % (bars.shape,))

if __name__ == "__main__":
    main()
