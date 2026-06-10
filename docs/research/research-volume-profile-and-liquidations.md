# Research Note -- Volume Profile (Auction Theory) + Liquidation Heatmap (2026-06-09)

From the order-flow research session (see `docs/research-charter.md`). Two future
bots requested: a Volume/Market-Profile bot, and a Liquidation-heatmap bot.
Same ground rules as `docs/research-flow-primitives.md`: additive/flag-gated,
validated in `pnpm backtest` before any live touch, every bar net of ~11 bps.

Headline verdicts:
- Volume Profile bot ...... PURSUE (no new data; primitive already exists).
- Liquidation bot ......... PARK -- DATA-BLOCKED. Needs a capture build first.
  Split: cascade-fade variant = PURSUE-AFTER-CAPTURE (cheap); magnet variant =
  PARK (bigger build, and it is an estimate of an estimate).

================================================================================
PART A -- VOLUME / MARKET PROFILE BOT  (verdict: PURSUE)
================================================================================

## Theory / mechanism
Market Profile (Peter Steidlmayer, CBOT 1984-85) and Auction Market Theory
(popularised by Dalton, "Mind Over Markets"): a market auctions between BALANCE
and IMBALANCE (balance -> trend -> balance, repeating). Key objects:
- POC (Point of Control): highest-volume price = the "fairest price"; acts as a
  MAGNET that price rotates around.
- Value Area (VA): the price band holding ~70% of volume around the POC
  (Steidlmayer's method: expand out from VPOC, add the higher-volume adjacent
  side until 70% is captured).
- HVN (High-Volume Node): thick shelves = support/resistance, price slows there.
- LVN (Low-Volume Node): thin nodes = price travels through them FAST to the next
  HVN (rejection / fast-fill).
The tradeable edges: mean-reversion to POC inside balance; acceptance/rejection
at value-area edges; and fast travel across LVNs toward the next HVN.

## Evidence / confidence
PRACTITIONER-heavy (Steidlmayer "Trading with Market Profile"; Dalton "Mind Over
Markets"); thin peer-reviewed validation. The mechanism is consistent with the
impact literature, though: price needs order-flow IMBALANCE to leave a
high-volume level, and reverts toward it without it (ties to our OFI/propagator
findings). Treat as a structured hypothesis, not an established result.

## What we already have
`artifacts/api-server/src/replay/primitives/volume-profile-primitive.ts` already
computes POC / VA (VAH/VAL) / LVN / HVN over a rolling 15-min window AND emits a
breakout-vs-revert decision at an LVN (strong one-sided flow -> breakout toward
next HVN; weak flow -> revert toward POC). This is the LV1-LV5 thesis, done
properly. So the bot is mostly WIRING + theory-completeness, not new research.

## Gaps vs the full theory (the upgrades)
1. SINGLE rolling window only. AMT works across timeframes -> add SESSION,
   DEVELOPING (intraday so far), and COMPOSITE (multi-day) profiles. The bias
   comes from the higher-timeframe profile; the trigger from the developing one.
2. No profile SHAPE classification (Normal / P / b / Double-Distribution /
   Trend). Shape is the balance-vs-imbalance read -> add it as the regime gate.
3. No VALUE-MIGRATION / acceptance-rejection logic (open above/below prior value;
   does price ACCEPT (trade & hold) or REJECT a level). This is the core Dalton
   decision and we don't have it.
4. No "naked"/untested POCs or single-prints retained as magnets/targets across
   sessions (untested levels are the highest-quality targets).

## Verdict: PURSUE
Strongest "future bot" because it needs NO new data (trades only, which we have)
and the primitive exists. The LVN->HVN travel and VA-edge rejection moves can be
large enough to clear 11 bps (unlike microstructure scalps).

## Experiment spec (hand to build session)
- Signal: VolumeProfile primitive decision at price (breakout/revert), gated by
  the new profile-shape regime + value-area relationship.
- Grid: profileWindow/session {developing, 1-session, multi-day composite};
  lvnPercentile {0.15,0.20,0.30}; valueAreaPct {0.68,0.70}; bucketSize
  {5,10,20}; flowThreshold {0.5,0.6,0.7}; mode {revert-only, breakout-only, both};
  hold {30,60,120,300}s; TP/SL anchored to STRUCTURE (POC / next HVN / VA edge)
  rather than fixed pct.
- Gate: trade reverts only inside BALANCE shape; breakouts only on IMBALANCE.
- Success bar: net P&L/trade > 0 after 11 bps; DSR > 0; OOS sign-consistent
  (IS/OOS split); structural-target MFE >= cost.
================================================================================
PART B -- LIQUIDATION HEATMAP BOT  (verdict: PARK -- DATA-BLOCKED)
================================================================================

## Two distinct theses (do not conflate them)
1. MAGNET: price is drawn toward dense UN-SWEPT liquidation clusters, because
   clearing them triggers cascades of forced orders. Trade TOWARD the nearest
   large cluster. Needs an ESTIMATED heatmap (open interest + leverage tiers).
2. CASCADE-FADE: when a cluster IS swept, a forced-liquidation BURST prints
   (one-sided aggressor spike + price spike + a forceOrder burst); the move is
   exhaustion -> FADE the reversal. Needs realised liquidation prints.

## Critical data nuance
A "liquidation heatmap" (Coinglass / Hyblock style) is NOT raw data. It is a
MODEL: aggregate open interest + assumed leverage tiers (5x/10x/25x/50x/100x) +
long/short positioning, back-solve liquidation prices, volume-weight into bins.
It is an ESTIMATE of where stops cluster. Bright bands act as magnets. Treat band
locations as a probabilistic prior, never ground truth.

## Literature
Cascades / deleveraging spirals are well documented:
- Cheng, Deng, Wang & Yu, "Liquidation, Leverage and Optimal Margin in Bitcoin
  Futures" (arXiv:2102.04591): forced liquidations are SUBSTANTIAL (~3.5%/day of
  OI on BitMEX) and liquidated traders ran ~60x average leverage.
