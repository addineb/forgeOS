#!/usr/bin/env python3
"""live_lagshot.py v2 - live Lagshot loop (ETH, HL-perp taker vs OKX-spot ref).
EXCHANGE is the source of truth: a position-poll thread reads the real position every 2s;
the state machine acts on REAL position, so a failed close just retries (no desync).
Both market feeds are REST polls. 1x leverage (no liquidation), tiny notional, daily halt.
Dry-run by default; --live to trade."""
import sys, time, threading, datetime
from collections import deque
from eth_account import Account
from hyperliquid.exchange import Exchange
from hyperliquid.info import Info
from hyperliquid.utils import constants
import requests

COIN="ETH"; OKX_INST="ETH-USDT"
THR_BPS=16.0; EXIT_BPS=2.0
WINDOW=500; SAMPLE_S=0.5; TOP_N=5
HOLD_S=600; COOLDOWN_S=30
NOTIONAL=11.0; LEVERAGE=1
CROSS_BPS=50.0; EXIT_CROSS_BPS=50.0; FEE_RT_BPS=9.0  # wide tolerance to GUARANTEE taker fill (matches backtest market-taker); real slippage is logged, not the tolerance
DAILY_LOSS_HALT=0.15; WARMUP_SAMPLES=60; POS_EPS=1e-9

LIVE="--live" in sys.argv
LOGF=open("/home/ubuntu/lagshot/live.log","a")
def log(*a):
    line=f"{datetime.datetime.now(datetime.UTC).isoformat()} "+" ".join(str(x) for x in a)
    print(line,flush=True); LOGF.write(line+"\n"); LOGF.flush()

c={}
for ln in open("/home/ubuntu/lagshot/secret.env"):
    if "=" in ln and not ln.startswith("#"):
        k,v=ln.split("=",1); c[k.strip()]=v.strip()
agent=Account.from_key(c["AGENT_KEY"]); ADDR=c["MAIN_ADDRESS"]
info=Info(constants.MAINNET_API_URL, skip_ws=True)
ex=Exchange(agent, constants.MAINNET_API_URL, account_address=ADDR)
state={"hl_micro":0.0,"okx_last":0.0,"hl_ts":0.0,"okx_ts":0.0,"pos":0.0,"equity":0.0,"pos_ts":0.0}

def hl_microprice(levels):
    try:
        bids,asks=levels[0],levels[1]
        if not bids or not asks: return 0.0
        bb=float(bids[0]["px"]); ba=float(asks[0]["px"])
        bq=sum(float(l["sz"]) for l in bids[:TOP_N]); aq=sum(float(l["sz"]) for l in asks[:TOP_N])
        if bq+aq<=0: return (bb+ba)/2.0
        return (bb*aq+ba*bq)/(bq+aq)
    except Exception: return 0.0

def hl_thread():
    f=0
    while True:
        try:
            b=info.l2_snapshot(COIN); m=hl_microprice(b["levels"])
            if m>0: state["hl_micro"]=m; state["hl_ts"]=time.time(); f=0
        except Exception as e:
            f+=1
            if f%20==1: log("HL REST poll error:",e)
        time.sleep(0.3)

def okx_thread():
    url="https://www.okx.com/api/v5/market/ticker?instId="+OKX_INST; s=requests.Session(); f=0
    while True:
        try:
            r=s.get(url,timeout=3); px=float(r.json()["data"][0]["last"])
            state["okx_last"]=px; state["okx_ts"]=time.time(); f=0
        except Exception as e:
            f+=1
            if f%20==1: log("OKX REST poll error:",e)
        time.sleep(0.4)

def pos_thread():
    f=0
    while True:
        try:
            st=info.user_state(ADDR); eq=float(st["marginSummary"]["accountValue"]); pz=0.0
            for p in st.get("assetPositions",[]):
                if p["position"]["coin"]==COIN: pz=float(p["position"]["szi"])
            state["pos"]=pz; state["equity"]=eq; state["pos_ts"]=time.time(); f=0
        except Exception as e:
            f+=1
            if f%20==1: log("POS poll error:",e)
        time.sleep(2.0)

threading.Thread(target=hl_thread,daemon=True).start()
threading.Thread(target=okx_thread,daemon=True).start()
threading.Thread(target=pos_thread,daemon=True).start()

log(f"=== LAGSHOT LIVE {'(LIVE)' if LIVE else '(DRY)'} acct={ADDR[:10]} coin={COIN} thr={THR_BPS}bps ===")
try:
    if LIVE: ex.update_leverage(LEVERAGE,COIN); log(f"leverage {LEVERAGE}x")
except Exception as e: log("leverage err:",e)
for _ in range(10):
    if state["pos_ts"]>0: break
    time.sleep(1)
