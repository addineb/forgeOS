#!/usr/bin/env python3
import io, requests, pandas as pd
try: import zstandard as zstd
except: zstd=None
key=open("/root/.chd_key").read().strip().split("=")[-1].strip().strip('"').strip("'")
def dl(path):
    u="https://api.cryptohftdata.com/download?file=%s&api_key=%s"%(path,key)
    r=requests.get(u,timeout=30)
    if r.status_code!=200: return r.status_code, None
    raw=r.content
    if zstd:
        try: raw=zstd.ZstdDecompressor().decompress(raw)
        except: pass
    try: return 200, pd.read_parquet(io.BytesIO(raw))
    except Exception as e: return 200, "PARSE_FAIL:%s len=%d"%(e,len(raw))
# Funding (mark_price stream)
cands=[
  "hyperliquid_futures/2026-02-01/12/BTC_mark_price.parquet.zst",
  "hyperliquid_futures/2026-02-01/08/BTC_mark_price.parquet.zst",
  "hyperliquid_futures/2026-02-01/00/BTC_mark_price.parquet.zst",
]
for c in cands:
    sc,df=dl(c)
    if sc==200 and isinstance(df,pd.DataFrame):
        print("OK  %-70s rows=%d cols=%s"%(c,len(df),list(df.columns)))
        print(df.head(3).to_string()); print("---")
    else: print("NO(%s) %s"%(sc,c))
# OI
cands2=[
  "hyperliquid_futures/2026-02-01/12/BTC_open_interest.parquet.zst",
  "hyperliquid_futures/2026-02-01/00/BTC_open_interest.parquet.zst",
]
for c in cands2:
    sc,df=dl(c)
    if sc==200 and isinstance(df,pd.DataFrame):
        print("OK  %-70s rows=%d cols=%s"%(c,len(df),list(df.columns)))
        print(df.head(3).to_string()); print("---")
    else: print("NO(%s) %s"%(sc,c))
