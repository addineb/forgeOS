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
---

## Tweak 1: exhaustion-conditioned entry (isolated)

Tested ALONE (no magnitude filter, no gapscope confirm - those come later). Default
OFF so baseline behavior is byte-preserved. Build + clippy(-D warnings) + tests all
green on the box; oiscope unit tests 7 -> 9 (2 new exhaustion-trigger tests), full
forgelag suite 95 -> 97 green.

### The exact rule + flags
Thesis: a cascade that REVERTS is one where the forced flow RUNS OUT - the OI-drop
decelerates AND price stops extending (stalls) - then it snaps back; a TRENDER keeps
bleeding OI / pushing price. Since latency is slack here, we can afford to WAIT for
that exhaustion before fading.

New flags (all default OFF / baseline-preserving):
  --exhaust                enable exhaustion-conditioned entry.
  --exhaust-stall <dur>    stall window / trailing lookback W_s (default 2s).
  --exhaust-decel <frac>   OI-bleed deceleration fraction f (default 0.5).

EXHAUSTION fires at the FIRST forward sample t (with >= W_s elapsed since the fire)
where BOTH hold:
  (a) OI STOPS BLEEDING: trailing OI-drop% over [t-W_s, t] <= f x the PEAK trailing
      OI-drop% seen since the fire. (If OI never bled forward, this leg is treated as
      already satisfied.)
  (b) PRICE STALLS: price has made NO new extreme in the cascade direction within the
      trailing W_s window.
Entry = fade at the exhaustion point, then the SAME delays (0/800/2000ms) are applied
AFTER it (so we keep the latency-realism A/B), with the SAME exit (10bps revert target
/ 30s max hold). If exhaustion never fires within the 60s horizon -> NO trade (skip).
Fees: this is a TAKER fade (cross on entry + exit). HL taker round-trip ~9bps. Below,
"gross" = captured bps; NET = gross - 9bps. Baseline grid matched exactly:
window 5s, oi-drop {0.2,0.5,1.0}% x move {5,10,20}bps, cooldown 30s, forward 60s,
delays 0/800ms/2000ms, fade target 10bps / hold 30s, 10 study days, ETH then BTC.

### KEY structural finding (read this first)
EXHAUSTION FIRED ON 100% OF CASCADES, SKIPPED 0, on EVERY cell of both assets
(median wait ~4-5s after the fire). Over a 60s horizon essentially every cascade
eventually stalls at least once - so the "skip the trenders that never exhaust"
mechanism that was the whole point almost NEVER triggers. In practice the tweak is
NOT a selectivity filter; it is an ENTRY-TIMING shift: fade ~4-5s after the fire at
the first stall, instead of at 0-2s. The trade COUNT is unchanged (n identical to
baseline in every cell). So a paused trender (a trend that takes a breather, trips
the stall, then resumes) still gets faded - which is why the fattest left-tail trades
survive (see worst column below).

### A/B - ETH (gross fade bps; t in parens). worst / %<-30bps at delay 0ms.
| oi/move | n | base @0ms | exh @0ms | base @2s | exh @2s | base worst /<-30 | exh worst /<-30 |
|---|---|---|---|---|---|---|---|
| 0.2/5  | 335 | -1.61 (t-2.08) | -0.26 (t-0.38) | -1.25 (t-1.64) | -0.74 (t-1.06) | -78.2 / 4% | -77.3 / 2% |
| 0.2/10 | 174 | -3.18 (t-2.43) | -1.13 (t-1.00) | -2.70 (t-2.11) | -1.70 (t-1.49) | -78.2 / 7% | -77.3 / 4% |
| 0.2/20 | 35  | -6.57 (t-1.52) | -0.55 (t-0.17) | -5.25 (t-1.26) | -2.72 (t-0.76) | -73.8 / 20% | -77.3 / 6% |
| 0.5/5  | 104 | -2.38 (t-1.75) | +0.22 (t+0.20) | -0.72 (t-0.61) | +0.02 (t+0.02) | -71.3 / 6% | -43.1 / 1% |
| 0.5/10 | 63  | -4.37 (t-2.16) | -1.82 (t-1.08) | -2.84 (t-1.64) | -1.94 (t-1.01) | -71.3 / 8% | -43.1 / 3% |
| 0.5/20 | 12  | -13.91 (t-1.95)| -2.40 (t-0.52) | -4.86 (t-0.94) | -8.49 (t-1.20) | -71.3 / 25% | -29.7 / 0% |
| 1.0/5  | 36  | -0.99 (t-0.54) | +1.04 (t+0.56) | +0.01 (t+0.00) | +0.22 (t+0.11) | -28.2 / 0% | -34.9 / 3% |
| 1.0/10 | 28  | -1.76 (t-0.89) | -0.39 (t-0.17) | -0.70 (t-0.34) | -1.11 (t-0.46) | -28.2 / 0% | -34.9 / 4% |
| 1.0/20 | 7   | -0.84 (t-0.15) | +0.96 (t+0.16) | +0.42 (t+0.07) | -3.90 (t-0.47) | -23.9 / 0% | -29.7 / 0% |

