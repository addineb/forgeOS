# Sideways stop-run / liquidity-sweep swing study (sweepscope)

Analysis-only. New lead: the BIGGER-per-trade swing edge (tens-to-hundreds of bps
targets), deliberately leaving the tick-scale microstructure that is dead at the
~9bps taker fee. Tool: `crates/forgelag/src/bin/sweepscope.rs` (reuses load_window
+ forge_book::OrderBook + top-N microprice + imbalance/top_depth; sacred core
untouched; clippy -D warnings clean; 16 unit tests + full forgelag suite green).
Data: HL ETH then BTC, 10 study days (2025-11-04,-08,-12,-16,-20,-24,-28,
12-02,-08,-15), all hours. Box run /root/runs/sweepscope/ (eth.log, btc.log,
eth_sweeps.csv, btc_sweeps.csv).

## VERDICT (one line)

NO tradeable pulse. The setup is REAL and the moves are BIG (reversal ~45-80bps,
continuation ~52-126bps mean - well over the fee bar), BUT (a) the orderflow
confirm does NOT separate reversal from continuation ahead of time, and (b) NO
honest trade - reversal-maker, reversal-taker, or continuation-taker - clears
net-positive at a trustable n (>30). Every n>=30 cell, both assets, all three
trades = NET NEGATIVE.

## What was mechanised (no presupposed direction, no-lookahead)

- SIDEWAYS RANGE over lookback L: [lo,hi] from the HL top-5 microprice; sideways
  only if width (hi-lo)/mid*1e4 <= range-max bps (skip if already trending). The
  poke is tested against the range ESTABLISHED before the current tick (never
  folded into its own range) = no lookahead.
- SWEEP: microprice pokes BEYOND an edge by margin S bps (above hi = UP sweep,
  below lo = DOWN sweep). De-duped by a 30m cooldown; one sweep = one event.
  OI-drop and net aggressive flow over the sweep are RECORDED (not gating).
- OUTCOME (measured, presupposing nothing): from the sweep extreme, over a 90m
  forward horizon, first-touch CONTINUATION (extends >= 20bps past the sweep
  before re-entering the range) vs REVERSAL (re-enters the range first).
- TWO honest first-touch trades (run-overs INCLUDED, realistic fees):
  (A) REVERSAL - enter AGAINST the sweep (UP->short / DOWN->long). MAKER variant:
      rest a limit AT the swept edge; the sweep trades THROUGH it = a maker fill
      (~6bps RT: 1.5 maker in + 4.5 taker out). TAKER variant too (~9bps RT).
      target 40bps / stop 15bps.
  (B) CONTINUATION - enter WITH the break (taker, ~9bps RT). target 60 / stop 20.
- ORDERFLOW CONFIRM (the separator, method-1): top-5 depth imbalance ABSORBED
  back toward the hit side (reversal-supporting) vs STAYS PULLED (continuation-
  supporting), read at fire+60s <= entry (no-lookahead), used as an optional gate.
- Grid (knob-bite): lookback L {15m,30m,60m} x range-max {30,50,80}bps x
  sweep-margin {5,10,20}bps = 27 cells. (margin=5 cells carry the events; wider
  margins fire only a handful of times = not trustable.)

## (1) Continuation-vs-reversal split + sizes

The setup is genuine. Sweeps fire and resolve into sizeable moves both ways.
Trustable cells (n>=30, all margin=5):

| coin | L/range/margin | n | continuation | reversal | cont mag (mean) | rev mag (mean) |
|------|----------------|---|--------------|----------|-----------------|----------------|
| ETH | 15m/50/5 | 83 | 41% | 59% | 98bps | 65bps |
| ETH | 15m/80/5 | 104 | 32% | 68% | 115bps | 80bps |
| ETH | 30m/80/5 | 83 | 36% | 64% | 112bps | 71bps |
| ETH | 30m/50/5 | 49 | 35% | 65% | 103bps | 66bps |
| BTC | 15m/50/5 | 52 | 40% | 60% | 72bps | 54bps |
| BTC | 15m/80/5 | 66 | 35% | 65% | 72bps | 61bps |
| BTC | 30m/80/5 | 50 | 36% | 64% | 78bps | 62bps |
| BTC | 60m/80/5 | 32 | 25% | 75% | 79bps | 68bps |