- Perpetual-futures fundamentals: He, Manela et al. (arXiv:2212.06888); Ackerer,
  Hugonnier & Jermann (arXiv:2310.11771) -- funding/leverage mechanics.
- Classic TradFi reference for the spiral: Brunnermeier & Pedersen, "Market
  Liquidity and Funding Liquidity" (margin spirals).
The cascade MECHANISM is real and large; the question is data + tradability.

## DATA REALITY -- the crux (we capture NONE of this today)
Confirmed available feeds:
- Binance `btcusdt@forceOrder` and `!forceOrder@arr` (USDS-M futures): realised
  forced-liquidation orders. IMPORTANT throttle: only the LARGEST liquidation per
  symbol per 1000ms is pushed -> the stream is an INCOMPLETE SAMPLE (undercounts
  volume; good as a cascade-PRESENCE signal, not exact size).
- Binance REST `/fapi/v1/forceOrders` (90-day history).
- Open Interest (REST/stream) -- needed to BUILD the estimated magnet heatmap.
- Hyperliquid liquidation data -- via API / third parties (purrdata, hyblock).
We currently capture only depth20/trade/bookDelta/bookSnapshot (Binance) + HL
quotes. No forceOrder, no OI.

## Verdict: PARK -- DATA-BLOCKED, with a cheap unlock path
- CASCADE-FADE variant = PURSUE-AFTER-CAPTURE (the cheaper, higher-priority one).
  It largely reuses `liquidity-sweep-primitive.ts`, which ALREADY models
  sweep -> exhaust -> reversal by INFERRING the burst from aggressor flow. Feeding
  it REAL forceOrder prints makes the trigger precise instead of inferred.
- MAGNET variant = PARK (bigger build): requires OI capture + a heatmap
  estimation engine, and it is an estimate of an estimate. Lower priority.

## Prerequisite capture task (additive, low-risk, respects hard rules)
Add `btcusdt@forceOrder` to the RESEARCH-PLANE capture sidecar, mirroring the
existing bookDelta capture pattern (additive instrumentation; NO live bot
consumes it). Optionally add periodic Open-Interest snapshots for the magnet
variant later. New Parquet streams: `liquidation` (+ `openInterest`). This is the
same kind of additive capture that trade/bookDelta capture already are, so it is
low risk -- but it IS a build, hence the PARK until it ships.

## Experiment specs (post-capture)
- CASCADE-FADE bot: trigger = forceOrder burst (>= N notional in W ms) coincident
  with a one-sided aggressor spike and a price extension; ENTER the reversal once
  the burst decelerates (reuse the sweep primitive's exhaustion logic). Grid:
  burst notional threshold, window W, deceleration window, hold {15,30,60,120}s,
  TP/SL. Success bar: 11 bps net OOS; favourable that cascades are LARGE moves
  (fees become noise) but model SLIPPAGE carefully (cascades widen spreads).
- MAGNET bot (later): from the OI-estimated heatmap, enter TOWARD the nearest
  large un-swept cluster; exit at the cluster. Grid: leverage tiers included,
  cluster-size threshold, max distance, hold. Success bar: same, plus must beat a
  naive "trade toward nearest round number" baseline (clusters concentrate at
  obvious stops anyway).

## Caveats to carry into any build
- forceOrder is throttled (largest-per-second) -> realised liquidation volume is
  UNDERCOUNTED. Use it as cascade-presence, not exact magnitude.
- The magnet heatmap is reverse-engineered; bands are a prior, not truth.
- Cascades are violent: the favourable side is that moves dwarf 11 bps; the
  unfavourable side is severe slippage/spread-widening at exactly the moment you
  trade. The realistic fill engine must model that or results will be optimistic.

## Suggested tasks
- L0 (prereq): add forceOrder (+ optional OI) capture to the research-plane
  sidecar. Additive; ships behind nothing live.
- L1: cascade-fade bot on captured forceOrder, reusing the sweep primitive.
- L2 (later): OI heatmap estimation engine + magnet bot.