ETH read: exhaustion LIFTS the mean by ~+1 to +2bps and roughly HALVES the mid
left-tail (e.g. <-30bps 4%->2%, 7%->4%, 20%->6%), turning the worst-losing cells from
clearly-negative to near-zero. BUT the single WORST trade barely moves (still ~-77bps)
- the fattest tail = paused trenders that stall then resume, which the timing-shift
cannot dodge. No ETH cell is gross-positive of note; net of 9bps every ETH cell is
still negative.

### A/B - BTC (gross fade bps; t in parens). worst / %<-30bps at delay 0ms.
| oi/move | n | base @0ms | exh @0ms | base @2s | exh @2s | base worst /<-30 | exh worst /<-30 |
|---|---|---|---|---|---|---|---|
| 0.2/5  | 235 | +0.05 (t+0.10) | +1.13 (t+2.13) | +1.03 (t+1.91) | +0.84 (t+1.60) | -27.0 / 0% | -23.4 / 0% |
| 0.2/10 | 75  | +0.75 (t+0.70) | +1.86 (t+1.74) | +2.07 (t+2.01) | +0.93 (t+0.89) | -27.0 / 0% | -23.4 / 0% |
| 0.2/20 | 9   | +5.41 (t+1.40) | +5.42 (t+1.42) | +5.36 (t+1.30) | +5.47 (t+1.52) | -19.7 / 0% | -23.4 / 0% |
| 0.5/5  | 66  | -0.28 (t-0.27) | +0.44 (t+0.44) | +0.58 (t+0.59) | +0.68 (t+0.67) | -17.4 / 0% | -20.5 / 0% |
| 0.5/10 | 30  | +2.29 (t+1.39) | +3.43 (t+2.39) | +3.01 (t+2.12) | +3.75 (t+2.58) | -17.4 / 0% | -12.7 / 0% |
| 0.5/20 | 3   | +11.59 (n=3)   | +10.54 (n=3)   | +10.64 (n=3)   | +11.33 (n=3)   | +10.8 / 0% | +10.2 / 0% |
| 1.0/5  | 13  | -1.06 (t-0.58) | +1.44 (t+0.80) | +0.98 (t+0.59) | +1.19 (t+0.56) | -11.1 / 0% | -7.9 / 0% |
| 1.0/10 | 8   | +2.04 (t+0.64) | +2.76 (t+1.00) | +1.66 (t+0.64) | +1.85 (t+0.58) | -8.4 / 0% | -7.9 / 0% |

BTC read: BTC had almost no left tail to cut (spikes are smaller; baseline <-30bps was
already ~0%). Exhaustion lifts the mean a touch and firms the t on the better cells -
the standout is oi0.5/move10 (n=30): +3.01bps t2.12 -> +3.75bps t2.58 at 2s; and it
makes the 0ms entry useful (oi0.2/move5 +0.05 t0.10 -> +1.13 t2.13). But the BEST
gross capture anywhere is still only ~+3.75bps (n=30) - WELL under the ~9bps taker
round-trip. Net of 9bps every BTC cell is negative.

### Knob-bite (ETH, single cell oi0.2/move10, n=174 fixed)
| dial | median wait | fade @0ms | fade @2s |
|---|---|---|---|
| decel 0.3 | 4.1s | -1.06 (t-0.94) | -1.60 (t-1.40) |
| decel 0.5 | 4.1s | -1.13 (t-1.00) | -1.70 (t-1.49) |
| decel 0.7 | 4.0s | -1.18 (t-1.05) | -1.68 (t-1.48) |
| stall 1s  | 2.5s | -1.27 (t-1.02) | -2.25 (t-1.90) |
| stall 2s  | 4.1s | -1.13 (t-1.00) | -1.70 (t-1.49) |
| stall 3s  | 5.9s | -1.26 (t-1.12) | -1.25 (t-1.06) |