REVERSAL is the more common outcome (~60-75%); continuation is the minority
(~25-40%) but when it runs it runs FAR (means 70-126bps; ETH bigger than BTC).
Time-to-decide medians ~30-180s. Both magnitudes comfortably exceed the ~6-9bps
fee bar - which is exactly why this looked promising and why the failure is NOT a
fee-floor problem this time.

## (2) Does the orderflow confirm separate them ahead of time? NO.

The depth-imbalance confirm does NOT predict reversal vs continuation. Across
every trustable cell, P(reversal | absorption-confirm) ~= P(reversal | no-confirm):

| coin | cell | P(rev \| absorb) | P(rev \| no-absorb) |
|------|------|-----------------|---------------------|
| ETH | 15m/50/5 | 60% (n=45) | 58% (n=38) |
| ETH | 15m/80/5 | 68% (n=59) | 69% (n=45) |
| ETH | 30m/80/5 | 62% (n=39) | 66% (n=44) |
| BTC | 15m/80/5 | 68% (n=28) | 63% (n=38) |
| BTC | 30m/80/5 | 57% (n=23) | 70% (n=27) |

Knob-bite: the gate DOES move the trade set (it filters ~half the sweeps), so the
dial bites mechanically - but it moves the outcome probability by ~0. It is
selecting essentially at random with respect to the thing we need it to predict.
A few small-n cells (n<=14) flatter the gate (e.g. ETH 15m/50/10 rev-TAKER+absorb
+8.2bps net, t1.97, n=12; BTC 60m/80/5 rev-TAKER+absorb +2.3, t1.47, n=14) but the
separation table shows that is noise, and t-stats sit below 2. So: the absorption/
imbalance confirm is NOT a valid separator for this setup.

## (3) Does either trade clear net-positive at trustable n + good RR? NO.

Every n>=30 cell, both assets, all three trades are NET NEGATIVE:

ETH 15m/80/5 (n=104):
- rev-MAKER (all)        GROSS -10.5  t -5.55  win 12%  RR 2.42  NET(-6) -16.5
- rev-MAKER (+absorb)    GROSS  -8.6  t -4.95  win 19%           NET(-6) -14.6
- rev-TAKER (all)        GROSS  -1.4  t -0.54  win 27%  RR 2.41  NET(-9) -10.4
- rev-TAKER (+absorb)    GROSS  -0.7           win 29%           NET(-9)  -9.7
- cont-TAKER (all)       GROSS  -2.6  t -0.76  win 24%  RR 2.66  NET(-9) -11.6

BTC 15m/80/5 (n=66):
- rev-MAKER (all)        GROSS -10.6  t -5.19  win 12%  RR 1.92  NET(-6) -16.6
- rev-TAKER (all)        GROSS  +2.0  t +0.63  win 35%  RR 2.23  NET(-9)  -7.0
- rev-TAKER (+absorb)    GROSS  +3.4           win 39%           NET(-9)  -5.6
- cont-TAKER (all)       GROSS  -1.6  t -0.39  win 26%  RR 2.58  NET(-9) -10.6

The pattern is identical everywhere:

- REVERSAL MAKER (the fee-escape hope) is GROSS NEGATIVE even before fees
  (-6 to -27bps). It "fills" 100% by construction - a sweep always trades through
  a limit resting at the swept edge - but that is the trap: you fill INTO the
  continuing poke and get stopped (win 0-25%). Resting at the edge catches the
  knife. The cheaper fee does not matter because the entry is bad.
- REVERSAL TAKER (enter at the poke extreme) is GROSS ~0 (-6 to +2bps); the 9bps
  fee turns it net -7 to -15. The stop placed just beyond the poke is hit by
  further overshoot before the 40bps reversal target prints.
- CONTINUATION TAKER is GROSS ~0 (-6 to +1bps); ~60-75% of sweeps reverse and
  stop it (win 16-29%). Net -8 to -20.

RR (win-size / loss-size) is HEALTHY (~2.0-2.8) because targets > stops, but win%
is far too low: first-touch stops bleed on the wrong side and overshoot trips the
stop before the (real, large) move develops. The big magnitudes are real; we just
cannot capture either side honestly without knowing direction in advance - and
the confirm cannot tell us (see (2)).

## Why it fails (the binding constraint this time)

