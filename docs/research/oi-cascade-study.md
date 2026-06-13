# OI-Drop Forced-Flow Cascade Study (oiscope)

Tool: `crates/forgelag/src/bin/oiscope.rs` (analysis only, NOT a strategy).
Branch: `forgelag`. Sacred core untouched. Build + clippy(-D warnings) + tests green
(7 new oiscope unit tests; full forgelag suite stays green).

## The question
Lagshot died because the basis reversion out-ran our ~0.8-2.4s execution latency.
A liquidation cascade is different: it is a move that has ALREADY PRINTED, so we
REACT to it rather than race a micro-reversion. So latency *should* bind far less.

THESIS tested: a sharp OI DROP (positions force-closed) + a simultaneous microprice
SPIKE + net one-sided aggressive HL trade flow in the move direction = a cascade.
Forced SELLING (longs liq) -> price down + OI down -> cascade DOWN; forced BUYING
(shorts liq) -> cascade UP. We detect both and ask: does the post-cascade reversion
exist, is it big enough, and is it SLOW enough to capture at 800ms-2000ms entry delay?

## How oiscope works (exact rules)

PRICE = HL size-weighted top-5 microprice (what we would actually trade against), NOT
mark. Deterministic, no-lookahead: book / OI / flow folded forward event by event,
only `<= now` state read. HL-internal study (HL book + HL trades + HL OI); no OKX.

DETECTION (rolling window W, default 5s):
  - OI-drop%  = (oi[t-W] - oi[t]) / oi[t-W] * 100      must be >= D   (--oi-drop)
  - price-move = (micro[t] - micro[t-W]) / micro[t-W] * 1e4 (bps), |.| >= P (--price-move)
  - net signed HL aggressive flow over the window (buy +, sell -) must be one-sided
    in the move direction (--min-flow magnitude, default 0 = sign only).
  - DOWN cascade: price-move <= -P AND net flow <= -min-flow AND OI-drop >= D.
  - UP   cascade: price-move >= +P AND net flow >= +min-flow AND OI-drop >= D.
  - window-start values are read from a 50ms-cadence buffer; a fire requires the buffer
    to span >= W/2 (no startup artefacts).
  - DE-DUP: after a fire, a --cooldown (default 30s) blocks new fires (one cascade =
    one event).

CHARACTERISATION (forward horizon --forward, default 60s; baseline = pre-cascade
microprice = micro at window start):
  - SPIKE bps   = peak excursion from baseline in the cascade direction.
  - REVERSION   = max recovery back from that peak toward/through baseline (bps).
  - revert_frac = reversion / spike. "REVERTED" if revert_frac >= --revert-frac (0.5);
    else "CONTINUE / trend".
  - time-to-HALF-revert / time-to-FULL-revert (back to baseline).

TRADEABILITY (the key measure) - a SIMPLE reactive FADE:
  - enter at a configurable DELAY after detection (--delays 0,800ms,2000ms = our
    latency band). Down cascade -> BUY (expect bounce); up cascade -> SELL.
  - hold until +R bps favourable reversion (--fade-revert 10) OR --fade-hold (30s),
    then mark to market. Report bps CAPTURED (signed) at each delay, mean + median +
    t-stat + win% across cascades. Entering at a DELAY is the whole point: if the edge
    survives an 800-2000ms-late entry, latency is not the killer.

SWEEP: --oi-drop and --price-move take comma lists; the tool runs the full grid so we
can see cascade count vs quality (knob-bite). Optional --dump writes a per-cascade CSV.

Run: 10 study days (2025-11-04 .. 2025-12-15), ETH then BTC, W=5s, grid
oi-drop {0.2,0.5,1.0}% x price-move {5,10,20}bps, cooldown 30s, forward 60s,
fade delays 0/800ms/2000ms, fade target 10bps / hold 30s.
## Results (pooled over 10 days, W=5s)

