# Lagshot-MAKER hunt - first research pass (coarse maker sweep)

Task 12 of the `lagshot-maker` spec. RESEARCH/ANALYSIS pass on real ETH data.
Honest-skeptic rules apply: report the numbers, name the fee/latency assumptions,
do not flatter. All performance metrics in PERCENT.

## Why this pass exists
Lagshot (the validated cross-venue basis-reversion edge) is REAL but NOT capturable
as a TAKER: real HL signal->fill latency is ~766ms calm and 1.3-2.4s in the
volatility when signals fire, and the reversion closes faster than we can fill, so
we pay 15-22bps chasing a gap that is already gone. The MAKER pivot rests a limit
order instead of racing to take, so entry no longer races latency. The maker's
enemy is ADVERSE SELECTION: a resting order fills preferentially when price is
continuing THROUGH it (the wrong side of a reversion). This pass asks: once we rest,
is there ANY (quote_offset, entry_threshold) region that captures net-positive edge
after realistic fees, with a believable fill rate?

## Setup
- Engine: `forgelag` `lag-hunt --maker` (honest queue+adverse-selection fill model;
  null-edge maker gate passes - a coinflip maker LOSES). Built on the Hetzner box.
- Coin: ETH (HL perp) vs OKX spot ETH-USDT reference (the validated best anchor).
- Data: real ETH days under /root/chd/fresh/ticks (hlbook + trade + funding; OKX ref).
- Days (10, spread across the Nov-Dec 2025 OOS window incl. mixed regimes):
  2025-11-04, 11-08, 11-12, 11-16, 11-20, 11-24, 11-28, 12-02, 12-08, 12-15
  (4.3M-7.6M events/day; 10/10 days loaded cleanly).
- Fixed knobs: reprice_tol=1bps, danger=40bps, maker-exit=2bps, pos-cap=100 (never
  bound; maxInv stayed < 0.9), qty=0.1, windows=500.
- Swept grid (cartesian, 20 cells): quote_offset_bps = {0,1,2,4,8} x
  entry_threshold_bps = {8,12,16,20}.

### Fee + latency realism (the honest operating point)
- REALISTIC fees: maker +1.5bps (realistic HL base maker fee for a small account -
  NOT the engine default, which is a -0.2bps REBATE that flatters makers), taker
  +4.5bps (HL taker; the exit is always a taker market order).
- A maker's ENTRY does not race latency (it rests). But the EXIT is a taker market
  and the danger-PULL is a cancel - both face real HL latency. Realistic operating
  point: `--latency-ns 800000000` (800ms exec) + `--cancel-lat 800ms` (800ms cancel).
- IDEALIZED upper bound: `--latency-ns 0` + `--cancel-lat 0` with the SAME realistic
  fees - the best case a resting maker can ever see. If it loses even here, the
  pivot is dead.
- MAXIMALLY-FLATTERING control: 0ms + the default REBATE schedule (maker -0.2bps,
  taker 2.5bps) - included only to show what the rebate mirage looks like.

### Exact commands
```
# REALISTIC (800ms exec + 800ms cancel + real fees)
lag-hunt --maker --coin ETH --symbol ETH-USDT --dates <10 days> --hours all \
  --latency-ns 800000000 --entry-thr 8,12,16,20 --quote-offset 0,1,2,4,8 \
  --reprice-tol 1 --danger 40 --maker-exit 2 --pos-cap 100 --qty 0.1 \
  --windows 500 --cancel-lat 800ms --maker-fee-bps 1.5 --taker-fee-bps 4.5 --top 20

# IDEALIZED upper bound (0ms + real fees)
lag-hunt --maker ... --latency-ns 0 ... --cancel-lat 0 --maker-fee-bps 1.5 --taker-fee-bps 4.5

# FLATTERING control (0ms + default rebate schedule)
lag-hunt --maker ... --latency-ns 0 ... --cancel-lat 0    # no fee override -> maker -0.2 / taker 2.5
```
NOTE on units: `--cancel-lat` parses durations, so a bare `800` = 800 NANOSECONDS.
Pass `800ms` for 800ms (the smoke run mistakenly used bare 800 = ~0ms cancel; the
graded runs use `800ms`). `--latency-ns` is nanoseconds, so 800000000 = 800ms.