Unlike every prior lead, per-trade SIZE is NOT the problem - the moves clear the
fee. The killers are: (1) DIRECTIONAL UNCERTAINTY - reversal vs continuation is
~60/40-to-75/25 and the orderflow confirm does not separate them ahead of time;
(2) STOP-BLEED - honest first-touch entries get stopped by the sweep's own
overshoot before the large move plays out; (3) the maker fee-escape backfires
because resting at the swept edge fills you into the run-over (gross-negative
before fees). The constraint shifted from "fee floor vs tiny edge" to "cannot pick
the side, and stops bleed."

## Caveats + cheap follow-ups (only if revisited)

- This is the cheap first look (10 days, 90m horizon, fixed target/stop grid,
  imbalance-only confirm). It is enough to reject the naive setup but not a deep
  spec.
- The separation failure (2) is the structural killer; tuning target/stop/L
  cannot fix a confirm that does not predict direction. A DIFFERENT separator
  (not depth imbalance) would be needed - but method-1's confirm IS the depth
  imbalance, which failed here exactly as it did as a microstructure indicator.
- A maker resting DEEPER (mid / far edge) rather than at the swept edge, or a
  taker entry only AFTER confirmed range RE-ENTRY, are the only ideas that target
  the bad-entry problem - but they still need a directional read we do not have.
- More days would tighten the small-n cells, but the large-n cells already speak
  clearly and consistently across both assets; more days are unlikely to flip a
  uniformly net-negative, separation-dead result.
## Flow-separator (3-state: absorbed/exhausted/continuation)

Re-test (sweepscope EXTENDED, branch forgelag, same bin): replace the dead depth-
imbalance confirm with the trader's flow-vs-impact model. MASTER VARIABLE =
price-impact-per-unit-forced-volume over the confirm window [fire, fire+60s]
(every read <= entry = no-lookahead): LOW impact = ABSORBED (predicts REVERSAL),
HIGH impact = EASY-PUSH (predicts CONTINUATION); plus a separate FLOW-DECEL
measure (late-half forced volume << early-half) = EXHAUSTED (predicts REVERSAL).
Aggressive volume + impact come from the HL trades; depth-ahead consumed-vs-pulled
read from the book. Sweep detection + the honest first-touch trade sims (rev
maker@edge + taker, cont taker) UNCHANGED - only the separator/gate changed.
Optional --min-oidrop liquidation gate (reported WITH and WITHOUT). All thresholds
configurable (--impact-lo/--impact-hi/--flow-decay/--absorb-minvol) and swept
(self-calibrated percentile knob-bite). Same 10 days ETH+BTC, populated grid
L{15,30,60}m x range-max{50,80} x margin 5. clippy -Dwarnings clean; 27 unit tests
(+4 new: 3-state assignment, impact-cut knob-bite, flow-decay knob-bite) + full
forgelag suite green. Logs /root/runs/sweepscope_flow/{eth,btc}.log + _rows.csv.

### VERDICT (one line)
PARTIAL WIN. BAD FIRST: still NO net-positive trade at trustable n (>=30) - every
gated reversal and continuation trade is net-negative after fees, WITH or WITHOUT
the OI-drop gate (the only green cells are thin n=9-19 noise). GOOD - and this is
the unlock: unlike depth-imbalance (which was RANDOM, ~0 lift), the flow model
SEPARATES. FLOW DECELERATION (exhaustion) lifts P(reversal) ~+15-30pp over the base
rate on BOTH ETH and BTC, no-lookahead. We finally have a real directional read;
we just cannot yet monetize it through the honest first-touch trade (stop-bleed +
fee still bind).

### (1) Did it separate where depth-imbalance failed? YES - via DECELERATION.
Self-calibrated percentile knob-bite, P(outcome | signal) vs P(outcome | !signal)
over decided sweeps. EXHAUSTION (decel = late forced-vol / early forced-vol; lower
cut = flow dying harder) is the robust, BOTH-ASSET separator:

| coin | cell | base rev | dec<= cut | P(rev|sig) | n | lift |
|------|------|----------|-----------|------------|---|------|
| ETH | 15m/80 | 68% | p20 | 86% | 22 | +23pp |
| ETH | 15m/80 | 68% | p30 | 84% | 32 | +23pp |
| ETH | 30m/80 | 64% | p20 | 82% | 17 | +23pp |
| ETH | 60m/80 | 72% | p20 | 90% | 10 | +22pp |
| BTC | 15m/50 | 60% | p30 | 75% | 16 | +22pp |
| BTC | 15m/80 | 65% | p30 | 86% | 21 | +30pp |
| BTC | 15m/80 | 65% | p40 | 81% | 27 | +28pp |
| BTC | 30m/80 | 64% | p10 | 83% |  6 | +22pp |

