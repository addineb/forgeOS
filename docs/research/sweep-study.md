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
## Option A (structural wick stop) + Option B (flow-reaccel exit)

Re-test (sweepscope EXTENDED, branch forgelag; --dyn-stop/--rr/--stop-buffer +
--flow-exit/--flow-exit-win/--flow-exit-k, all default OFF, baseline byte-preserved;
clippy -Dwarnings clean; 31 unit tests incl 4 new). The trader's call: manipulation
sweeps are DYNAMIC (no fixed levels), so a fixed bps stop / fixed R:R is wrong.
- OPTION A = REVERSAL taker with a STRUCTURAL stop placed just BEYOND the sweep WICK
  (adverse extreme over [fire, entry], no-lookahead) + a buffer; target = rr * stop.
- OPTION B = REVERSAL taker with NO price stop; exit when the FORCED (break-dir)
  aggressive flow RE-ACCELERATES (trailing-window rate >= k * the per-sweep entry-
  window flow rate = self-calibrated, units transfer across assets).
Gated on the EXHAUSTION separator. 10 days ETH+BTC, range-max 80, sweep-margin 5.

### VERDICT (bad first): NO net-positive trade at trustable n - the 9bps TAKER FEE.
Every trustable-n cell, both assets, still NET-NEGATIVE after the 9bps round-trip
taker fee. Best cell ETH 15m/80, rr 2.0, struct(all): n=104 win 40% GROSS +4.0bps
t1.49 -> NET -5.0bps. Higher reward multiples are WORSE (rr 3/4/6 all degrade: the
reversal does NOT run to big targets reliably, so win-rate collapses faster than the
winners grow). Option B (flow-exit) ~ same as A, a touch worse.