day_start_equity=state.get("equity",0.0) or float(info.user_state(ADDR)["marginSummary"]["accountValue"])
log(f"start equity ${day_start_equity:.2f} start pos {state.get('pos',0.0)}")

gaps=deque(maxlen=WINDOW); gap_sum=0.0
ready_at=0.0; entry_ts=0.0; prev_pos=state.get("pos",0.0); pending_until=0.0
cur_day=None; day_halted=False; next_sample=0.0; sc=0; loop_n=0
def now(): return time.time()
log("warming up... (need %d samples)"%WARMUP_SAMPLES)
while True:
    t=now()
    if t<next_sample: time.sleep(0.02); continue
    next_sample=t+SAMPLE_S
    hl=state["hl_micro"]; okx=state["okx_last"]; loop_n+=1
    if hl<=0 or okx<=0 or t-state["hl_ts"]>3 or t-state["okx_ts"]>3 or t-state["pos_ts"]>10:
        if loop_n%60==0: log(f"[wait] stale hl_age={t-state['hl_ts']:.0f} okx_age={t-state['okx_ts']:.0f} pos_age={t-state['pos_ts']:.0f}")
        continue
    gap=(hl-okx)/okx*1e4; base=(gap_sum/len(gaps)) if gaps else gap; dev=gap-base
    have=len(gaps)>=WARMUP_SAMPLES
    gaps.append(gap); gap_sum+=gap
    if len(gaps)==WINDOW: gap_sum=sum(gaps)
    sc+=1
    pos=state.get("pos",0.0)
    if abs(prev_pos)<=POS_EPS and abs(pos)>POS_EPS: entry_ts=t; log(f"POS OPENED {pos} entry_dev={dev:.1f}")
    if abs(prev_pos)>POS_EPS and abs(pos)<=POS_EPS: ready_at=t+COOLDOWN_S; log("POS CLOSED -> cooldown")
    prev_pos=pos
    if sc%40==0: log(f"[hb] hl={hl:.2f} okx={okx:.2f} gap={gap:.1f} dev={dev:.1f} base={base:.1f} pos={pos} eq={state.get('equity',0):.2f}")
    d=datetime.datetime.now(datetime.UTC).date()
    if d!=cur_day: cur_day=d; day_start_equity=state.get("equity",day_start_equity); day_halted=False; log(f"new day {d} eq ${day_start_equity:.2f}")
    if state.get("equity",1e9)<day_start_equity*(1-DAILY_LOSS_HALT) and not day_halted: day_halted=True; log("*** DAILY LOSS HALT ***")
    if t<pending_until: continue
    if abs(pos)>POS_EPS:
        held=(t-entry_ts) if entry_ts>0 else 0
        revert=have and abs(dev)<=EXIT_BPS
        if revert or held>=HOLD_S:
            reason="revert" if revert else "timeout"
            if LIVE:
                t0=time.perf_counter()
                try:
                    r=ex.market_close(COIN,slippage=EXIT_CROSS_BPS/1e4); lat=(time.perf_counter()-t0)*1000
                    okc=isinstance(r,dict) and r.get("status")=="ok" and "'error'" not in str(r)
                    log(f"EXIT {reason} dev={dev:.1f} held={held:.0f}s lat={lat:.0f}ms ok={okc} r={str(r)[:140]}")
                except Exception as e: log("EXIT err:",repr(e))
            else: log(f"[dry] EXIT {reason} dev={dev:.1f} held={held:.0f}s")
            pending_until=t+4
        continue
    if day_halted or t<ready_at or not have: continue
    if abs(dev)<THR_BPS: continue
    rich=dev>0; is_buy=not rich; sz=round(NOTIONAL/hl,4); net_edge=abs(dev)-FEE_RT_BPS-CROSS_BPS
    if LIVE:
        t0=time.perf_counter()
        try:
            r=ex.market_open(COIN,is_buy,sz,None,CROSS_BPS/1e4); lat=(time.perf_counter()-t0)*1000
            fill=None
            try:
                sd=r["response"]["data"]["statuses"][0]; fill=float(sd["filled"]["avgPx"]) if "filled" in sd else None
            except Exception: pass
            slip=((fill-hl)/hl*1e4*(1 if is_buy else -1)) if fill else 0.0
            log(f"ENTRY {'BUY' if is_buy else 'SELL'} dev={dev:.1f} netEdge={net_edge:.1f}bps lat={lat:.0f}ms fill={fill} slip={slip:.1f}bps ok={chr(39)+'error'+chr(39) not in str(r)} r={str(r)[:120]}")
        except Exception as e: log("ENTRY err:",repr(e))
        pending_until=t+4
    else:
        log(f"[dry] ENTRY {'BUY' if is_buy else 'SELL'} dev={dev:.1f} netEdge={net_edge:.1f}bps")
        pending_until=t+2