## RESULT 1 - REALISTIC fees, 800ms exec + 800ms cancel (the honest operating point)
Top cells by net mean% (least-negative first). EVERY cell is NEGATIVE.

| off | ent | trips | mean% | t | win% | RR | paper% | maxDD% | fill% | fills |
|----:|----:|------:|------:|----:|-----:|----:|-------:|-------:|------:|------:|
| 8 | 16 | 35 | -0.0046 | -0.22 | 48.6% | 0.94 | -0.7 | 0.37 | 0.3% | 39 |
| 8 | 20 | 25 | -0.0077 | -0.29 | 48.0% | 0.89 | -0.8 | 0.39 | 0.3% | 29 |
| 8 | 8  | 113 | -0.0144 | -1.19 | 47.8% | 0.77 | -4.9 | 1.66 | 0.2% | 118 |
| 4 | 20 | 30 | -0.0215 | -1.48 | 23.3% | 1.53 | -2.6 | 0.40 | 0.4% | 36 |
| 8 | 12 | 55 | -0.0326 | -1.51 | 43.6% | 0.66 | -6.5 | 1.64 | 0.2% | 62 |
| 4 | 16 | 61 | -0.0301 | -2.74 | 23.0% | 1.23 | -6.7 | 0.91 | 0.5% | 69 |
| ... tight offsets bleed hard ... | | | | | | | | | | |
| 0 | 8  | 489 | -0.0696 | -21.75 | 6.3% | 0.91 | -37.1 | 8.40 | 1.0% | 472 |
| 1 | 8  | 343 | -0.0603 | -21.86 | 7.0% | 0.79 | -35.6 | 2.92 | 1.1% | 326 |

Read it: at TIGHT offsets (0/1/2bps) the maker bleeds catastrophically (t -17 to -22,
win 6-9%, paper -31 to -37%) - the classic adverse-selection signature: the only
fills you get are continuation fills (price trading through you = the wrong side of
the reversion). Pushing the offset OUT to 8bps lifts win% to ~48%, but (a) the fill
rate collapses to ~0.3% (the order almost never fills - 25-35 completed trips over
10 days = ~3/day), and (b) expectancy is STILL negative after the 6bps round-trip
fee. Best realistic cell = breakeven-slightly-negative with no fills. NO net-positive
cell exists.

## RESULT 2 - IDEALIZED upper bound, 0ms exec + 0ms cancel, SAME realistic fees
Best cells (least-negative). EVERY cell is still NEGATIVE.

| off | ent | trips | mean% | t | win% | RR | paper% | fill% |
|----:|----:|------:|------:|----:|-----:|----:|-------:|------:|
| 8 | 20 | 29 | -0.0047 | -0.34 | 41.4% | 1.16 | -0.6 | 0.3% |
| 8 | 16 | 39 | -0.0051 | -0.48 | 48.7% | 0.83 | -0.8 | 0.3% |
| 8 | 8  | 154 | -0.0112 | -1.65 | 46.8% | 0.73 | -1.5 | 0.3% |
| 8 | 12 | 68 | -0.0249 | -1.98 | 44.1% | 0.50 | -5.2 | 0.3% |

DECISIVE: 0ms is NOT meaningfully better than 800ms under realistic fees (best cell
~-0.005% at both). This confirms the pivot DID sidestep the latency race - the maker
entry rests, so cutting exec/cancel latency to zero barely moves the result. What
kills it is ADVERSE SELECTION + realistic fees, NOT latency. Since 0ms is the best
case a resting maker can ever see, the pivot fails its own kill-criterion: it loses
even at the idealized upper bound.

## RESULT 3 - MAXIMALLY-FLATTERING control (0ms + default REBATE: maker -0.2 / taker 2.5)
Here, and ONLY here, some offset=8 cells turn POSITIVE:

