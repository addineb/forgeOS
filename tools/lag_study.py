import pandas as pd, glob, numpy as np
DATE="2026-02-01"; COST=11.0
tf=sorted(glob.glob(f"/root/chd/data/ticks/BTCUSDT/trade/{DATE}/*.parquet"))
qf=sorted(glob.glob(f"/root/chd/data/ticks/BTC/hlquote/{DATE}/*.parquet"))
print(f"date={DATE}  trade_files={len(tf)} hl_files={len(qf)}")
tr=pd.concat([pd.read_parquet(f,columns=["ts","price"]) for f in tf]).sort_values("ts").rename(columns={"price":"bn"})
hq=pd.concat([pd.read_parquet(f,columns=["ts","mid"]) for f in qf]).sort_values("ts").rename(columns={"mid":"hl"})
print(f"binance trades={len(tr)}  hl quotes={len(hq)}")
dt=hq["ts"].diff().dropna()/1000.0
print(f"HL cadence: updates/day={len(hq)}  median_gap_s={dt.median():.1f}  p90_gap_s={dt.quantile(.9):.1f}")
m=pd.merge_asof(hq, tr, on="ts", direction="backward").dropna()
m["gap_bps"]=(m["hl"]-m["bn"])/m["bn"]*1e4
ab=m["gap_bps"].abs()
print(f"HL-vs-Binance gap (bps): median={ab.median():.2f} p90={ab.quantile(.9):.2f} p99={ab.quantile(.99):.2f} max={ab.max():.2f}")
print(f"  (note: gap = mostly spot(Binance) vs perp(HL) BASIS, not lag)")
trad=int((ab>COST).sum())
print(f"|gap|>{COST}bps (cost wall): {trad}/day = {100*trad/len(m):.1f}% of updates")
m["hl_ret_bps"]=(m["hl"].shift(-1)-m["hl"])/m["hl"]*1e4
g=m["gap_bps"].fillna(0).values; r=m["hl_ret_bps"].fillna(0).values
c=np.corrcoef(g,r)[0,1]
print(f"corr(gap, HL_next_move)={c:.3f}  (negative => HL reverts toward Binance = lag/basis pull)")
