# Project Handoff Notes

_Living scratchpad for context that is not obvious from the code. Update as you go._

## Current live state (as of last session)

- Deployed on Railway service `wall-bot-tournament`; public URL:
  https://wall-bot-tournament-production.up.railway.app
  (`/health` for full JSON, `/health/simple` for uptime checks).
- 11 bots: V1-V6 (wall tournament) + LV1-LV5 (LVN tournament) on BTCUSDT, paper trading.
- **Daily loss limit is currently DISABLED**: `dailyLossLimitPct` was set from `0.05`
  to `1.0` for all bots (commit on 2026-06-06) so the tournament trades continuously
  during the data-gathering phase. A bot now only halts if it loses ~100% of its
  daily starting balance. Re-enable by reverting to `0.05` in
  `artifacts/api-server/src/bot/bot-configs.ts`.

## Open threads / TODO

1. **v4 / lv4 / lv5 never trade (real-only configs).**
   - These three have `tradeFake: false`, so they only fire on a `REAL` wall
     classification, which is produced <~1% of the time -> they sit at zero trades.
   - Root cause is in `engine.ts` `updateWallTrackers`: execution vs cancellation
     attribution is inverted/conflated. A size DECREASE at a wall is booked entirely
     as `executedQty`, and a size INCREASE (stacking/reinforcing) is booked as
     `cancelledQty`. L2 depth alone cannot tell execution from cancellation.
   - Proper fix: use the captured TRADE stream to attribute a size decrease to
     execution only when trades actually printed at that wall price; otherwise treat
     it as cancellation. The engine already tracks `aggressorHits`/`totalHits` per
     wall from trades but does not use them for this split.
   - This is core signal logic — validate any change with `pnpm backtest` over
     captured Parquet BEFORE deploying. Do not change live blindly.

2. **"Step 6d" — exit-side Hyperliquid execution pricing.** Entries are decomposed
   against HL BBO; exits (TP/SL) still fill at the trigger price. Pricing exits off HL
   changes trigger behaviour, so it needs explicit sign-off + backtest validation.

3. **Replay/backtest fidelity** improves as captured history accrues. Trade + HL-quote
   capture (Phase 0) shipped 2026-06-06; replays of windows before that lack those
   streams and fall back to synthetic HL.

## Replay / backtest engine (built 2026-06-06)

- `pnpm replay [--from <iso>] [--to <iso>] [--warmup-min 5]` — re-runs live configs over
  captured ticks and compares vs live `closed_trades`.
- `pnpm backtest --config <overrides.json> [--from] [--to] [--regime ...]` — test config
  variants offline. Override file: `{ "global": {...}, "bots": { "v1": {...} }, "onlyBots": [...] }`.
- Runs require `DATABASE_URL`. Artifacts (meta.json + metrics.json, with a determinism
  hash) land in `artifacts/replay-runs/` (gitignored). See SETUP.md section 9.
- Same `BotEngine` code path as live; only the edges differ (Parquet input, virtual
  clock, injected HL quotes, mocked Redis, DB writes disabled via `WALLBOT_MODE`).

## Origin material

- `docs/origin/` holds the pre-Kiro source: the strategy spec (txt + PDFs), the original
  single-file `liquidity-wall-bot(.js / -FIXED.js)` this project was rebuilt from, and an
  early session log. Reference only; not wired into the build.

## Security reminder

- The GitHub PAT and Railway project token used in past sessions should be ROTATED.
  Never commit real tokens; only `.env.example` (placeholders) is tracked.

## Phase B progress (2026-06-06) — A/B backtest of trade-aware classification

**Goal:** validate the Open-thread #1 fix offline before any live change. Compare
legacy vs `useTradeAwareClassification:true` over captured ticks; the headline
metric is the REAL-vs-FAKE classification rate (today REAL is <1% of signals) and
whether REAL's apparent edge (75% win / +1.00 avg net over 12 live trades vs FAKE
50.3% / -0.06 over 917) survives a like-for-like backtest.

