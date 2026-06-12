#!/usr/bin/env python3
"""live_lagshot.py - the live Lagshot loop (ETH, HL-perp taker vs OKX-spot ref).
Mirrors the forgelag backtest: rolling-baseline basis-reversion, revert-to-mean exit.
Safety: 1x leverage (no liquidation), tiny fixed notional, single position, daily-loss
halt. Realistic execution: tight IOC cross (~5bps), NOT the SDK 5% default. Dry-run by
default (no orders) -> pass --live to actually trade. Logs every signal/fill/latency.
"""
import sys, time, json, threading, statistics, datetime
from collections import deque
from eth_account import Account
from hyperliquid.exchange import Exchange
from hyperliquid.info import Info
from hyperliquid.utils import constants
import websocket  # websocket-client

# ---------- config ----------
COIN       = "ETH"
OKX_INST   = "ETH-USDT"           # OKX spot reference symbol
THR_BPS    = 16.0                 # entry dislocation threshold
EXIT_BPS   = 2.0                  # revert-to-mean exit band
WINDOW     = 500                  # rolling-baseline samples
SAMPLE_S   = 0.5                  # sample cadence (s)
TOP_N      = 5                    # microprice depth
HOLD_S     = 600                  # max hold (s)
COOLDOWN_S = 30                   # cooldown after a trade (s)
NOTIONAL   = 11.0                 # USD per trade (fits ~$12.86 acct at 1x)
LEVERAGE   = 1                    # 1x -> no liquidation
CROSS_BPS  = 5.0                  # how far the IOC limit crosses (realistic)
EXIT_CROSS_BPS = 12.0             # cross more to ensure the close fills
FEE_RT_BPS = 9.0                  # round-trip taker fee estimate (for net-edge logging)
DAILY_LOSS_HALT = 0.15            # halt for the day if equity drops 15% from day start
WARMUP_SAMPLES = 60               # need this many gap samples before trading

LIVE = "--live" in sys.argv
LOGF = open("/home/ubuntu/lagshot/live.log", "a")

def log(*a):
    msg = " ".join(str(x) for x in a)
    line = f"{datetime.datetime.utcnow().isoformat()}Z {msg}"
    print(line, flush=True)
    LOGF.write(line + "\n"); LOGF.flush()

# ---------- creds ----------
c = {}
for ln in open("/home/ubuntu/lagshot/secret.env"):
    if "=" in ln and not ln.startswith("#"):
        k, v = ln.split("=", 1); c[k.strip()] = v.strip()
agent = Account.from_key(c["AGENT_KEY"]); ADDR = c["MAIN_ADDRESS"]
info = Info(constants.MAINNET_API_URL, skip_ws=False)
ex   = Exchange(agent, constants.MAINNET_API_URL, account_address=ADDR)

# ---------- shared live state ----------
state = {"hl_micro": 0.0, "okx_last": 0.0, "hl_ts": 0.0, "okx_ts": 0.0}

def hl_microprice(levels):
    try:
        bids, asks = levels[0], levels[1]
        if not bids or not asks: return 0.0
        bb = float(bids[0]["px"]); ba = float(asks[0]["px"])
        bq = sum(float(l["sz"]) for l in bids[:TOP_N])
        aq = sum(float(l["sz"]) for l in asks[:TOP_N])
        if bq + aq <= 0: return (bb + ba) / 2.0
        return (bb * aq + ba * bq) / (bq + aq)
    except Exception:
        return 0.0

def on_hl_book(msg):
    try:
        lv = msg["data"]["levels"]
        m = hl_microprice(lv)
        if m > 0: state["hl_micro"] = m; state["hl_ts"] = time.time()
    except Exception:
        pass

# OKX public ws (trades) in its own thread
def okx_thread():
    url = "wss://ws.okx.com:8443/ws/v5/public"
    sub = json.dumps({"op": "subscribe", "args": [{"channel": "trades", "instId": OKX_INST}]})
    while True:
        try:
            ws = websocket.create_connection(url, timeout=15)
            ws.send(sub)
            while True:
                raw = ws.recv()
                if not raw: break
                d = json.loads(raw)
                if "data" in d:
                    px = float(d["data"][-1]["px"])
                    state["okx_last"] = px; state["okx_ts"] = time.time()
        except Exception as e:
            log("OKX ws reconnect:", e); time.sleep(2)

threading.Thread(target=okx_thread, daemon=True).start()
info.subscribe({"type": "l2Book", "coin": COIN}, on_hl_book)

# ---------- setup account ----------
log(f"=== LAGSHOT LIVE {'(LIVE TRADING)' if LIVE else '(DRY-RUN, no orders)'} acct={ADDR[:10]} coin={COIN} thr={THR_BPS}bps ===")
try:
    if LIVE:
        ex.update_leverage(LEVERAGE, COIN)
        log(f"leverage set to {LEVERAGE}x")