### GOOD: the stop idea was RIGHT - it fixed the bleed (first gross-positive cell).
The dynamic structural wick-stop flipped the ETH 15m/80 reversal taker from GROSS
-1.4bps (fixed 15bps stop) to GROSS +4.0bps, win 27% -> 40% - the FIRST time this
setup's reversal trade is gross-positive at a trustable n. Confirms the trader: the
fixed stop really was the leak (the sweep's own overshoot was tripping it). Structural
stops came out SMALL (~10-14bps) because entry is 60s after the poke (the confirm
window), by which point the wick is close - so the win is from the dynamic R:R, not a
wide stop.

### Bottom line + next
Stop-bleed is FIXED (gross turned positive) but the BINDING WALL is back to the ~9bps
TAKER FEE (gross +4 < fee 9). Same wall as every prior lead. The only escape on this
setup = CUT THE FEE: a MAKER entry (~6bps RT) gated on EXHAUSTION (which picks the
80-90% reverters, so resting limits should dodge most of the run-over/adverse selection
that killed the earlier maker test) + the structural/flow exit. Logs /root/runs/optab*.
## Option A maker (fee escape) - REJECTED (adverse selection)

Test (sweepscope --maker-rev + --rr/--stop-buffer, default OFF; clippy clean, 32
tests incl new maker test green). Resting-limit REVERSAL at the swept edge (~6bps RT
vs 9bps taker), structural wick stop, EXHAUSTION hold-gate. 10 days ETH+BTC.

VERDICT (bad): WORSE than the taker - GROSS goes NEGATIVE before fees. Every cell,
both assets: maker fills ~100% (the aggressive poke ALWAYS trades through the resting
edge) but GROSS -8 to -10bps, win 10-21%, t -5 to -10. NET-of-6 ~ -14 to -16bps. The
EXHAUSTION hold-gate does NOT help: the fill happens AT the poke (before the 60s
confirm window), so the gate only flattens at the window-end = locks in the adverse
move (gate cells are MORE negative, RR collapses to 0.2-0.9). This is textbook ADVERSE
SELECTION: the resting limit fills precisely because price is pushing THROUGH it
(continuation in that instant); on the ~60% reverters it works, but the fill bias is
to the wrong side -> gross-negative. The cheaper maker fee is irrelevant when gross is
already deeply negative. Confirms the earlier maker-fade kill on a different setup.

### SWEEP LEAD - EXHAUSTED (both doors closed, evidence-based)
- TAKER (Option A struct): stop-bleed FIXED, gross turned +4bps at trustable n, but the
  9bps taker fee eats it -> net -5. Best version, still sub-fee.
- MAKER (fee escape): cheaper fee but fills into the knife (adverse selection) ->
  gross -8 -> net -14. Worse.
DURABLE WINS to carry forward: (1) EXHAUSTION/flow-deceleration is a REAL no-lookahead
directional read (+15-30pp P(reversal), both assets) - the first separator that beats
random; (2) the dynamic STRUCTURAL WICK-STOP fixes stop-bleed (turns the taker reversal
gross-positive). Both are reusable for a future edge that is BIGGER per trade (>>9bps)
or runs on LOWER fees. The sweep reversal itself is ~a-few-bps gross = below the fee
floor, same wall as every prior lead. No deployable strategy here. 0 euros risked.
## Test A - RECLAIM entry (wait for re-entry into the range) - BEST RESULT YET

sweepscope --reclaim (default OFF; clippy clean, 34 tests incl 2 new). Instead of
entering at the poke / 60s anchor, WAIT for price to come back INSIDE the range (the
classic failed-breakout reclaim), then enter the reversal with the structural stop
beyond the already-printed wick (~26-38bps). 10 days ETH+BTC, range-max 80, margin 5,
rr sweep {1.5,2,3} + fixed-40.

### VERDICT (good, with an honest caveat): FIRST net-positive cells - but THIN.
GOOD: the reclaim entry closes the capture gap that the late poke-entry was bleeding.
- "all" reclaim: gross flipped from ~0 (poke entry) to +4-10bps, win 43-51%
  (ETH 15m/80 rr1.5 +6.7 t1.50; BTC 30m/80 rr1.5 +10.4 t1.81 NET +1.4 n=45).
- EXHAUSTION-gated subset = NET-POSITIVE after the FULL 9bps taker fee:
    ETH 15m/80 +EXH: rr1.5 +5.2 / rr2 +7.3 / rr3 +8.6bps NET (n=11, win 55%, gross
    +14-18, RR 2.0-2.3) - positive on EVERY reward dial (robust, not a knife-edge).
    BTC 15m/80 rr1.5 +EXH +5.9bps NET (n=7).
This confirms the 85%-revert read was a REAL edge all along; the 60s-late poke entry
was throwing it away. Entering on the reclaim (so the overshoot is already behind us,
and the stop sits beyond the printed wick) recovers it.

CAVEAT (bad/honest): the net-positive cells are THIN (n=7-11, t~1.1-1.3 = NOT yet
significant, need t>2). The bigger-n "all" cells are ~breakeven (best +1.4). 30m/BTC
+EXH is shaky small-n (negative). So this is PROMISING, NOT PROVEN - it could still be
small-sample noise.

### Next gate
MORE DAYS on the winning config (reclaim + EXHAUSTION gate, ETH 15m/80, rr~2) to push n
and t past significance (t>2). We have 61d Nov-Dec 2025 + Feb from the prior OOS pulls.
If the +5-9bps net survives at trustable n with t>2 and an OOS period -> first real
candidate from this whole study. Logs /root/runs/reclaim/.
## Test A @ SCALE (61d train Nov-Dec25 + 36d OOS Feb-Jun26) - DID NOT SURVIVE

Ran the winning config (reclaim + EXHAUSTION gate, rr2, 15m/80 + 30m/80) on the full
data: 61 train days + 36 independent OOS days, ETH+BTC.

### VERDICT (bad): the 10-day net-positive was SMALL-SAMPLE OPTIMISM. No robust edge.
- The EXHAUSTION SEPARATOR COLLAPSES at scale. P(rev|exhausted) vs base:
  ETH train 15m/80 73% vs 70% (+3pp); 30m/80 69% vs 68% (+1pp); BTC NEGATIVE
  (71% vs 75%). The +15-30pp lift seen on 10 days was an ARTIFACT - at 61 days it is
  ~+1-3pp = essentially gone.
- +EXH NET (after 9bps) mostly negative/breakeven: ETH train 15m/80 -3.0 (n=60),
  30m/80 +0.6 (n=45); BTC train 30m/80 -11.4 (n=26). OOS: ETH 15m/80 +EXH DIED
  (gross +1.0, net -8.0); ETH 30m/80 net -2.7.
- Does NOT replicate train<->OOS: BTC 30m/80 +EXH train -11.4 vs OOS +13.3 (t2.48,
  n=17) = a SIGN FLIP = noise, not signal (the one t>2 cell has a negative train twin
  and n=17 - not trustable).
- The big-n "all" reclaim cells are ~breakeven gross (ETH train +2.5/+3.0, OOS
  -0.1/+0.9), never near the 9bps fee.

### GOOD (small, honest)
The reclaim ENTRY is a real mechanical capture improvement (it moved the "all" gross
from negative to ~0-+3bps and win to 38-45%) - keep it as a TECHNIQUE. But it does not,
by itself, create a net edge.

### SWEEP LEAD - CONCLUDED
Real setup; the reclaim entry improves capture; the structural wick-stop fixes bleed -
but NO robust net-positive edge survives a proper sample + OOS, and the EXHAUSTION
"tell" was OVERFIT to 10 days (collapses to +1-3pp at 61d). The bigger sample caught
the overfit BEFORE any euros were risked - the process working (unlike the prior project
that lied). Durable, reusable across future leads: (1) reclaim entry, (2) structural
wick-stop, (3) the lesson that a 10-day separator MUST be re-checked at 60d+ / OOS
before trust. 0 euros risked.
## Ladder (grid) entry on the sweep reversal - PER-TRADE LIE, euro-realistic NEGATIVE

Built the trader's LADDER entry (simulate_ladder: N maker rungs INTO the dislocation
from the sweep extreme, average entry = mean of FILLED rungs, exit toward value, HARD
invalidation = full-size stop, run-overs counted; clippy + 38 tests green). Fee 6bps
(maker in + taker out). 61d train + 36d OOS, ETH+BTC.

