# ENGINE-GRADE VERDICT (2026-06-11): REAL but UNPROVEN. Built in the lag-subspace.

The basis-reversion strategy now runs inside the engine (branch lag-subspace) with
realistic HL fills (taker, walks the book = spread+slippage), order latency, and
adverse selection. 6 days x 8h windows (Dec..Jun). Result:

## What it shows (clean latency ladder, fill_timeout fixed >= latency)
Best configs (thr in bps on |dev|, reversion, depth-5 microprice vs Binance trades):
- net-POSITIVE and degrades SMOOTHLY with order latency (no cliff):
  thr=12/hold=3m: ~9.4bps/trade @0ms -> ~8 @300ms -> ~3.2 @700ms -> ~0 @1s (33 trips).
  thr=8/hold=2m:  ~5.4bps/trade @0ms -> ~3.4 @300ms -> ~0.6 @700ms (117 trips).
- Direction is REAL: the shuffled-direction control LOSES (PBO 0.84, net negative).
- P&L concentrates in SIDEWAYS regime at every latency (reversion works in chop).
- PAPER GATE PASSES: EUR500/20x/10% size @300ms latency -> +5.7% (thr12) / +7.8%
  (thr8) over the 6 days, maxDD 1.4-4%, never ruined.

## What it does NOT show (the honest brake)
- PROMOTION GATE FAILS at every latency: DSR ~= 0.000, 0 promote / 0 park / all
  retire. On 6 days and 33-117 trades, deflated-Sharpe says it is NOT statistically
  distinguishable from luck. Paper-passing does NOT override this (project rule).
- So: REAL signal, survives realistic latency, profitable on these days - but too
  THIN / INFREQUENT to trust live on current data.

## Why this is the OPPOSITE of pure lead-lag (and why it survived)
Pure cross-venue lead-lag dies in a <700ms latency race (lag-avenues-study). Basis
reversion holds for MINUTES, so 300-700ms execution latency only shaves the edge
(9->3bps) rather than killing it. That is exactly why this lead is worth more data.

## NEXT (decided): get statistical power
The whole pipeline now exists (converter hlbook stage -> *.forge -> basis sweep ->
paper). Re-run over MANY MORE DAYS (the 17 full days on the box, + pull more) so
DSR/PBO have power to PROMOTE or RETIRE honestly. Also test a sideways-only regime
gate (the edge lives there). Engine core stayed UNTOUCHED (work is data + strategy
+ sweep layers); the proven main engine is intact.

---
# UPDATE (2026-06-11): pulse SURVIVES spread haircut. Now blocked only on engine-grade fills.

## Spread-aware multi-day result (6 days/7 months, pay real HL bid/ask + 9bps fee)
HL BTC spread is tiny (~0.13bps = 1 tick on $78k). thr>8bps, ~2min hold:
- REAL fills (enter at ask/bid, exit opposite) + 9bps fee: NET +8.55bps/trade,
  ~9 trades/day, POSITIVE ALL 6 DAYS (+14.1,+15.5,+4.7,+12.6,+9.3,+3.2 bps).
- Spread cost only ~0.35bps total (negligible - liquid, coarse $1 tick).
- Funding negligible at 2min holds (1/240 of an 8h period).