| off | ent | trips | mean% | t | win% | paper% | fill% |
|----:|----:|------:|------:|----:|-----:|-------:|------:|
| 8 | 8  | 154 | +0.0258 | +3.80 | 68.8% | +17.1 | 0.3% |
| 8 | 16 | 39 | +0.0319 | +3.04 | 69.2% | +5.1 | 0.3% |
| 8 | 20 | 29 | +0.0323 | +2.35 | 72.4% | +3.8 | 0.3% |
| 8 | 12 | 68 | +0.0121 | +0.95 | 66.2% | +3.3 | 0.3% |

THIS IS A REBATE MIRAGE - the exact wall-bot-tournament lie pattern (a rebate makes
a losing strategy look like a winner). The SAME offset=8 cells under realistic fees
(Result 1/2) are NEGATIVE. The gross per-trip edge at offset 8 is only ~3-4bps; the
realistic round-trip fee (1.5 maker + 4.5 taker = ~6bps) is larger than the edge, so
honest fees push it under water. The green appears only when you (a) credit a maker
rebate, (b) use the low 2.5bps taker, AND (c) assume zero latency - none of which a
real EUR500 HL account gets. Even in this flattered case the fill rate is ~0.3%
(29-154 fills over 10 days) - it barely trades.

## Answers to the three key questions
1. FILL RATE: very low everywhere. Reported fill% is 0.2-1.1% (filled / submitted
   quotes, and the denominator is inflated because every reprice counts as a quote),
   but the ABSOLUTE fills are the real story: at usable (wide) offsets the maker
   completes only ~3/day; the only way to fill more is to rest tight, where every
   fill is adverse. There is no offset that gives both a usable fill rate AND good
   fills.
2. ANY net-positive + knob-bite-valid cell after realistic fees? NO. Zero cells net
   positive at either the realistic 800ms point or the idealized 0ms point. The
   knob-bite report is VALID (the entry-thr and quote-offset dials demonstrably moved
   trades - quotes/fills/trips change across every adjacent pair), so the negative
   verdict counts: it is a tested "no edge", not an inert dial.
3. ADVERSE SELECTION: glaring and confirmed. Tight offsets -> win 3-9%, t -8 to -22
   (picked off on continuation). Win% only normalizes (~48% realistic, ~68% rebate)
   when the offset is so wide the order rarely fills. This reproduces the prior
   naive-maker result (t -8..-11, win ~10%) across the WHERE knob.

## PLAIN VERDICT - NO PULSE. Maker pivot is DEAD (killed cheap in sim).
Under honest, realistic HL fees (maker +1.5bps, taker +4.5bps), the resting-maker
Lagshot variant has NO net-positive region - not at the realistic 800ms operating
point and not at the idealized 0ms upper bound. The pivot succeeded at its stated
goal (it sidesteps the latency race: 0ms ~= 800ms), but it runs straight into the
wall it was warned about - adverse selection. A resting order only fills when the
price trades THROUGH it, which is continuation = the wrong side of the reversion;
the clean reversions that made the taker edge real snap back WITHOUT trading through
our resting price, so we never get the good fill. The only positive cells appear in
the rebate-flattered control, and that is a fee artifact, not captured edge.

This is a valid, valuable result: we killed the maker pivot in simulation for the
cost of a few minutes of compute, never risking a euro - exactly the discipline the
project exists for (vs the prior project that lied). 

NO finer sweep / OOS / shuffle control is warranted: the idealized 0ms upper bound
(the most favorable case a resting maker can structurally achieve) already fails
under realistic fees, so there is nothing to refine. Lagshot remains "real but
not capturable by us" - the taker version is latency-locked and the maker version
is adverse-selection-locked. 

RECOMMENDATION: shelve the maker pivot. The next lead should be an edge that does
NOT depend on either out-racing a reversion (taker) or being crossed-to on the right
side (maker) - e.g. Type C forced-flow / liquidation cascades (CHD liq data), per
the standing roadmap.

