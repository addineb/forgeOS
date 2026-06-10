# BTCUSDT Liquidity Wall Trading Bot — Rebuild Notes

## What This Is
Paper-trading tournament bot that watches Binance Futures BTCUSDT order book for
liquidity walls (large bid/ask levels). When a wall forms and gets hit by aggressive
flow, the bot enters a leveraged trade in the direction the wall implies.

Two simultaneous tournaments run on the same dashboard:
- **Wall** (V1–V5): 5 bots competing with different signal filters
- **LVN** (LV1–LV5): 5 bots competing with same filters but only trade at Low Volume
  Node zones (volume profile gaps)

Live: https://wall-bot-tournament-production.up.railway.app (Railway)

---

## Core Trading Logic

### Signal Direction (CRITICAL — gets confused easily)
```
REAL BID wall hit → SHORT (big buyer absorbed, price will drop)
REAL ASK wall hit → LONG  (big seller absorbed, price will rise)
FAKE BID wall hit → LONG  (wall was fake, price breaks up)
FAKE ASK wall hit → SHORT (wall was fake, price breaks down)
```

### Wall Classification
A wall is REAL if it survives aggressor flow (reload count met, lifetime exceeded).
A wall is FAKE if it disappears quickly under aggressor pressure.

### Signal Scoring (minScore filter)
Each signal gets a score 0–3 based on:
- Reload count (wall absorbed multiple waves): +1
- Aggressor flow ratio above threshold: +1
- Exec ratio above threshold: +1

### Confidence
0.0–1.0 score from combining: execRatio, aggressorFlow, reloadCount, lifetime.

---

## Shared Settings (all bots)

```
startingBalance:      €500
leverage:             20x
takerFee:             0.00025 (0.025%)
makerFee:            -0.00002 (-0.002%, rebate)
minWallQtyBtc:        2.0 BTC (minimum wall size to detect)
wallSignalCooldownMs: 120,000 (2 min per-wall cooldown — same wall can't signal again for 2 min)
globalSignalCooldownMs: 30,000 (30s global cooldown — no bot takes 2 signals within 30s)
aggressorPriceRange:  10 (price ticks to count as aggressor)
historySize:          1000
wallHistoryWindow:    100
minSnapshotsToDetect: 20
minHistoryPoints:     10
wallCleanupMs:        30,000
maxClosedTrades:      500 (DB storage limit per bot)
maxBalanceHistory:    1000
```

---

## Bot Configurations

### V1-BASELINE (color: #6366f1)
"Baseline — current live settings. Control group."
```
tournament:            wall
wallStddevThreshold:   2      (2σ wall detection)
bookLevels:            20
minConfidence:         0.85
minScore:              1
execRatioReal:         0.60
execRatioFake:         0.40
reloadCountReal:       1
lifetimeReal:          10
lifetimeFake:          5
aggressorFlowReal:     0.70
aggressorFlowFake:     0.30
tradeReal:             true
tradeFake:             true
takeProfitPct:         0.0025  (0.25%)
stopLossPct:           0.0050  (0.50%)
maxOpenPositions:      3
positionSizePct:       0.10    (10% of balance per trade)
dailyLossLimitPct:     0.05    (5% daily loss limit)
```

### V2-SNIPER (color: #ef4444)
"Sniper — only perfect signals. 3σ detection, 95% confidence, score≥2."
```
tournament:            wall
wallStddevThreshold:   3
bookLevels:            20
minConfidence:         0.95
minScore:              2
execRatioReal:         0.70
execRatioFake:         0.30
reloadCountReal:       2
lifetimeReal:          15
lifetimeFake:          3
aggressorFlowReal:     0.80
aggressorFlowFake:     0.20
tradeReal:             true
tradeFake:             true
takeProfitPct:         0.0035  (0.35%)
stopLossPct:           0.0050  (0.50%)
maxOpenPositions:      2
positionSizePct:       0.10
dailyLossLimitPct:     0.05
```