KNOB-BITE (valid): cascade count moves monotonically with the dials, spike size scales
up with the thresholds. The detection set genuinely responds to the knobs.

### ETH  (fade bps captured; t-stat in parentheses)

| oi-drop% / move bps | casc/day | spike mean | revert% | fade@0ms | fade@800ms | fade@2s | n |
|---|---|---|---|---|---|---|---|
| 0.2 / 5  | 33.5 | 23.3 | 61% | -1.61 (t-2.08) | -1.28 (t-1.67) | -1.25 (t-1.64) | 335 |
| 0.2 / 10 | 17.4 | 31.3 | 60% | -3.18 (t-2.43) | -2.39 (t-1.86) | -2.70 (t-2.11) | 174 |
| 0.2 / 20 | 3.5  | 51.5 | 60% | -6.57 (t-1.52) | -5.77 (t-1.32) | -5.25 (t-1.26) | 35  |
| 0.5 / 5  | 10.4 | 23.7 | 60% | -2.38 (t-1.75) | -1.27 (t-1.01) | -0.72 (t-0.61) | 104 |
| 0.5 / 10 | 6.3  | 31.3 | 54% | -4.37 (t-2.16) | -3.56 (t-1.89) | -2.84 (t-1.64) | 63  |
| 0.5 / 20 | 1.2  | 56.4 | 42% | -13.91 (t-1.95)| -6.25 (t-1.05) | -4.86 (t-0.94) | 12  |
| 1.0 / 5  | 3.6  | 24.1 | 44% | -0.99 (t-0.54) | +0.18 (t+0.10) | +0.01 (t+0.00) | 36  |
| 1.0 / 10 | 2.8  | 28.2 | 39% | -1.76 (t-0.89) | -1.04 (t-0.51) | -0.70 (t-0.34) | 28  |
| 1.0 / 20 | 0.7  | 47.0 | 57% | -0.84 (t-0.15) | +1.44 (t+0.23) | +0.42 (t+0.07) | 7   |

ETH reversion timing: time-to-HALF-revert ~20s, time-to-FULL-revert ~24s (SLOW).

### BTC  (fade bps captured; t-stat in parentheses)

| oi-drop% / move bps | casc/day | spike mean | revert% | fade@0ms | fade@800ms | fade@2s | n |
|---|---|---|---|---|---|---|---|
| 0.2 / 5  | 23.5 | 16.3 | 59% | +0.05 (t+0.10) | +0.57 (t+1.04) | +1.03 (t+1.91) | 235 |
| 0.2 / 10 | 7.5  | 25.0 | 60% | +0.75 (t+0.70) | +1.51 (t+1.47) | +2.07 (t+2.01) | 75  |
| 0.2 / 20 | 0.9  | 39.7 | 56% | +5.41 (t+1.40) | +4.86 (t+1.21) | +5.36 (t+1.30) | 9   |
| 0.5 / 5  | 6.6  | 18.7 | 48% | -0.28 (t-0.27) | +0.23 (t+0.23) | +0.58 (t+0.59) | 66  |
| 0.5 / 10 | 3.0  | 23.4 | 57% | +2.29 (t+1.39) | +2.84 (t+1.87) | +3.01 (t+2.12) | 30  |
| 0.5 / 20 | 0.3  | 40.4 | 67% | +11.59 (n=3)   | +11.47 (n=3)   | +10.64 (n=3)   | 3   |
| 1.0 / 5  | 1.3  | 17.4 | 46% | -1.06 (t-0.58) | -0.39 (t-0.24) | +0.98 (t+0.59) | 13  |
| 1.0 / 10 | 0.8  | 20.5 | 50% | +2.04 (t+0.64) | +0.07 (t+0.03) | +1.66 (t+0.64) | 8   |
| 1.0 / 20 | 0.0  | -    | -   | -              | -              | -              | 0   |

