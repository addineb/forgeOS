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