### V3-BALANCED (color: #22c55e)
"Balanced R:R — equal TP and SL at 0.50%. Best at 83%+ win rate."
```
tournament:            wall
wallStddevThreshold:   2
bookLevels:            20
minConfidence:         0.85
minScore:              1
execRatioReal:         0.60
execRatioFake:         0.40
reloadCountReal:       1
lifetimeReal:          10
lifetimeFake:          5
aggressorFlowReal:     0.70
aggressorFlowFake:     0.30
tradeReal:             true
tradeFake:             true
takeProfitPct:         0.0050  (0.50%)
stopLossPct:           0.0050  (0.50%)
maxOpenPositions:      3
positionSizePct:       0.10
dailyLossLimitPct:     0.05
```

### V4-REAL-ONLY (color: #f59e0b)
"Real walls only — skip fake wall trades."
```
tournament:            wall
wallStddevThreshold:   2.5
bookLevels:            20
minConfidence:         0.85
minScore:              1
execRatioReal:         0.65
execRatioFake:         0.35
reloadCountReal:       1
lifetimeReal:          12
lifetimeFake:          5
aggressorFlowReal:     0.75
aggressorFlowFake:     0.25
tradeReal:             true
tradeFake:             FALSE  ← key difference
takeProfitPct:         0.0040  (0.40%)
stopLossPct:           0.0050  (0.50%)
maxOpenPositions:      3
positionSizePct:       0.10
dailyLossLimitPct:     0.05
```

### V5-ELITE (color: #a78bfa)
"Elite — combines Sniper quality filters with Balanced R:R."
```
tournament:            wall
wallStddevThreshold:   3
bookLevels:            20
minConfidence:         0.92
minScore:              2
execRatioReal:         0.68
execRatioFake:         0.32
reloadCountReal:       2
lifetimeReal:          12
lifetimeFake:          4
aggressorFlowReal:     0.75
aggressorFlowFake:     0.25
tradeReal:             true
tradeFake:             true
takeProfitPct:         0.0050  (0.50%)
stopLossPct:           0.0050  (0.50%)
maxOpenPositions:      2
positionSizePct:       0.12    (12% of balance)
dailyLossLimitPct:     0.05
```

---

## LVN Bot Configurations

All LVN bots additionally require: price must be inside a Low Volume Node zone
(a gap in the volume profile where very few trades occurred — these are
high-probability reversal or continuation areas).

### LV1-BASE (color: #6366f1)
"LVN Base — wall + LVN filter. 1:3 R:R (SL 0.25%, TP 0.75%)."
```
tournament:            lvn
useLvnFilter:          true
wallStddevThreshold:   2
bookLevels:            20
minConfidence:         0.85
minScore:              1
execRatioReal:         0.60
execRatioFake:         0.40
reloadCountReal:       1
lifetimeReal:          10
lifetimeFake:          5
aggressorFlowReal:     0.70
aggressorFlowFake:     0.30
tradeReal:             true
tradeFake:             true
takeProfitPct:         0.0075  (0.75%)
stopLossPct:           0.0025  (0.25%)
maxOpenPositions:      3
positionSizePct:       0.10
dailyLossLimitPct:     0.05
```

### LV2-SNIPER (color: #ef4444)
"LVN Sniper — LVN filter + score≥2, 95% confidence. 1:3 R:R."
```
tournament:            lvn
useLvnFilter:          true
wallStddevThreshold:   3
bookLevels:            20
minConfidence:         0.95
minScore:              2
execRatioReal:         0.70
execRatioFake:         0.30
reloadCountReal:       2
lifetimeReal:          15
lifetimeFake:          3
aggressorFlowReal:     0.80
aggressorFlowFake:     0.20
tradeReal:             true
tradeFake:             true
takeProfitPct:         0.0075
stopLossPct:           0.0025
maxOpenPositions:      2
positionSizePct:       0.10
dailyLossLimitPct:     0.05
```