### The trap, caught by size-weighting
PER-TRADE (each trade = 1 unit) looked like the best result ever: net +8 to +12bps,
win 69-73%, t 5-9, and it REPLICATED OOS (BTC r120 train +10.9 -> OOS +12.2). It was a
LIE. The ladder fills PARTIAL size (~2 rungs) on winners but FULL size (5 rungs) on the
big invalidation run-overs. Weighting P&L by the capital actually deployed (SIZE-
WEIGHTED, euro-realistic):
- EVERY one of 36 configs (rungs{3,5} x range{40,80,120} x invalid{10,20,40}), BOTH
  assets = NEGATIVE: -5.0 to -11.2bps. NONE positive.
- worst-case full-size hit -115 to -563 bps-units (5 rungs x ~100bps).
ROOT CAUSE: deep overshoots fill the most rungs (most size) but are disproportionately
real BREAKOUTS that do NOT revert -> the ladder loads maximum size into the losers.
No stop/range/rung setting fixes it (swept the whole space).

### What it DID prove (constructive)
The ladder fixed the FEE (6 vs 9) and the ENTRY-TIMING (per-trade flipped from breakeven
to strongly positive) -> those WERE real problems the trader correctly identified. The
remaining, unbeatable killer on THIS setup is the SIZE asymmetry (averaging up into
trends), which is intrinsic to a fixed-per-rung grid and needs a directional read
(reversal vs continuation at the deep end) we have never found.

