import pandas as pd, glob, numpy as np
def run(DATE, thr=20.0, maxhold=60):
    tf=sorted(glob.glob(f"/root/chd/data/ticks/BTCUSDT/trade/{DATE}/*.parquet"))
    qf=sorted(glob.glob(f"/root/chd/data/ticks/BTC/hlquote/{DATE}/*.parquet"))
    if not tf or not qf: print(f"{DATE}: missing"); return
    tr=pd.concat([pd.read_parquet(f,columns=["ts","price"]) for f in tf]).sort_values("ts").rename(columns={"price":"bn"})
    hq=pd.concat([pd.read_parquet(f,columns=["ts","mid"]) for f in qf]).sort_values("ts").rename(columns={"mid":"hl"})
    m=pd.merge_asof(hq,tr,on="ts",direction="backward").dropna().reset_index(drop=True)
    m["gap"]=(m["hl"]-m["bn"])/m["bn"]*1e4
    m["base"]=m["gap"].rolling(200,min_periods=20).mean()
    m["dev"]=m["gap"]-m["base"]
    hl=m["hl"].values; dev=m["dev"].values; ts=m["ts"].values
    n=len(m); i=0; trades=[]
    while i<n-1:
        if not np.isfinite(dev[i]) or abs(dev[i])<thr: i+=1; continue
        d=-1 if dev[i]>0 else 1   # dev>0 (HL rich) -> short
        entry=hl[i]; j=i+1; held=0
        while j<n and held<maxhold and np.isfinite(dev[j]) and np.sign(dev[j])==np.sign(dev[i]):
            j+=1; held+=1
        ex=hl[min(j,n-1)]
        pnl=d*(ex-entry)/entry*1e4
        trades.append((pnl,(ts[min(j,n-1)]-ts[i])/1000.0))
        i=j+1
    if not trades: print(f"{DATE}: no trades"); return
    p=np.array([t[0] for t in trades]); hold_s=np.array([t[1] for t in trades])
    g=p.mean()
    print(f"{DATE}: trades/day={len(p)} gross/trade={g:.2f}bps win%={(p>0).mean()*100:.1f} avg_hold_s={hold_s.mean():.1f}")
    for cost,lbl in [(11,'taker11'),(5,'mixed5'),(2,'maker2'),(0,'gross0')]:
        net=g-cost; tot=(g-cost)*len(p)
        print(f"    net@{lbl}={net:.2f}bps/trade  day_total~{tot:.0f}bps")
for D in ["2026-02-01","2025-12-01","2026-05-01"]:
    run(D)