Honest knob-bite: the STALL window is the binding dial - it moves the median wait
monotonically (2.5s -> 4.1s -> 5.9s) and shifts the captured bps with it. The DECEL
fraction is nearly INERT (0.3/0.5/0.7 all ~the same wait and result), because OI bleed
decelerates fast and is almost always already below 0.5x its peak by the time price
stalls - so leg (b) price-stall is what actually binds, not leg (a) OI-decel. The dials
move the ENTRY TIMING / captured bps but NOT the trade count (always 174) - because the
gate fires 100% of the time. So the dial "moves trades" in the timing sense, not the
selectivity sense.

### Verdict on Tweak 1 (isolated): NO - it does not make the fade tradeable.
- Did it CUT THE LEFT TAIL? Partially. On ETH it roughly halves the mid tail
  (<-30bps fraction) and lifts the mean ~+1 to +2bps, flipping the worst-losing cells
  to near-zero. But the SINGLE worst trades (~-77bps) survive untouched - those are
  paused trenders, and a timing shift cannot avoid them.
- Did it LIFT mean / t ABOVE THE FEE FLOOR? No. Best gross is BTC ~+3.75bps (oi0.5/
  move10, t2.58, n=30); ETH stays negative-to-flat. The ~9bps taker round-trip is
  never cleared. NET of 9bps every cell on both assets is negative.
- Why it underperforms the thesis: the gate fires on 100% of cascades (skips 0) over
  the 60s horizon, so it never actually SEPARATES reverters from trenders - it only
  delays entry to the first stall (~4-5s). It is an entry-timing tweak, not the
  selectivity filter the thesis wanted.

CONCLUSION: exhaustion-conditioning ALONE trims the ETH mid-tail and nudges BTC
mean/t modestly, but it leaves the fade structurally sub-fee on both assets. It trades
the same set, just later, without fixing the fat tail. Not tradeable on its own. (The
modest tail-trim + the fact that the stall window is the live dial are worth carrying
into the later combined tests with the magnitude filter / gapscope confirm.) Logs:
/root/runs/oiscope_exh/{eth,btc}_{base,exh}.log + eth_cell_*.log.
---

## Tweak 2: magnitude filter (isolated)

Tested ALONE (exhaustion OFF, no gapscope-confirm - those are separate). Default
OFF so baseline behavior is byte-preserved (verified: with the filter off the
pooled numbers reproduce the baseline study EXACTLY, e.g. ETH oi0.2/move5 n=335,
fade@0ms -1.61 t-2.08, worst -78.2, <-30bps 4%). Build + clippy(-D warnings,
all-targets) + tests all green on the box; oiscope unit tests 9 -> 11 (2 new
magnitude-gate tests incl a NO-LOOKAHEAD assertion), full forgelag suite 97 -> 99.

### The exact rule + flags (read the no-lookahead note carefully)
Thesis: only the BIGGEST cascades overshoot far enough that the reversion clears
the ~9bps taker fee floor; small cascades give tiny reverts that never pay for
themselves and just add noise. So FILTER to fade only large cascades.

New flags (default OFF / baseline-preserving):
  --min-spike <bps>    only fade a cascade whose GATE-SPIKE >= this (primary dial).
  --min-oidrop <pct>   also require the realized OI-drop% >= this (secondary gate).
A cascade below either threshold is still DETECTED + CHARACTERISED but produces NO
trade (all fade entries = None / skipped). Everything else (detection grid, fade
direction, delays 0/800/2000ms, exit = 10bps revert target / 30s hold, fee
accounting) is EXACTLY baseline.

*** NO-LOOKAHEAD GATE-SPIKE DEFINITION (this is the honest crux) ***
The gate spike is the price move REALIZED from the cascade window-start to the
FIRE/detection moment = |price_move_bps| (window-start microprice -> fire
microprice). This is already printed at the instant we decide to enter - it uses
ONLY information at/before entry. It is NOT the forward peak excursion
(`Reversion::spike_bps`), which looks across the full 60s forward horizon and
would be LOOKAHEAD. The unit test `magnitude_gate_uses_realized_move_not_forward_
peak` constructs a cascade with a small (12bps) realized move but a large (60bps)
FORWARD peak and asserts the gate FILTERS it (uses the realized 12, ignores the
future 60). The gate is delay-independent (judged at the fire), so the passing
SET is identical across the 0/800/2000ms entries - one clean n per cell.

CONSEQUENCE (important for reading the sweep): the realized window move is much
SMALLER than the forward-peak "spike" reported in the baseline tables. In the
ETH oi0.2/move5 cell the forward-peak spike mean is ~23bps but the gate-spike
(realized move) distribution is min5 / median7 / p90 16 / max47 bps. So a
--min-spike of 20/30/50/80 (forward-peak scale) nearly EMPTIES the set
(>=30bps keeps 3/335 on ETH, >=80 keeps 0). The honest filter therefore has to
be swept on the REALIZED-move scale (here 8..30bps) where the dial actually
bites. Reported below on that scale.

