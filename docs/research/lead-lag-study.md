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