**KEY DISCOVERY to remember (why the fix matters):**
- Edge appears concentrated in REAL walls, but the classifier almost never emits
  REAL because `updateWallTrackers` mis-attributes L2 depth deltas: a wall size
  DECREASE is booked entirely as executed, and a size INCREASE (reload/stack) is
  booked as cancelled. That corrupts the executed/cancelled ratio that drives
  REAL vs FAKE. Fix = `wall-metrics.ts attributeWallDelta()` (flag-gated, legacy
  byte-identical) which uses the TRADE stream to split a decrease into
  executed=min(removed,tradeVol) vs cancelled=remainder, and treats increases as
  reloads. Needs the trade stream (`executedVolBuffer` from `processAggTrade`).
- v4 / lv4 / lv5 (`tradeFake:false`) sit at zero trades because of this. If the
  fix raises the REAL rate, those bots should start trading in backtest.

**Data:** pulled `/data/ticks` from the Railway volume to local `./data/ticks`.
Coverage 2026-06-06 UTC: depth hours 00-11; trade + hlquote hours 05-11 (Phase 0
capture went live ~05:00). So the only valid A/B window (needs all 3 streams) is
**05:00-12:00 UTC**. Current-hour partial files (`12.parquet`, no parquet footer)
were dropped.

**How the pull worked (for next time):** `railway ssh` opens an interactive PTY
that hangs agent tooling, but it DOES run a passed command and stream stdout first.
Working recipe: register an SSH key (`railway ssh keys add`), pre-seed
`known_hosts` (`ssh-keyscan ssh.railway.com`), then run via cmd.exe (NOT PS) so
binary is not corrupted:
`railway ssh -p <proj> -e <env> -s <svc> -i <key> "cd /data && tar cf - ticks" > ticks.tar`
wrapped in a PowerShell Start-Job with Wait-Job -Timeout to kill the lingering PTY
after the stream completes. Verify parquet footers (last 4 bytes == `PAR1`).
Railway's own chatter goes to stderr, so stdout is clean tar bytes.

**PERF BOTTLENECK (FOLLOW-UP):** the backtest is far below the 100x-realtime target
- ~1 hour of depth (~36k snapshots x 11 bots) takes >5 min / ~400s CPU locally.
  Cause: per-snapshot wall detection in JS (slice + map + median/stddev over the
  history window x book levels, x11 bots). Full 7h ~= 25 min/run. TODO before this
  is a comfortable EDGE-iteration loop: optimise the hot path (precompute rolling
  median/stddev, avoid slice/map allocs, or run bots in parallel workers), and/or a
  faster/incremental snapshot download (scheduled hourly pull, or compress better).
## Perf work (2026-06-06) — feat/perf-detectwalls (merged to main)

Backtest hot path was ~7-10x realtime, far below the 100x target. Profiling
(`artifacts/perf-profiles/hot5min.cpuprofile`) showed `median` + its sort
comparator = 58.6% of CPU and `detectWalls` +14.6% (wall math ~73%); Parquet
I/O only ~10%. Two low-risk mechanical changes (candidate A + C; B/quickselect
deferred):
- A: `median` uses native `Float64Array.from(values).sort()` (no JS
  comparator); hoisted the `bookHistory.slice` out of the per-level loop.
- C: single-pass per-level history extraction; `stddev` uses `d*d` instead
  of `Math.pow` in a plain loop.
Result: **~2.2x faster** (hour-11: 484s->222s; 7x->16x realtime), **2.4x fewer
CPU samples**, and **byte-identical** output — legacy determinism hashes
reproduced exactly (`15f8eb71...` 11:00-12:00, `f9fdf40e...` 11:30-11:35) and
50/50 vitest green. Post-change hot path: `median` native sort still #1 at
39.4% (the comparator frame is gone) -> that is the target if candidate B
(quickselect) is ever needed. Real path to 100x = worker-thread parallelism
across the 11 independent bots. Profiles saved in `artifacts/perf-profiles/`.