The lift concentrates in the strongly-decelerating tail (p10-p30, n 6-32) and
washes to ~0 by p50 - i.e. it is the sweeps whose forced flow DIES that revert.
Consistent in sign across nearly every trustable cell, both assets. This is the
opposite of the depth-imbalance confirm, which gave P(rev|absorb) ~= P(rev|no-
absorb) everywhere.

ABSORPTION via the impact ratio (LOW impact -> reversal) is ASSET-SPLIT, not robust:
- BTC: WORKS as hypothesised - imp<= p10-p40 lifts P(rev) +12..+41pp (e.g. BTC
  30m/80 imp<=p10 100% n=6 +41pp; 30m/50 imp<=p20 86% n=7 +25pp).
- ETH: INVERTED - low impact predicts LESS reversal (15m/50 imp<=p50 48% vs base
  59%, lift -23pp; 30m/80 imp<=p10 -22pp). On ETH high-impact "easy-push" sweeps
  REVERSE more and low-impact "absorbed" sweeps CONTINUE more - the reverse of the
  hypothesis. So on ETH the discriminator is the flow TIME-PROFILE (decel), not the
  impact magnitude. EASY-PUSH (high impact -> continuation) is weak/negative on ETH
  and only weakly positive on BTC at small-n upper cuts.

### (2) Tradeability: NO net-positive at trustable n (gated, run-overs IN, real fees).
Gate REVERSAL on (ABSORBED|EXHAUSTED), CONTINUATION on (CONTINUATION-sig). Configured
dials impact-lo 0.02 / impact-hi 0.05 / flow-decay 0.10 (ETH-scaled; impact units do
NOT transfer across assets - BTC lands nearly all CONTINUATION).

| coin | cell (n) | rev-MAKER net | rev-TAKER net | cont-TAKER net |
|------|----------|---------------|---------------|----------------|
| ETH | 15m/80 (104) | -15.2 | -13.2 | -18.1 |
| ETH | 30m/80 (83)  | -14.6 | -14.3 | -21.3 |
| ETH | 15m/50 (83)  | -16.0 | -14.9 | -14.4 |
| BTC | 15m/80 (66)  | -16.6 | -4.2 (n=9) | -13.0 |
| BTC | 30m/80 (50)  | -17.0 | -15.7 (n=6) | -14.9 |

Every n>=30 cell, both assets, all three gated trades NET-NEGATIVE. rev-MAKER stays
gross-negative before fees (fills into the continuing poke = catches the knife, win
7-26%). The honest first-touch stop is still tripped by the sweep's own overshoot
before the (real, big) reversal develops - the separation tells us reversal is LIKELY
(~80-90% on the decel tail) but the TRADE STRUCTURE cannot capture it net. Best green
cells are thin-n artifacts (ETH 60m/80 +OI rev-TAKER +7.2bps n=9; ETH 30m/50 +OI
rev-TAKER -6.5 n=12) - not trustable.

### (3) OI-drop gate: no unlock.
--min-oidrop 0.05% keeps ~40-55% of sweeps (e.g. ETH 15m/80 43 of 104; BTC 15m/50
22 of 52). Separation roughly preserved but n drops; NO trustable-n trade flips
net-positive with the gate (ETH 15m/80 +OI rev-TAKER n=25 NET -17.0). The
liquidation footprint does not, by itself, turn the setup tradeable here.

### Knob-bite: VALID.
State counts and lifts move monotonically with the percentile cuts and with the
--impact-lo/--impact-hi/--flow-decay dials; unit tests impact_cut_knob_bites_state
+ flow_decay_knob_bites_state pin it. The depth-ahead consumed-vs-pulled proxy was
UNINFORMATIVE (top-5 depth reprices around the sweep -> the [0,1] consumed share
saturates at 1.0); it is reported but carries no signal - exhaustion/impact carry it.

### Bottom line
Depth-imbalance was a random separator; FLOW DECELERATION is a REAL one (+15-30pp,
both assets, no-lookahead) - that is the genuine progress and the reusable primitive.
But a working SEPARATOR is necessary, not sufficient: the honest first-touch trade
still bleeds on overshoot + fees, so 0 promote. NEXT (if revisited): a trade STRUCTURE
that survives the overshoot (enter on confirmed range RE-ENTRY, or wider/scaled stop)
applied to the exhaustion-selected reversals - the prediction is there to build on.