### Knob-bite (valid - confirmed in-tool AND by post-processing the dump)
The --min-spike dial moves the trade set monotonically. In-tool runs:
ETH oi0.2/move5: 335 (off) -> 3 (>=30bps) -> 0 (>=80bps).
BTC oi0.2/move5: 235 (off) -> 1 (>=30bps).
These in-tool "passed X/N" counts and per-delay means match the dump
post-processing exactly (e.g. ETH >=30 n=3 mean -26.32 @0ms; BTC >=30 n=1 +11.87)
- the gate and the sweep agree. Knob-bite confirmed.

Fees: TAKER fade, HL taker round-trip ~9bps. "gross" = captured bps; NET = gross
- 9. Below: gross at delays 0/800ms/2s + NET at 2s (consistently the best,
latency-robust delay) + left-tail (<-30bps share and worst trade, at 0ms).

### A/B + threshold sweep - ETH (gross fade bps; net = gross-9)
ETH base oi-drop>=0.2% / move>=5bps (the largest set, n=335 off):
| min-spike | nPass | gross@0 | gross@800 | gross@2s | NET@2s | <-30%@0 | worst@0 |
|---|---|---|---|---|---|---|---|
| 0 (off) | 335 | -1.61 (t-2.1) | -1.28 | -1.25 (t-1.6) | -10.25 | 4%  | -78.2 |
| 8       | 127 | -3.11 (t-1.9) | -3.22 | -3.56 (t-2.3) | -12.56 | 8%  | -78.2 |
| 10      | 89  | -3.17 (t-1.6) | -3.31 | -3.26 (t-1.7) | -12.26 | 9%  | -78.2 |
| 12      | 57  | -2.20 (t-0.8) | -2.41 | -2.76 (t-1.0) | -11.76 | 11% | -78.2 |
| 15      | 37  | -4.04 (t-1.0) | -3.56 | -4.01 (t-1.0) | -13.01 | 16% | -78.2 |
| 20      | 20  | -8.67 (t-1.4) | -7.77 | -7.34 (t-1.2) | -16.34 | 25% | -73.8 |
| 30      | 3   | -26.32(t-1.1) | -24.99| -28.17(t-1.1) | -37.17 | 33% | -70.4 |

ETH base oi-drop>=0.2% / move>=10bps (baseline headline cell, n=174 off):
| min-spike | nPass | gross@0 | gross@2s | NET@2s | <-30%@0 | worst@0 |
|---|---|---|---|---|---|---|
| 0 (off) | 174 | -3.18 (t-2.4) | -2.70 (t-2.1) | -11.70 | 7%  | -78.2 |
| 12      | 72  | -3.70 (t-1.5) | -4.36 (t-1.8) | -13.36 | 11% | -78.2 |
| 15      | 39  | -4.89 (t-1.2) | -5.30 (t-1.4) | -14.30 | 15% | -78.2 |
| 20      | 19  | -11.79(t-1.9) | -10.31(t-1.6) | -19.31 | 26% | -73.8 |
| 30      | 3   | -26.32(t-1.1) | -28.17(t-1.1) | -37.17 | 33% | -70.4 |

ETH base oi-drop>=0.5% / move>=10bps (n=63 off):
| min-spike | nPass | gross@0 | gross@2s | NET@2s | <-30%@0 | worst@0 |
|---|---|---|---|---|---|---|
| 0 (off) | 63 | -4.37 (t-2.2) | -2.84 (t-1.6) | -11.84 | 8%  | -71.3 |
| 12      | 25 | -9.41 (t-2.3) | -7.08 (t-2.2) | -16.08 | 16% | -71.3 |
| 15      | 16 | -9.40 (t-1.6) | -5.80 (t-1.5) | -14.80 | 19% | -71.3 |
| 20      | 10 | -18.20(t-2.3) | -8.30 (t-1.5) | -17.30 | 30% | -71.3 |
| 30      | 3  | -14.61(t-1.0) | +2.04 (t+0.2) | -6.96  | 33% | -35.9 |