### LV3-AGGRESSIVE (color: #22c55e)
"LVN Aggressive — 1:4 R:R (SL 0.25%, TP 1.00%)."
```
tournament:            lvn
useLvnFilter:          true
wallStddevThreshold:   2
bookLevels:            20
minConfidence:         0.85
minScore:              1
execRatioReal:         0.60
execRatioFake:         0.40
reloadCountReal:       1
lifetimeReal:          10
lifetimeFake:          5
aggressorFlowReal:     0.70
aggressorFlowFake:     0.30
tradeReal:             true
tradeFake:             true
takeProfitPct:         0.0100  (1.00%)
stopLossPct:           0.0025  (0.25%)
maxOpenPositions:      3
positionSizePct:       0.10
dailyLossLimitPct:     0.05
```

### LV4-REAL (color: #f59e0b)
"LVN Real-only — REAL walls in LVN zones only. 1:3 R:R."
```
tournament:            lvn
useLvnFilter:          true
wallStddevThreshold:   2.5
bookLevels:            20
minConfidence:         0.85
minScore:              1
execRatioReal:         0.65
execRatioFake:         0.35
reloadCountReal:       1
lifetimeReal:          12
lifetimeFake:          5
aggressorFlowReal:     0.75
aggressorFlowFake:     0.25
tradeReal:             true
tradeFake:             FALSE
takeProfitPct:         0.0075
stopLossPct:           0.0025
maxOpenPositions:      3
positionSizePct:       0.10
dailyLossLimitPct:     0.05
```

### LV5-ELITE (color: #a78bfa)
"LVN Elite — REAL only + score≥2 + LVN. 1:4 R:R. Maximum quality filter."
```
tournament:            lvn
useLvnFilter:          true
wallStddevThreshold:   3
bookLevels:            20
minConfidence:         0.92
minScore:              2
execRatioReal:         0.68
execRatioFake:         0.32
reloadCountReal:       2
lifetimeReal:          12
lifetimeFake:          4
aggressorFlowReal:     0.75
aggressorFlowFake:     0.25
tradeReal:             true
tradeFake:             FALSE
takeProfitPct:         0.0100  (1.00%)
stopLossPct:           0.0025  (0.25%)
maxOpenPositions:      2
positionSizePct:       0.12
dailyLossLimitPct:     0.05
```

---

## Tech Stack

- **Runtime**: Node.js 24, TypeScript 5.9, pnpm workspaces monorepo
- **Backend**: Express 5 API server (port 8080 in production)
- **Database**: PostgreSQL + Drizzle ORM (bot state persisted as JSONB)
- **Validation**: Zod v4, drizzle-zod
- **Build**: esbuild (CJS → ESM bundle, `dist/index.mjs`)
- **Frontend**: React + Vite dashboard (separate artifact)
- **Data source**: Binance Futures WebSocket — `btcusdt@depth20@100ms` + `btcusdt@aggTrade`
- **Logging**: pino (structured JSON logs)

## Architecture: Key Lessons Learned

1. **Put `dist/` in git** (remove from .gitignore for the API server artifact).
   Production VM persistent disk keeps old dist across deployments otherwise.
   The build step in artifact.toml does NOT run on Replit's VM deployment.

2. **Single DB for dev + prod** — dev and production share the same Postgres DB.
   This means dev bot activity can corrupt production state. Either use separate DBs
   or ensure the reset logic can cleanly separate them.

3. **Reset mechanism**: Write a `__RESET__` row to `bot_state` table from dev.
   Production polls every 10s (NODE_ENV=production only), calls resetAllBots()
   which deletes all DB rows and calls hardReset() on every engine.

4. **JSON state files are dangerous** — old JSON files on persistent disk survive
   redeploys. Purge them at startup AND disable JSON fallback in persistence.ts.
   Rely on DB as the only source of truth.

5. **Global signal cooldown** — without a 30s global cooldown, all 10 bots fire
   simultaneously on the same wall signal, making the tournament meaningless.

---

## Dashboard Features
- Live leaderboard sorted by balance (highest first)
- Two tabs: Wall tournament + LVN tournament
- Per-bot stats: balance, P&L, win rate, W/L count, open positions
- Real-time price ticker (from active bot engine)
- Balance history sparklines per bot
- Color-coded by bot identity