### Lesson (permanent)
A variable-size entry (grid/ladder/DCA) MUST be judged SIZE-WEIGHTED, never per-trade -
per-trade equal-weighting hides that losers are bigger than winners. This is the exact
class of bug that made the prior project lie. The size-weighted + worst-case check is
now in sweepscope and must be used for any future ladder/scale-in test. Logs
/root/runs/ladhunt, /root/runs/ladoos, /root/runs/ladsw.
## Ladder CORRECTION (per-trade sizing = the right lens) + EUR/drawdown

Trader clarified: the ladder is ONE trade (fixed position; rungs only set the average
entry), NOT one-trade-per-rung. So the SIZE-WEIGHTED (per-rung) view above was the wrong
lens for how he sizes - it only applies if deeper fills add MORE risk (grid/martingale).
Under fixed-size-per-trade (the ladder just improves the average entry, total position
fixed/risk-capped to the invalidation), the PER-TRADE equal-weight is correct.

### Result (97 days full, ETH r120/i30 + BTC r80/i30, fee 6, lookback 15m, margin 5)
- PER-TRADE EDGE (validated IN + OUT of sample): ETH n=905 win 72% net +11.2bps t=10.5;
  BTC n=587 win 70% net +10.6bps t=9.5. Combined ~15 trades/day. Worst single -106bps.
  This is the strongest, only OOS-replicating edge in the whole project.
- EUR (NON-COMPOUNDED, fixed notional per trade - removes the sequential-compounding
  lie; trades overlap so you CANNOT compound them serially): EUR500 over ~3 months ->
  1x notional +163% (maxDD 3.4%), 2x +326% (maxDD 5%), 4x +653% (maxDD 9.4%).
- SEQUENTIAL-COMPOUNDED (EUR260k / +51,858%) = FANTASY, rejected: ~15 overlapping
  trades/day cannot be compounded serially and cannot all carry full size concurrently.

### Honest gates remaining before this is trusted (NOT yet deployable)
1. CONCURRENCY/sizing model: ~15 overlapping trades/day; total exposure must be capped
   (can't put 4x on every concurrent trade) - the realistic euro is nearer the 1x-2x
   line with a concurrency cap.
2. PARTIAL FILLS: each trade treated at full size; partial-fill winners are smaller.
3. MAKER-FILL realism: rungs assumed to fill on touch (fine for tiny retail size).
4. SLIPPAGE + the -106bps worst-trade tail (already in the additive curve + maxDD).
5. Regime coverage (Nov-Dec + Feb/May/Jun) and a LIVE paper run.

### Verdict
First real, OOS-validated per-trade edge. The ladder (trader's idea) + correct maker fee
(6 vs 9) + letting it run is what unlocked it - exactly the execution fixes he insisted
on. Euro is genuinely positive even non-compounded with modest drawdown. Next = a proper
concurrency-capped position-sizing sim + live paper, NOT more parameter hunting.
## Concurrency-capped sizing sim - PASSED (concurrency is a non-issue)

Event-driven sim from the ladder dumps (entry + exit times). MAX concurrent open
positions across the whole 1492-trade window = ONLY 2 (30m cooldown + fast resolution
-> signals rarely overlap within a coin; basically ETH + BTC at most). So a cap of 2
skips zero trades and compounding is legitimate. EUR500, ~222d window:
- cap3 / 1x notional (safe): -> EUR2501 (+400%), maxDD 5.6%, worst single -EUR25.
- cap2 / 2x notional: -> EUR11970 (+2294%), maxDD 11%, worst single -EUR233.
- cap1 / 4x notional (aggressive): -> EUR59584, maxDD 25%.
The earlier sequential-compounded EUR260k was inflated by ignoring time; the proper
event-driven number is +400% conservative / +2294% at 2x, single-digit-to-11% DD.
Remaining gates before live: partial fills, taker-exit slippage, maker-fill realism as
size grows, regime coverage, and a LIVE PAPER run. First genuine deployable candidate.