## TWEAK PASS (2026-06-12) - PATH placement + maker-exit option

Two trader-requested tweaks, both OPT-IN (defaults keep the validated FADE +
taker-exit behaviour, so the prior 80 tests stay green; 84 tests now, all green;
clippy -D warnings clean). Same 10 ETH days, OKX ref, realistic fees
(maker +1.5 / taker +4.5 bps), both 0ms and 800ms, same grid as the fade sweep
(quote_offset {0,1,2,4,8} x entry_threshold {8,12,16,20}).

### What PATH mode does (exact convention as implemented)
FADE (original): rest a quote AS A FADE on the dislocation - dev>0 (HL rich)
rests an ASK ABOVE fair value (sell the richness); dev<0 (HL cheap) rests a BID
BELOW fair value. Price formula: Ask = fv*(1+off), Bid = fv*(1-off).

PATH (new): INVERT the SIDE only; the price formula is unchanged.
- dev>0 (HL rich, expected to FALL toward fv): rest a BID BELOW fv. The falling
  tape trades DOWN through the bid and fills us LONG below fv.
- dev<0 (HL cheap, expected to RISE): rest an ASK ABOVE fv. The rising tape
  trades UP through the ask and fills us SHORT above fv.
Resulting position: PATH puts us in the reversion DIRECTION at fill (long when
rich-and-falling, short when cheap-and-rising). Exit unchanged: revert-to-mean
|dev|<=exit_bps taker market (or maker, tweak 2), plus optional hold backstop.
CLI: --quote-mode fade|path (default fade); --path shorthand. Side-inversion +
in-path fill verified by new unit tests reusing the force_for_test pattern.

### Tweak 2 - maker exit
--maker-exit-order rests the CLOSE as a passive maker Place at fv +/- offset on
the closing side instead of a taker Market, to save the taker exit fee. Falls
back to a taker Market on the hold timeout / danger path (never stuck). Default
off = validated taker exit.

### THE HEADLINE: does PATH fill more than FADE? YES - a lot, at tight offsets.
Same cell, PATH vs FADE, 800ms, realistic fees:

| off | ent | mode | quotes | fills | fill% | trips | mean% | t | win% | paper% | maxDD% |
|----:|----:|:-----|-------:|------:|------:|------:|------:|----:|-----:|-------:|-------:|
| 0 | 8 | PATH | 9414 | 870 | 9.2% | 678 | -0.0708 | -36.85 | 3.1% | -38.0 | 36.92 |
| 0 | 8 | FADE | 49187 | 472 | 1.0% | 489 | -0.0696 | -21.75 | 6.3% | -37.1 | 8.40 |
| 1 | 8 | PATH | 13397 | 884 | 6.6% | 706 | -0.0705 | -40.49 | 2.7% | -38.1 | 17.93 |
| 1 | 8 | FADE | 29179 | 326 | 1.1% | 343 | -0.0603 | -21.86 | 7.0% | -35.6 | 2.92 |
| 8 | 8 | PATH | 36817 | 59 | 0.2% | 48 | -0.0459 | -4.06 | 18.8% | -8.5 | 1.06 |
| 8 | 8 | FADE | 48485 | 118 | 0.2% | 113 | -0.0144 | -1.19 | 47.8% | -4.9 | 1.66 |
| 8 | 16 | PATH | 8217 | 22 | 0.3% | 14 | -0.0282 | -1.04 | 28.6% | -1.6 | 0.40 |
| 8 | 16 | FADE | 14146 | 39 | 0.3% | 35 | -0.0046 | -0.22 | 48.6% | -0.7 | 0.37 |

0ms (idealized upper bound):

