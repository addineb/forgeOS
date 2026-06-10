import pandas as pd, glob, numpy as np
COST=11.0
for DATE in ["2026-02-01","2025-12-01","2026-05-01"]:
    tf=sorted(glob.glob(f"/root/chd/data/ticks/BTCUSDT/trade/{DATE}/*.parquet"))
    qf=sorted(glob.glob(f"/root/chd/data/ticks/BTC/hlquote/{DATE}/*.parquet"))
    if not tf or not qf:
        print(f"{DATE}: missing data (trade={len(tf)} hl={len(qf)})"); continue
    tr=pd.concat([pd.read_parquet(f,columns=["ts","price"]) for f in tf]).sort_values("ts").rename(columns={"price":"bn"})
    hq=pd.concat([pd.read_parquet(f,columns=["ts","mid"]) for f in qf]).sort_values("ts").rename(columns={"mid":"hl"})
    m=pd.merge_asof(hq,tr,on="ts",direction="backward").dropna()
    m["gap_bps"]=(m["hl"]-m["bn"])/m["bn"]*1e4
    # de-trend the slow basis: gap minus its rolling mean -> the tradeable deviation
    m["base"]=m["gap_bps"].rolling(200,min_periods=20).mean()
    m["dev"]=m["gap_bps"]-m["base"]
    m["hl_ret_bps"]=(m["hl"].shift(-1)-m["hl"])/m["hl"]*1e4
    m=m.dropna()
    print(f"=== {DATE}  hl_updates={len(hq)} ===")
    for thr in [11,20,30]:
        s=m[m["dev"].abs()>thr]
        if len(s)==0: print(f"  dev>{thr}: none"); continue
        pnl=-np.sign(s["dev"])*s["hl_ret_bps"]   # bet reversion of the DEVIATION
        net=pnl.mean()-COST
        print(f"  |dev|>{thr}bps: trades/day={len(s)} gross={pnl.mean():.2f}bps net(after{COST})={net:.2f}bps win%={(pnl>0).mean()*100:.1f}")
