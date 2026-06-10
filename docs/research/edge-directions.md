# Edge Directions - approach upgrade (READ ME when picking the next thesis)

Why this note exists: the early theses (OFI, book-imbalance, CVD) are PUBLIC
indicators used plainly. On liquid BTC they are arbitraged to ~zero - our sweeps
confirm it (0 promote, edge ~ -fees). Lesson: the edge is almost never the
indicator. It is one of three things, none of which is "the signal":
  A. CONTEXT - a weak signal that only works in a specific situation (regime /
     book state). Conditioning, not the raw signal.
  B. HARD-TO-COMPUTE - signals nobody ships as a one-click indicator because
     they need stream-matching or messy data. Difficulty is the moat.
  C. FORCED FLOW - someone who MUST trade (liquidations, funding, basis, stops).
     Mechanics, not predictions; harder to arbitrage because a party is forced
     to be the dumb side.

Our setup (be honest about it): retail infra, small account (~EUR500), no colo,
execution on Hyperliquid, signal data from Binance. We CANNOT win pure latency
races vs HFT. So we hunt structural / behavioural / hard-to-replicate edges,
not speed.

## Hit-list, ranked by realism for OUR setup
1. REAL-vs-FAKE WALL (spoof/pull vs eaten) - type B. Already designed
   (docs/research/real-vs-fake-wall.md). Not a platform indicator; needs
   trade-tape vs book-diff matching. Best first bet. BUILT, pending sweep.
2. LIQUIDATION CASCADES - type C. Forced flow: stops/liqs fire in clusters and
   overshoot, then snap back. Needs a liquidation feed OR inference (violent
   move + funding extreme + thin book). High edge potential, medium data lift.
3. FUNDING / BASIS (perp vs spot) - type C. When funding is extreme, perps are
   crowded one way and mean-revert / get squeezed. Needs a spot feed alongside
   the perp. Structural, slow-horizon, scalable, low-toxicity.
4. STOP-RUN / LIQUIDITY-SWEEP (SW1) - type B+C. Wall pulled -> sweep runs the
   stops -> exhaust -> fade. Thin layer on top of real-vs-fake wall.
5. CONDITIONAL COMBINATIONS - type A. Take the "dead" public signals and only
   fire them in the regime/book-state where they are not dead. Cheap to try
   with what we already have (regime layer exists). Squeeze before discard.
6. CROSS-VENUE LEAD-LAG (Binance -> Hyperliquid) - type C-ish. Real only if we
   can act inside the lag AFTER modelling true latency. HONEST: probably
   marginal for retail; model the latency first, do not assume we win it.
7. ORDER-FLOW TOXICITY / ADVERSE SELECTION - meta. Measure how often our fills
   are immediately wrong; use as a FILTER on any of the above, not a thesis.

## How this changes the workflow
- Stop sweeping public indicators used plainly (done: OFI/wall/CVD = no edge).
- Prefer type B + C theses; use type A (conditioning) to revive weak signals.
- Each new thesis still goes through the SAME gates: shuffled control, null-edge,
  knob-bite, DSR/PBO, then the EUR500 paper gate. The engine is the asset; we
  only change WHAT we feed it.
- Data we will likely need next: a spot feed (funding/basis) and/or a liquidation
  feed (cascades). Note for when we pull more data.

## Order of attack (current decision)
1. Finish testing all CURRENT bots (OFI/wall/CVD hunt + sweep the wall-flow bot).
2. Then pivot to type B/C: real-vs-fake wall sweep -> liquidation cascades or
   funding/basis (whichever data is easiest to add).
3. Revisit this note whenever picking the next thesis; upgrade the ranking as we
   learn what the data says.