| off | ent | mode | quotes | fills | fill% | trips | mean% | t | win% | paper% |
|----:|----:|:-----|-------:|------:|------:|------:|------:|----:|-----:|-------:|
| 0 | 8 | PATH | 39823 | 1909 | 4.8% | 1909 | -0.0693 | -65.02 | 3.0% | -41.1 |
| 0 | 8 | FADE | 52541 | 99 | 0.2% | 99 | -0.0784 | -12.82 | 3.0% | -17.9 |
| 8 | 8 | PATH | 50136 | 54 | 0.1% | 54 | +0.0152 | 0.89 | 51.9% | +3.3 |
| 8 | 8 | FADE | 48353 | 154 | 0.3% | 154 | -0.0112 | -1.65 | 46.8% | -1.5 |

Read it:
- At TIGHT offset (0-1bps) PATH fills FAR more than FADE: 0ms off0/ent8 PATH 1909
  fills vs FADE 99 = ~19x; 800ms off0/ent8 PATH 870 fills (9.2%) vs FADE 472
  (1.0%) = ~1.8x the fills and ~9x the fill rate. The tape DOES trade through a
  quote rested in the path of the move.
- At WIDE offset (8bps) NEITHER fills (PATH 59 vs FADE 118 at 800ms) - the order
  sits far enough out that the move rarely reaches it, same as FADE.
- The extra PATH fills are WORSE, not better: PATH win% 2.7-3.1% / t -37 to -65 at
  tight offsets vs FADE win 6-7% / t -12 to -22. Resting in the path gets you RUN
  OVER - the through-trades are the CONTINUATION / overshoot of the move, so we
  fill LONG into a price that keeps falling (or SHORT into one that keeps rising).
  PATH flips WHICH side is picked off; it does not escape adverse selection, it
  deepens it.

### Net-positive after realistic fees? NO - same verdict as FADE.
- 800ms PATH: every cell negative (least-bad ~-0.028% at off8/ent16, sub-signif).
- 0ms PATH: only the WIDE off8 cells flicker barely positive (mean +0.012 to
  +0.015%, t 0.5-0.9, ~5 trips/day, fill 0.1%) - below significance and gone at
  800ms. Same non-result as FADE wide cells, not captured edge.
- Knob-bite VALID: the entry-thr dial moved trades on every adjacent pair (quotes
  / fills / trips all change), so the negative verdicts count.

### Maker-exit comparison (off8 cells, 800ms, +3m taker hold backstop)
maker-exit does NOT rescue the pivot:
- High-volume cell (off8/ent8): the passive close FAILS to fill in the same
  adverse regime (to close a PATH-long you rest an ask above fv, but price fell,
  so the ask seldom fills), stranding inventory until the 3m taker backstop fires
  - producing a DEGENERATE inventory / drawdown blow-up (maxInv and maxDD ran away
  to absurd values). This is the documented maker-exit risk made concrete, NOT a
  usable result, and a caveat to flag if maker-exit is ever pursued.
- Low-volume cells (ent16 / ent20) show positive t (3.43 / 2.60) but on only 13 /
  7 completed trips over 10 days = noise, with the few-lucky-fills signature
  (win 85%, RR ~13). Not trustworthy.
- The taker-exit equivalent on the same cells is clean and uniformly negative
  (t -1.08 to -3.96). Taker market exit stays the sane choice.

### HONEST READ - is the close trade-driven or phantom? TRADE-DRIVEN.
The prior pass guessed the gap closes WITHOUT trading through our quote (phantom /
pure re-quote). PATH mode DISPROVES that: a quote rested in the path of the move
fills far more than a fade quote (up to ~19x at 0ms). So the close IS
trade-driven - there ARE trades to catch. The problem is WHAT those trades are:
they are the directional continuation / overshoot of the dislocation, not
providable mean-reverting liquidity. Resting in their path gets you run over (long
into a falling tape), so PATH is even MORE adverse than FADE. Either way the maker
is on the wrong side of the print that fills it.

This SHARPENS the kill rather than reopening it: the maker pivot is dead not
because there is nothing to fill against, but because the fillable flow is
momentum, not mean-reversion. That directly informs the queued lead: the trades
around a dislocation are DIRECTIONAL flow - study them as forced-flow / Type C,
do not try to provide liquidity into them. Maker pivot stays SHELVED.