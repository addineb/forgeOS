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