ETH read: filtering to bigger realized moves makes the fade WORSE, not better.
The mean falls monotonically (-1.6 -> -8.7 -> -26 at >=30) and the LEFT TAIL
FATTENS (<-30bps 4% -> 16% -> 25% -> 33%; worst trade stays ~-78bps). The biggest
cascades on ETH TREND harder - they are exactly the forced moves that keep going,
not the ones that snap back. Every ETH cell is net-negative at every threshold,
and increasingly so. Thesis REFUTED for ETH (confirms the baseline note: ETH's
bigger spikes overshoot/trend, they don't revert).

### A/B + threshold sweep - BTC (gross fade bps; net = gross-9)
BTC base oi-drop>=0.2% / move>=5bps (n=235 off):
| min-spike | nPass | gross@0 | gross@800 | gross@2s | NET@2s | <-30%@0 | worst@0 |
|---|---|---|---|---|---|---|---|
| 0 (off) | 235 | +0.05 (t+0.1) | +0.57 | +1.03 (t+1.9) | -7.97 | 0% | -27.0 |
| 8       | 62  | +1.74 (t+1.4) | +2.53 | +2.86 (t+2.4) | -6.14 | 0% | -27.0 |
| 10      | 39  | +2.80 (t+1.9) | +3.58 | +3.56 (t+2.4) | -5.44 | 0% | -27.0 |
| 12      | 27  | +3.25 (t+1.7) | +4.20 | +4.42 (t+2.5) | -4.58 | 0% | -27.0 |
| 15      | 13  | +5.71 (t+2.5) | +5.72 | +5.93 (t+2.6) | -3.07 | 0% | -12.8 |
| 20      | 4   | +11.66 (n=4)  | +10.96| +11.44 (n=4)  | +2.44 | 0% | +11.5 |
| 30      | 1   | +11.87 (n=1)  | +10.82| +10.44 (n=1)  | +1.44 | 0% | +11.9 |

BTC base oi-drop>=0.2% / move>=10bps (baseline headline cell, n=75 off):
| min-spike | nPass | gross@0 | gross@2s | NET@2s | <-30%@0 | worst@0 |
|---|---|---|---|---|---|---|
| 0 (off) | 75 | +0.75 (t+0.7) | +2.07 (t+2.0) | -6.93 | 0% | -27.0 |
| 12      | 34 | +3.18 (t+1.9) | +4.36 (t+2.8) | -4.64 | 0% | -27.0 |
| 15      | 16 | +5.43 (t+2.6) | +6.19 (t+3.0) | -2.81 | 0% | -12.8 |
| 20      | 5  | +11.35 (n=5)  | +12.05 (n=5)  | +3.05 | 0% | +10.1 |
| 30      | 1  | +11.87 (n=1)  | +10.44 (n=1)  | +1.44 | 0% | +11.9 |

BTC base oi-drop>=0.5% / move>=10bps (n=30 off):
| min-spike | nPass | gross@0 | gross@2s | NET@2s | <-30%@0 | worst@0 |
|---|---|---|---|---|---|---|
| 0 (off) | 30 | +2.29 (t+1.4) | +3.01 (t+2.1) | -5.99 | 0% | -17.4 |
| 12      | 14 | +1.97 (t+0.7) | +3.53 (t+1.7) | -5.47 | 0% | -17.4 |
| 15      | 7  | +5.56 (t+1.6) | +4.16 (t+1.5) | -4.84 | 0% | -7.5  |
| 20      | 2  | +11.99 (n=2)  | +10.95 (n=2)  | +1.95 | 0% | +11.5 |
| 30      | 0  | (none)        | (none)        | -     | -  | -     |

BTC read: this is the one direction with a real signal, and the filter behaves
EXACTLY as the thesis hoped - QUALITY scales cleanly with magnitude: gross mean,
win%, and t-stat ALL rise monotonically with --min-spike, and the left tail stays
~0% throughout (BTC big cascades do NOT trend like ETH's). At a STATISTICALLY
MEANINGFUL count (thr15, n=13-16, t~2.5-3.0) gross is ~+5.7-6.2bps - the best
honest gross anywhere in the whole study - BUT net of 9bps it is still ~-3bps
(SUB-FEE). Only at thr>=20 does gross clear the fee (~+11-12bps, net +2-3bps) -
but there n COLLAPSES to 4-5 over 10 days (~0.5 trades/day) and the eye-popping
t-stats (+24..+143) are a tiny-n artifact (4-5 near-identical ~+11bps winners),
NOT real significance. At thr30 n=1. So the fee-clearing only appears on a handful
of trades too thin to trust.

### Verdict on Tweak 2 (isolated): NO - it does not make the fade tradeable.
- Does filtering to LARGE cascades lift the reversion above the ~9bps fee floor?
  * ETH: NO - the opposite. Bigger realized moves TREND harder; mean falls to
    -8..-26bps and the left tail fattens to 25-33% <-30bps. Net-negative and
    worsening at every threshold. The big cascades are trenders, not reverters.
  * BTC: quality genuinely improves with size (mean/win%/t all rise, tail ~0),
    and the very biggest moves (>=20bps realized) DO clear the fee net +2-3bps -
    but only at n=4-5 over 10 days (~0.5/day), too thin to trust; the t there is a
    tiny-n artifact. At any meaningful count (thr<=15, n>=13) it is still net
    ~-3bps, sub-fee.
- Knob-bite: VALID. --min-spike moves the set monotonically (335->3->0 ETH;
  235->1 BTC), in-tool counts match the dump exactly.
- So as an ISOLATED change: NOT tradeable. ETH gets worse; BTC gets better-quality
  but either sub-fee (at trustable n) or fee-clearing-but-too-thin (at n<=5).

CONCLUSION: the magnitude filter is a CLEAN, no-lookahead, knob-bite-valid dial,
and it surfaces one honest nugget - on BTC the reversion QUALITY rises smoothly
with cascade size while the left tail stays flat (the structural opposite of ETH,
where size = trend). But alone it does not clear the taker fee at any count we can
trust. The asymmetry (BTC scales with size + no tail; ETH trends with size) is the
real finding to carry into the combined tests: a BTC-only magnitude-gated fade is
the only sub-thread with a pulse, and it needs either (a) more days to see whether
the thr15-20 band holds net-positive on a larger sample, or (b) pairing with the
gapscope-confirm (tweak 3) to keep count while lifting quality. ETH is dead for
the fade in any size bucket. Logs/CSVs: /root/runs/oiscope_mag/{eth,btc}_dump.csv
+ {eth,btc}_base.log + {eth,btc}_confirm_ms*.log.
---

## Tweak 3: order-book confirm + maker-fill feasibility (isolated)

Tested ALONE (exhaustion OFF, magnitude OFF). Both new parts default OFF so the
baseline is byte-preserved (VERIFIED: with all flags off the pooled numbers
reproduce the baseline study EXACTLY - ETH oi0.2/move5 n=335 fade@0ms -1.61
fade@2s -1.25; BTC oi0.2/move5 n=235 fade@2s +1.03 t+1.91). Build + clippy
(-D warnings, --all-targets) + tests all green on the box; oiscope unit tests
11 -> 13 (2 new: confirm-revert gate + maker-fill-through, incl a no-lookahead /
pre-peak assertion); full forgelag suite 99 -> 101 green.

Reuses gapscope's order-book primitives: top-N depth `imbalance()` and the
"trade prints through a resting level" idea (sacred core untouched).

### PART A - REVERT-vs-TREND confirm gate (--ob-confirm, default OFF)

THE RULE. A DOWN cascade was HIT on the BID (forced sells ate bids); an UP
cascade was hit on the ASK. After the fire we watch whether LIQUIDITY RETURNS to
the hit side = top-N depth imbalance shifts back toward that side. We read the
imbalance at the fire (`imb_start`) and again at the end of a confirm window
(`imb_end`); the toward-hit shift = `imb_end-imb_start` (down) / `imb_start-
imb_end` (up). --ob-confirm only takes the fade if that shift >= --ob-confirm-imb
(default 0.10); non-confirming cascades are SKIPPED (no trade). New flags:
  --ob-confirm            enable the gate (default OFF / baseline-preserving).
  --ob-confirm-window <d> confirm window (default 800ms).
  --ob-confirm-imb <f>    min toward-hit imbalance shift (default 0.10).

NO-LOOKAHEAD (the crux). The confirm reads book state only at the fire and at
fire+window. Entry is DELAYED to the confirm-window-end (latency is slack here),
and the same 0/800/2000ms delays are applied AFTER that anchor. So the entry time
is always >= the confirm read time => the gate uses ONLY book state at-or-before
entry. (Window 800ms <= the ~15-23s reversion half-life, so the delay costs little.)

KNOB-BITE (VALID - and it is a real SELECTIVITY filter, unlike tweak1). Cell
oi0.2/move10, varying --ob-confirm-imb, traded set moves MONOTONICALLY:
| min-imb-shift | ETH confirmed | BTC confirmed |
|---|---|---|
| 0.00 | 90/174 = 52% | 47/75 = 63% |
| 0.05 | 80/174 = 46% | 40/75 = 53% |
| 0.10 | 75/174 = 43% | 31/75 = 41% |
| 0.20 | 63/174 = 36% | 28/75 = 37% |
Unlike tweak1 (exhaustion fired on 100% = timing only), the confirm gate SKIPS
~50-64% of cascades and the count moves with the dial. It genuinely separates a
subset. Fees: TAKER fade, ~9bps round-trip. "gross" = captured bps; NET = gross-9.

A/B - ETH (gross fade bps; conf = --ob-confirm @ imb0.10/win800ms; left tail @0ms)
| oi/move | nBase | conf% | base@0 | conf@0 | base@2s | conf@2s (t) | NET conf@2s | conf <-30%@0 | conf worst@0 |
|---|---|---|---|---|---|---|---|---|---|
| 0.2/5  | 335 | 47% | -1.61 | +0.53 | -1.25 | +0.12 (t+0.13) | -8.88 | 3%  | -38.6 |
| 0.2/10 | 174 | 43% | -3.18 | +0.54 | -2.70 | -0.49 (t-0.32) | -9.49 | 4%  | -39.4 |
| 0.2/20 | 35  | 51% | -6.57 | -6.02 | -5.25 | -2.95 (t-0.71) | -11.95| 17% | -64.5 |
| 0.5/5  | 104 | 51% | -2.38 | +0.66 | -0.72 | +1.35 (t+0.99) | -7.65 | 0%  | -26.1 |
| 0.5/10 | 63  | 44% | -4.37 | -1.27 | -2.84 | -1.90 (t-0.85) | -10.90| 4%  | -32.0 |
| 0.5/20 | 12  | 42% | -13.91| -3.75 | -4.86 | -1.68 (t-0.26) | -10.68| 0%  | -26.1 |

ETH read: ob-confirm is the strongest tail-cutter yet. It LIFTS the mean by ~+2-3bps
on every cell, cuts the worst trade from ~-78bps (baseline) to ~-39bps, and flips
the looser cells from clearly-negative to ~flat/slightly-positive (best = 0.5/5
+1.35bps @2s, tail 0%). It is doing exactly what the thesis wanted - liquidity
RETURNING predicts a revert, liquidity STAYING pulled = the trender tail we skip.
BUT: t-stats are ~0-1 (not significant) and NO ETH cell clears the 9bps taker fee -
NET of 9bps is still ~-8 to -12bps everywhere. ETH taker-fade-with-confirm: improved
from "dead" to "flat", still NOT tradeable.

A/B - BTC (gross fade bps; left tail @0ms)
| oi/move | nBase | conf% | base@0 | conf@0 | base@2s | conf@2s (t) | NET conf@2s | conf <-30%@0 | conf worst@0 |
|---|---|---|---|---|---|---|---|---|---|
| 0.2/5  | 235 | 49% | +0.05 | +1.16 | +1.03 | +0.40 (t+0.52) | -8.60 | 0% | -23.0 |
| 0.2/10 | 75  | 41% | +0.75 | +3.34 | +2.07 | +2.87 (t+1.80) | -6.13 | 0% | -13.2 |
| 0.2/20 | 9   | 44% | +5.41 | +10.98| +5.36 | +10.93 (n~4)   | +1.93 | 0% | +10.4 |
| 0.5/5  | 66  | 45% | -0.28 | -0.49 | +0.58 | -1.54 (t-0.88) | -10.54| 0% | -16.8 |
| 0.5/10 | 30  | 37% | +2.29 | +5.99 | +3.01 | +5.80 (t+3.03) | -3.20 | 0% | -8.9  |
| 0.5/20 | 3   | 33% | +11.59| +11.48| +10.64| +10.18 (n~1)   | +1.18 | 0% | +11.5 |

BTC read: ob-confirm consistently LIFTS the gross mean (+2-3bps) and FIRMS the t -
the standout is oi0.5/move10: +3.01bps t2.12 (base) -> +5.80bps t3.03 (confirm) at
2s, tail 0%, worst -8.9bps (traded n ~11). That ~+5.8bps gross at t3 is the best
honest, knob-bite-valid TAKER number in the whole study. BUT net of 9bps it is
still ~-3.2bps (SUB-FEE). The only cells that clear the fee (0.2/20, 0.5/20 net
~+1-2bps) collapse to n~1-4 over 10 days = too thin to trust (tiny-n t artifacts).
So the confirm makes BTC cleaner/firmer but, at any trustable n, the TAKER fade is
STILL ~3bps under the fee wall.

### PART B - post-cascade MAKER-FILL feasibility (--maker-fill, MEASUREMENT ONLY)

METHOD (reuses gapscope's "trade prints through a resting level"). For each
cascade we record the aggressive HL trades over the forward window. After the
spike peak (the reversion leg) we ask: did the tape PRINT THROUGH a resting maker
level, in the reversion direction? Two levels: (1) the pre-cascade BASELINE and
(2) the POST-SPIKE extreme. DOWN cascade reverts UP -> a maker SELL at the level
fills when an aggressive BUY prints >= level; UP cascade reverts DOWN -> a maker
BUY fills on an aggressive SELL <= level. Reported on cascades that REVERTED.
Maker fee taken as ~1.5bps vs ~9bps taker (--maker-fee, default 1.5).

| asset cell | reverted n | baseline-level FILLED | post-spike FILLED | gross revert | NET maker(-1.5) | NET taker(-9) |
|---|---|---|---|---|---|---|
| ETH 0.2/5  | 203 | 126/203 = 62% | 203/203 = 100% | +25.05 | +23.55 | +16.05 |
| ETH 0.2/10 | 104 | 61/104  = 59% | 104/104 = 100% | +31.77 | +30.27 | +22.77 |
| ETH 0.5/10 | 34  | 19/34   = 56% | 34/34   = 100% | +26.18 | +24.68 | +17.18 |
| BTC 0.2/5  | 138 | 85/138  = 62% | 138/138 = 100% | +17.83 | +16.33 | +8.83  |
| BTC 0.2/10 | 45  | 24/45   = 53% | 45/45   = 100% | +23.18 | +21.68 | +14.18 |
| BTC 0.5/10 | 17  | 12/17   = 71% | 17/17   = 100% | +23.42 | +21.92 | +14.42 |

HONEST reading of Part B (do NOT over-read these big positives):
- The 100% post-spike fill is MECHANICAL/tautological - the spike extreme is by
  definition a traded price, so "a trade printed through it on the way back" is
  ~always true. It only says a maker resting AT the spike level always gets filled
  (it is where the forced flow printed). Ignore it as an edge signal.
- The MEANINGFUL number is the BASELINE-level fill: in ~53-71% of reverted cascades
  the recovery actually TRADES BACK THROUGH the pre-cascade level (a resting maker
  there fills); the other ~30-45% re-quote back WITHOUT trading through = the
  re-quote VACUUM gapscope warned about. So post-cascade HL is NOT a total vacuum -
  a maker fills the majority of reverters, partially softening the gapscope worry.
- The "+18-32bps gross, net maker +16-30bps" looks fee-beating, but it is measured
  ONLY on cascades that REVERTED. Selecting on the revert outcome is LOOKAHEAD; and
  revert_bps is the idealized peak recovery (perfect exit), and it EXCLUDES the
  ~40-50% of cascades that TREND (where a resting maker fills and then gets run
  over). So this is a FEASIBILITY UPPER BOUND, not a tradeable number.

### Verdicts (isolated Tweak 3)
(A) Does ob-confirm make the TAKER fade clear fees? NO - but it is the best tweak
    so far. It is a REAL selectivity filter (skips ~50-64%, knob-bite monotonic,
    unlike tweak1), it cuts ETH's worst tail roughly in half (-78 -> -39bps) and
    lifts every cell ~+2-3bps, and it firms BTC oi0.5/move10 to +5.8bps gross t3.03
    (best honest taker gross in the study). But at trustable n NEITHER asset clears
    the ~9bps taker round-trip: ETH still net ~-8 to -12, BTC still net ~-3. Only
    n~1-4 cells clear it. Taker fade with confirm = still sub-fee.
(B) Would a MAKER fade FILL on BTC reverting cascades, and does maker-fee math turn
    BTC's reversion net-positive? FILL: YES, partially - a maker resting at the
    pre-cascade baseline fills in ~53-71% of reverted BTC cascades (the rest =
    re-quote vacuum). FEE MATH: on reverters the maker fee (1.5bps) is trivially
    beaten by the ~18-23bps reversion - BUT that is conditioned on reversion
    (lookahead) and ignores the trender tail, so it is an upper bound, not proof.
    The honest takeaway: maker fills DO happen post-cascade (not a pure vacuum), and
    the binding constraint remains the same as tweak2 - it is the TAKER FEE, and the
    fee-beating path (maker entry that PRE-SELECTS reverters via the confirm gate)
    is now the one combined test left worth running. Isolated, neither part is a
    green light; together (ob-confirm gate PLUS maker fill) is the next experiment.

Knob-bite: confirm dial moves the trade set monotonically (52->36% ETH, 63->37%
BTC across imb 0.00->0.20). Logs: /root/runs/oiscope_ob/{ETH,BTC}_{base,obconfirm,
maker}.log + {ETH,BTC}_kb_imb*.log.