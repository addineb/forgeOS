# Research Note -- Q2: Measuring Impact, Predicting Move SIZE + PERSISTENCE (2026-06-09)

From the research session (see `docs/research-charter.md`, Q2). Question: what is
the best way to MEASURE the force of a push and predict move SIZE and PERSISTENCE
from OUR data -- most predictive vs cheapest to compute? Same ground rules:
additive/flag-gated, backtest before live, every bar net of ~11 bps.

## The core insight: SIZE and PERSISTENCE are TWO different predictions
A bot needs two numbers, and they come from different measurements:
- SIZE  = how far will the push go? (decides: does it clear 11 bps at all?)
- PERSISTENCE = how long until it stalls/reverts? (decides: how long to hold?)
Treating "predict the move" as one thing is the mistake. Below, each gets its own
best-and-cheapest tool.

================================================================================
PART 1 -- MOVE SIZE
================================================================================

## Two regimes (do not use one model for both)
1. SHORT horizon / small flow: price change is ~LINEAR in OFI, slope ~ 1/depth
   (Cont, Kukanov & Stoikov, arXiv:1011.6402). Cheapest, most predictive at the
   sub-minute scale. We already compute OFI.
2. LARGE pushes / metaorders: impact follows the SQUARE-ROOT LAW -- impact ~
   sqrt(Q/V) in cumulated signed volume. Confirmed ON BITCOIN over ~4 decades of
   size (Donier & Bonart, "A Million Metaorder Analysis of Market Impact on the
   Bitcoin", arXiv:1412.4503), and it holds along the WHOLE trajectory, not just
   the final fill. Refinements: a logarithmic form fits even better over 5 orders
   of magnitude and impact relaxes to ~2/3 of peak (Zarinelli, Treccani, Farmer &
   Lillo, arXiv:1412.2152); participation rate adds a second sqrt (Durin,
   Rosenbaum & Szymanski, arXiv:2311.18283).

## What this means for us
- Convert the size estimate into BPS and compare to the 11 bps wall DIRECTLY:
  only arm a trade when predicted move > 11 bps + margin. This is the single most
  useful output -- it turns "is there a signal" into "is the signal big enough to
  pay for itself".
- Crypto note: trade-flow imbalance explains contemporaneous crypto price change
  at least as well as aggregate OFI, and the linear relation strengthens over
  larger intervals (crypto order-flow analyses). We have signed trades, so this
  is cheap.

================================================================================
PART 2 -- PERSISTENCE / DECAY (how long to hold)
================================================================================

## Mechanism + literature
- Order flow has LONG MEMORY: trade-sign autocorrelation decays as a power law
  (Lillo-Mike-Farmer; Bouchaud "How markets slowly digest...", arXiv:0809.0822).
  High autocorrelation => the push is more likely to CONTINUE.
- Impact is TRANSIENT, not permanent: the propagator / Transient Impact Model
  (Bouchaud et al., arXiv:1602.02735) writes price as past order signs weighted by
  a DECAYING kernel. The "double square-root" result (Maitrier, Loeper, Kanazawa
  & Bouchaud, arXiv:2502.16246) gives the decay shape: impact decays ~ 1/sqrt(t)
  in time after the sqrt-in-volume build-up. No-arbitrage forces permanent impact
  to be linear and small (Gatheral), so most of a push's move is transient and
  partially REVERTS (~1/3 of peak relaxes away).

## What this means for us
- An OPTIMAL HOLD exists: hold long enough to capture the transient move, exit
  BEFORE the 1/sqrt(t) decay gives back the reverting ~1/3.
- Set hold-time from MEASURED persistence: high trade-sign autocorrelation ->
  longer hold; low -> shorter. (This is exactly T5 in research-flow-primitives.)

================================================================================
RANKING -- most predictive vs cheapest (the Q2 answer)
================================================================================
| Tool                         | Predicts     | Predictiveness | Compute | Have? |
|------------------------------|--------------|----------------|---------|-------|
| OFI (depth-normalised)       | SIZE (short) | high (<1 min)  | trivial | yes   |
| sqrt-law on signed volume    | SIZE (large) | high (big push)| cheap   | yes   |
| Trade-flow imbalance         | SIZE (contemp)| high          | trivial | yes   |
| MLOFI / integrated           | SIZE         | marginal +     | cheap   | yes   |
| Trade-sign autocorrelation   | PERSISTENCE  | high (the hold)| trivial | yes   |
| Hawkes intensity             | both         | duplicates above| HIGH   | yes   |

Verdict: PURSUE. The winning combo is CHEAP and uses data we already have:
  - depth-normalised OFI for short-horizon size + direction,
  - sqrt-law on cumulated signed volume for large-push size,
  - trade-sign autocorrelation for the hold.
Hawkes stays PARK (heavy, duplicates the above) -- consistent with Q5.

================================================================================
DELIVERABLE -- an impact-calibration layer the bots consume
================================================================================
This is mostly a MEASUREMENT/labelling layer, buildable now (no new data). It
calibrates three numbers on OUR captured Parquet and exposes them to bots:
- beta  : the linear OFI->bps slope (and its ~1/depth scaling).
- Y      : the sqrt-law prefactor for predicted move = Y * sigma * sqrt(Q/V).
- G(t)  : the impact-decay kernel (fit the 1/sqrt(t) relaxation + permanent ~2/3).

Bots then use them as:
- ENTRY FILTER: predicted move (bps) > 11 bps + margin, else skip. Kills the
  fee-bleed trades up front.
- EXIT TIMING: hold to the kernel's peak-capture point, exit before reversion.

## Experiment / build tasks
- Q2-A (analysis, no new data): on the 05:00-12:00 window, regress forward move
  (multiple horizons) on depth-normalised OFI to estimate beta and its stability;
  fit the sqrt-law prefactor Y on reconstructed signed-volume bursts; fit the
  decay kernel G(t) (peak time, % reverted). Output a calibration artifact.
- Q2-B: add a `predictedMoveBps` filter to the OFI bot entry (uses beta + sqrt
  blend). Sweep the margin above 11 bps. Success bar: improves net-per-trade and
  hit-rate-of-clearing-cost on OOS vs no filter.
- Q2-C: kernel-timed exit -- exit at estimated peak-capture instead of a fixed
  timer; compare vs best fixed hold and vs T5 adaptive hold on OOS.

## Caveats
- The linear (OFI) and sqrt (metaorder) regimes are DIFFERENT; pick by
  horizon/size, do not force one model across both.
- Crypto LOB is noisy; calibrations are window-dependent. Our ~7h window is THIN
  -- treat beta/Y/G(t) as provisional until more capture lands (the standing
  data dependency).
- sqrt-law needs cumulated signed volume over the push; our forceOrder gap does
  not affect this (trades suffice), but very large cascade moves may sit in a
  different regime than ordinary metaorders.