## What is PROVEN vs NOT (honest)
PROVEN (python, dense correct data): a real, knob-biting, out-of-sample-consistent
basis-reversion pulse at the >8bps stretch; survives spread + taker fees.
NOT YET MODELLED (python cannot - this is the engine's job):
1. LATENCY / adverse selection: study fills at the SAME snapshot dev crosses thr.
   Real orders land ~hundreds of ms later (HL ~700ms structural lag per Arrakis) -
   price moves against us in that gap. THE main risk. Engine two-clock models this.
2. QUEUE / market impact / top-of-book depth for ~0.13 BTC (EUR500x20x) size.
3. Statistical: 52 trades at thr>8 -> need more days for DSR/PBO.

## Next step (NEEDS SIGN-OFF - touches the sacred engine)
Build basis-reversion in a "lag subspace" = a BRANCH/COPY of the engine, with
realistic HL latency + bid/ask fills + adverse selection, then run the full gates
(null-edge, knob-bite, DSR/PBO, EUR500 paper). Only if it survives engine-grade
fills is it trusted. Per project rule, engine work waits for explicit sign-off.

---
# !!! CORRECTION (2026-06-11, later same day): the KILL below was WRONG - my bug.

The "KILLED on dense data" verdict directly below was produced by a TIMESTAMP BUG
in my own study: Binance trade `ts` is already in MILLISECONDS, but my code divided
it by 1,000,000 (treating it as nanoseconds). That collapsed the Binance leg to a
single frozen price, so the study accidentally tested "HL reverts to its OWN rolling
mean", NOT spot-perp basis. Same class of self-inflicted error the project exists to
catch - caught it while building the liquidation study (same bug there).

## Corrected single-day result (2026-02-01, 8h, CORRECT alignment)
Reversion, depth-5 HL microprice vs time-aligned Binance trade price, dev = gap -
rolling-500 mean, fixed-horizon exit:
- thr>8bps H=180s: ~39 trades/day, gross +15.2bps, NET +4.2bps after 11bps, win 92%.
- thr>8bps H=360s: net +2.3bps, win 82%.   thr>8 H=60s: net -0.9bps, win 93%.
- Momentum side = clean MIRROR (strongly negative) -> real directional content.
- BUT thin: only 11-14 trades at thr>8 (one day); thr>5 (larger N) is net-negative;
  fills idealized (HL microprice, no spread/slippage); funding still ignored.

## Status: RE-OPENED. Pulse survives correct alignment but unproven.
Multi-day out-of-sample validation (8 days across 7 months) RUNNING. Verdict pending.
The lag-subspace engine plan is back on the table ONLY if the pulse holds across days
with adequate sample - and that build touches the engine -> needs explicit sign-off.

---
# Spot-Perp Basis Reversion (was "lead-lag") - KILLED on dense data (2026-06-11)

## VERDICT: NO EDGE. The pulse was a DATA ARTIFACT, not a real edge.
The earlier "first pulse" (~23-30bps gross, 80-93% win, ~40 trades/day) was
produced ENTIRELY by the thin, BBO-collapsed `hlquote` feed (the 5-10s sparse
updates). When the study is re-run on the REAL HL full-depth L2 - a fresh 20-level
snapshot every ~0.55s (52,492 microprice points over 8h on 2026-02-01, 1.82/sec) -
the edge VANISHES and goes gross-NEGATIVE.

### Dense re-study results (tools/basis_restudy.py + basis_restudy_robust.py)
Depth-weighted top-5 HL microprice vs Binance BTCUSDT, dev = gap - rolling-500 mean.
- Hold-to-sign-flip exit: gross -2.6 to -5.3 bps at EVERY threshold (5-25bps),
  win 42-59%. Net @11bps = -13 to -16 bps. The trade loses BEFORE costs.
- Fixed-horizon sweep (thr 5/8/12/18, H 60/180/360 snaps), BOTH directions:
  - Reversion: all net-negative; best net -6.56bps (40 trades, noise).
  - Momentum: also all net-negative; best gross +2.78bps - pure noise vs 11bps wall.
  - Win rates 45-59% across the board = coinflip.
- median|dev| ~4.8bps, p90 ~17.6bps: the real local stretch is far below the
  11bps round-trip cost. There is no room to pay for the trade.

### Why the old number lied (same class of bug as the first TS engine)
The collapsed BBO `hlquote` only printed on BBO change, so the "microprice" jumped
in coarse 5-10s steps. A trade entered at one stale print and "reverted" to the
next stale print looked like a clean 20-30bps round-trip - but that move was the
quantization of the feed, not an executable price path. Dense data removes the
illusion. The data sparsity WAS the fake edge.

### Consequence for the plan
The lag-subspace ENGINE CLONE is CANCELLED. It was justified only by a trusted
pulse; there is no pulse. We do NOT touch the sacred engine for this. No churn:
we tested the lead properly (1 day, 8h, dense, both directions, multiple exits)
and it is dead. Could still re-test other venue pairs/days for completeness, but
the headline reason to build is gone.

---
# (HISTORICAL - the pre-study that turned out to be an artifact)
# Spot-Perp Basis Reversion (was "lead-lag") - FIRST PULSE, NOT yet trusted

CORRECTION: earlier "blocked - HL too sparse" was wrong (judged from one dead-hour
file). Full days show ~1600-2800 HL quotes/day (median ~1s, bursty). Reframe: this
is NOT sub-second lead-lag - it is SPOT(Binance) vs PERP(Hyperliquid) BASIS
mean-reversion. No speed race; ~20-min holds, so sparse HL quotes are fine.

## What the data shows (python pre-study, tools/lag_study*.py)
Signal: gap = (HL_mid - Binance)/Binance in bps; dev = gap - rolling-200 mean.
Bet reversion of dev (HL rich -> short HL), hold until dev sign flips.
- 3 days (2026-02-01, 2025-12-01, 2026-05-01): ~14-42 trades/day, gross ~23-30
  bps/trade, win 81-93%, avg hold ~20-40 min.
- Net after taker (~11 bps r/t): +12 to +19 bps/trade. Net at maker (~2): +21-28.
- Naive "exit next quote" version: wins 90% but only ~1 bp gross -> loses to costs.
  The edge (if real) needs the FULL reversion hold, not one step.

## DO NOT TRUST YET - skeptic flags (must clear before believing)
1. Idealized fills: traded at HL mid at the quote instant; no spread/slippage, and
   HL quotes are sparse - may not be executable when wanted.
2. Tiny sample: ~100 trades / 3 days. 80-93% win on that N is not significance.
3. Hand-picked knobs: 20 bps trigger, 60-step hold - not swept, no control.
4. Funding ignored: holding a perp ~20 min has a funding cost/credit.
5. ~500-1000 bps/day is implausibly high -> assume artifact until proven.

## Honest next step (build it properly, run the gates)
- Strategy in forge-strategy: trigger on |dev|>thr, hold to reversion. Execute
  ONLY at real HL quotes; pay HL spread + fee + funding. Engine extension is
  modest because holds are minutes (no latency race) - HL quote stream as the
  executable points.
- Gates: shuffled-direction control, sweep thr/hold across ALL 15 windows,
  DSR/PBO, then EUR500 paper gate. Trust the verdict, not the python pre-study.
- This is a TYPE-C (forced-flow/basis) edge; pairs with funding data if we add it.

## Status
PROMISING (first pulse). Pre-study only. Not validated. High priority to build
honestly because nothing else has shown a pulse.
---
# POST-MORTEM (2026-06-11): two wrong explanations in one morning - logged so we never repeat
1. WRONG verdict: "basis-reversion is dead / no edge." Cause: a divide-by-1e6 bug in
   basis_restudy.py - Binance trade_time is MILLISECONDS but I divided it as if it
   were nanoseconds, freezing the Binance leg to one price (tested HL-vs-its-own-mean).
   CHD scales differ: trade_time/event_time = ms, received_time = ns. Confused them.
2. WRONG explanation: "the earlier pulse was an artifact of thin hlquote sparsity."
   FALSE - the original lag_study*.py used correct, consistent ms timestamps and DID
   show the pulse. The pulse was never the artifact; my divide bug was.

BLAST RADIUS: only the basis/lag/cascade PYTHON pre-studies. The orderflow hunts
(OFI/wall/CVD/wallflow/absorption, ~15k cfgs, 0 promote) run via the RUST engine fed
by chd-to-parquet.py, which scales timestamps CORRECTLY (trade_time/event_time as-is
ms; received_time //1e6). Engine also passes null-edge + det-hash -> data path proven
clean. So those verdicts stand; only the morning basis "kill" was corrupted.

NOT 100% CERTAIN the fix = a tradeable edge: the corrected study proves the SIGNAL
exists (out-of-sample positive, knob-bite, momentum-mirror, spread-survives) but NOT
that it is CAPTURABLE under real latency / adverse selection / impact. That is the
engine build + gates job. Trust nothing until it survives engine-grade fills + paper.