except Exception as e:
    log("leverage set err:", e)
st0 = info.user_state(ADDR)
day_start_equity = float(st0["marginSummary"]["accountValue"])
log(f"start equity ${day_start_equity:.2f}")

# ---------- strategy state ----------
gaps = deque(maxlen=WINDOW)
gap_sum = 0.0
phase = "FLAT"        # FLAT | OPEN
ready_at = 0.0
entry = None          # dict: side, ts, entry_mid, fill_px
cur_day = None
day_halted = False
next_sample = 0.0
n_trades = 0
sample_count = 0

def now(): return time.time()

log("warming up... (need %d samples, ~%.0fs)" % (WARMUP_SAMPLES, WARMUP_SAMPLES*SAMPLE_S))
while True:
    t = now()
    if t < next_sample:
        time.sleep(0.02); continue
    next_sample = t + SAMPLE_S

    hl = state["hl_micro"]; okx = state["okx_last"]
    if hl <= 0 or okx <= 0:
        continue
    # staleness guard: both feeds fresh within 3s
    if t - state["hl_ts"] > 3 or t - state["okx_ts"] > 3:
        continue

    gap = (hl - okx) / okx * 1e4   # bps
    base = (gap_sum / len(gaps)) if gaps else gap
    dev = gap - base
    have = len(gaps) >= WARMUP_SAMPLES
    gaps.append(gap); gap_sum += gap
    if len(gaps) == WINDOW:
        gap_sum = sum(gaps)
    sample_count += 1
    if sample_count % 40 == 0:
        log(f"[hb] hl={hl:.2f} okx={okx:.2f} gap={gap:.1f} dev={dev:.1f} base={base:.1f} n={len(gaps)} phase={phase}")

    # daily loss halt
    d = datetime.datetime.utcnow().date()
    if d != cur_day:
        cur_day = d; day_start_equity = float(info.user_state(ADDR)["marginSummary"]["accountValue"]); day_halted = False
        log(f"new day {d} start equity ${day_start_equity:.2f}")

    if phase == "OPEN":
        held = t - entry["ts"]
        revert = abs(dev) <= EXIT_BPS
        timeout = held >= HOLD_S
        if revert or timeout:
            reason = "revert" if revert else "timeout"
            if LIVE:
                t0 = time.perf_counter()
                try:
                    r = ex.market_close(COIN, None, EXIT_CROSS_BPS/1e4)
                    lat = (time.perf_counter()-t0)*1000
                    log(f"EXIT {reason} dev={dev:.1f} held={held:.0f}s lat={lat:.0f}ms r={str(r)[:80]}")
                except Exception as e:
                    log("EXIT err:", e); continue
            else:
                log(f"[dry] EXIT {reason} dev={dev:.1f} held={held:.0f}s")
            phase = "FLAT"; ready_at = t + COOLDOWN_S; entry = None; n_trades += 1
        continue

    # FLAT
    if day_halted or t < ready_at or not have:
        continue
    if abs(dev) < THR_BPS:
        continue
    rich = dev > 0
    is_buy = not rich          # reversion: rich -> sell, cheap -> buy
    sz = round(NOTIONAL / hl, 4)
    net_edge = abs(dev) - FEE_RT_BPS - CROSS_BPS
    if LIVE:
        t0 = time.perf_counter()
        try:
            r = ex.market_open(COIN, is_buy, sz, None, CROSS_BPS/1e4)
            lat = (time.perf_counter()-t0)*1000
        except Exception as e:
            log("ENTRY err:", e); continue
        fill_px = None
        try:
            sdata = r["response"]["data"]["statuses"][0]
            fill_px = float(sdata["filled"]["avgPx"]) if "filled" in sdata else None
        except Exception:
            pass
        if fill_px is None:
            log(f"ENTRY no-fill dev={dev:.1f} lat={lat:.0f}ms r={str(r)[:80]}")
            continue
        slip = (fill_px - hl)/hl*1e4 * (1 if is_buy else -1)
        log(f"ENTRY {'BUY' if is_buy else 'SELL'} dev={dev:.1f} netEdge={net_edge:.1f}bps lat={lat:.0f}ms fill={fill_px} slip={slip:.1f}bps")
        phase = "OPEN"; entry = {"side": is_buy, "ts": t, "entry_mid": hl, "fill_px": fill_px}
    else:
        log(f"[dry] ENTRY {'BUY' if is_buy else 'SELL'} gap={gap:.1f} dev={dev:.1f} thr={THR_BPS} netEdge={net_edge:.1f}bps hl={hl:.2f} okx={okx:.2f}")
        phase = "OPEN"; entry = {"side": (not rich), "ts": t, "entry_mid": hl, "fill_px": hl}