BTC reversion timing: time-to-HALF-revert ~15-23s, time-to-FULL-revert ~22-35s (SLOW).

## Honest interpretation

1) DO cascades exist + how often? YES. With OI-drop + price-spike + one-sided flow
   firing together, ETH ~6-33/day and BTC ~3-23/day at the looser-to-mid thresholds;
   1-4/day at the strict end. Real, frequent, knob-responsive.

2) DO they revert or trend? MIXED. ~50-60% revert by >= half the spike; ~40-50%
   CONTINUE (trend). Only ~30-40% fully return to the pre-cascade level. So a cascade
   is NOT a reliable snap-back - a large minority keep going.

3) Is the reversion SLOW enough for our latency? YES - and this is the one part of the
   thesis that is VINDICATED. time-to-half ~15-23s, time-to-full ~22-35s. Crucially the
   fade captured at 2000ms is ~= (often slightly BETTER than) at 0ms: entering 0.8-2.0s
   late costs us essentially nothing. Latency is NOT the binding constraint here, unlike
   Lagshot. (Entering at 0ms is actually worst, because the spike is still extending at
   the instant of detection.) The central bet - "react, do not race" - was correct.

4) Is the reactive fade TRADEABLE? NO - it fails on edge size, not on latency.
   - ETH: NEGATIVE mean capture at every meaningful cell (-1 to -14bps), t-stats
     negative or near-zero, never significantly positive. The ~40% of cascades that
     TREND produce large losses (-30 to -70bps on the worst days) that swamp the small
     reversion wins. (Note the median is mildly positive while the mean is negative =
     a fat LEFT tail of trending cascades.) ETH - the Lagshot champion - is the WORSE
     asset here: bigger spikes that overshoot/trend harder.
   - BTC: a faint POSITIVE pulse - +2-3bps gross mean with t~2.0 at oi-drop0.2/move10
     (n=75) and oi-drop0.5/move10 (n=30), and the 2s-delay entry is consistently best
     (latency-robust). But the magnitude is tiny.
   - THE FEE WALL: this is a TAKER fade (cross the spread on entry and exit). HL taker
     round-trip is ~9bps (4.5bps/side) plus spread/slippage. The best gross capture
     anywhere is BTC ~+3bps; ETH is negative. NOTHING clears the ~9bps hurdle. Net of
     fees the simple reactive fade is negative on both assets.

## Verdict

NO tradeable pulse for a naive reactive fade - do NOT open a strategy spec on this
result. The thesis's key hope (latency does not bind, because we react to a printed
move) is CONFIRMED and is genuinely different from the Lagshot trap. But the captured
edge is negative-to-tiny and far below the ~9bps taker fee floor, and ~40-50% of
cascades trend rather than revert, producing a fat left tail that kills mean expectancy
(worst on ETH). A cascade overshoots and EITHER snaps back slowly (small win) OR keeps
going (large loss); a symmetric fade does not separate the two in advance.

This kills the naive version cheap (0 euros risked), exactly as intended.

### The only cheap follow-ups worth a look (not a commitment)
- Asymmetry: median > mean means the damage is the trending tail. A hard STOP / a
  "abort the fade if the move is still extending against us at entry+X" filter could
  cut the left tail. (Caveat: Lagshot already found stops hurt mean-reversion; treat
  with skepticism.) Even if a stop salvaged ETH, the +2-3bps BTC gross still must beat
  9bps fees - so a fade is likely structurally sub-fee regardless.
- A MAKER cascade fade (rest a limit at the pre-cascade level, earn the rebate instead
  of paying 9bps) would change the fee math - but gapscope already showed post-event HL
  is a re-quote VACUUM with walls PULLED, so adverse selection is the likely killer
  there too.

On current evidence: shelve the OI-cascade fade. Detection works and is honest; the
edge is not there at our cost structure. CSVs: /root/runs/oiscope/{eth,btc}_cascades.csv.