# Research Note -- Q1 / Q3 / Q4 / Q6 (2026-06-09)

Closes out the charter (see `docs/research-charter.md`). Same ground rules:
additive/flag-gated, backtest before live, every bar net of ~11 bps.

================================================================================
Q1 -- BROAD SURVEY: which order-flow families survive RETAIL cost+latency?
================================================================================
The north-star question. At retail latency (100ms-seconds) and ~11 bps
round-trip, edge CANNOT come from speed -- it must come from PREDICTING A MOVE
BIGGER THAN COST, then capturing it. That single principle sorts everything:

SURVIVE (predict-a-big-enough-move, or hold the push):
- OFI direction + persistence (depth-normalised), ride-the-push not scalp.
- Square-root SIZE on large signed-volume pushes (Donier & Bonart on BTC,
  arXiv:1412.4503) -> only arm when predicted move > cost (Q2).
- Absorption / CVD-divergence REVERSALS (fade exhaustion).
- Volume-Profile LVN->HVN travel and value-edge rejection.
- Liquidity-grab / stop-run fades and liquidation cascade-fades (big moves).

HFT-ONLY / COST-KILLED (drop at retail):
- Queue-imbalance one-tick prediction; microstructure spread scalping.
- Naive cross-venue latency/taker-lag chase.
- Reacting-to-spoof front-running; flickering-quote games.

Why: the LOB-mechanics study (arXiv:2502.18625) shows the taker fee is THE
hurdle and maker fills are adversely selected; long-memory (Lillo-Farmer) +
square-root impact say the durable money is in the persistent push, not the tick.
Verdict: this is already the lab's operating thesis -- the ledger reflects it.
"Ride the push / fade the exhaustion", never "win the race".

================================================================================
Q3 -- LIQUIDITY TRAPS: stop-runs / liquidity grabs / sweep-and-reverse
================================================================================
## Mechanism + literature
Stops cluster at OBVIOUS levels (round numbers, swing highs/lows, prior-day
H/L). Large players push price INTO the cluster to harvest that liquidity, then
reverse. Strongest academic support: Osler, "Stop-Loss Orders and Price Cascades
in Currency Markets" -- stop propagation creates price cascades, helps explain
fat tails, and shows prices respond to NON-informative order flow (i.e. the move
is mechanical, then reverts). Practitioner literature (ICT/"smart money") frames
the same WATCH->SWEEP->REVERSE pattern.

## What we already have
`liquidity-sweep-primitive.ts` (SW1) implements it precisely:
WATCH (strong wall persists) -> PULLED (size collapses with little volume = it
was CANCELLED not eaten = the tell) -> SWEEP (one-sided burst through the price)
-> EXHAUST (burst decelerates) -> FADE back to the wall's old price. Uses the
execution-vs-cancellation split.

## Gap vs the literature (upgrades)
1. Locate clusters the way the theory says: add SWING-PIVOT, ROUND-NUMBER, and
   prior-session H/L maps as the "WHERE", not only resting walls.
2. Require RECOVERY-INSIDE confirmation (price spikes through then closes back
   beyond the level) -- the canonical sweep signature, reduces false fades.
3. Confirm the reversal with absorption / volume-reversal at the extreme.
4. Same structure as liquidation cascade-fade -> once forceOrder capture lands
   (L0), a real liquidation burst is a stronger SWEEP trigger than inference.

## Verdict: PURSUE (primitive exists, data ok)
Sweep the trade-management hard (entry strength, R:R, hold, cooldown -- the
project's hard lesson). Target (return to value) is usually a move that can clear
11 bps. Success bar: net > 0 after 11 bps, DSR>0, OOS sign-consistent.
================================================================================
Q4 -- SPOOFING: detect from L2 deltas; tradeable, or only a filter?
================================================================================
## Mechanism + literature
Spoofing/layering = posting size with no intent to execute to push price, then
cancelling; plus flickering quotes. Detectable signatures in L2 deltas: high
place-then-cancel rate, large orders away from the touch that vanish before
execution, reload-vs-cancel asymmetry, and short order lifetimes. Methods:
multi-scale Hawkes order-flow variables that account for order SIZE and PLACEMENT
DISTANCE ("Learning the Spoofability of Limit Order Books...", arXiv:2504.15908,
crypto CEX focus); event-level impact models (Bouchaud et al., arXiv:1107.3364).
Regulatory/legal note: spoofing is defined by INTENT, which data alone rarely
proves -- so "detection" is really "likelihood the wall is non-bona-fide".

## What we already have
We capture `bookDelta` (top-15/side, each update pre-classified add/cancel/
execute) -- exactly the L2 delta stream this needs. Raw material is in hand.

## Verdict: PARK as a standalone strategy; PURSUE as a FILTER (high value)
- Standalone: NO. Reacting to a spoof and front-running the real move is a
  sub-100ms speed game -> HFT-only, cost-killed for us.
- As a FILTER: YES, and it targets the project's #1 open problem. A spoof-
  likelihood score on a wall is precisely a better REAL-vs-FAKE wall classifier
  (NOTES.md open-thread #1: REAL is emitted <1% because executed/cancelled
  attribution is corrupted). A "this wall is likely spoofed/cancel-bound" signal:
  (a) stops bots trusting a fake wall as support/resistance, and
  (b) gates out manipulation regimes (flicker/layering bursts) as no-trade.
This is the most useful Q4 outcome: spoof-detection = sharper FAKE classification
= unlocks v4/lv4/lv5 (the real-only configs that never trade). Cheap; reuses
bookDelta. Validate against the trade-aware classifier in `pnpm backtest`.

================================================================================
Q6 -- CROSS-VENUE LAG (Binance -> Hyperliquid)
================================================================================
## What the literature says
Binance is the dominant BTC price-discovery venue: ~70% of price transmission
starts on Binance, leading even semi-regulated venues like Coinbase
("Where is the Price of Bitcoin Determined?", price-discovery studies). Recent
cross-platform measurement puts Binance ~700ms AHEAD of Hyperliquid on perps;
arXiv:2506.08718 finds centralized markets lead crypto price discovery, with
high-volatility periods mixed. Our own measured ~1s Binance->HL lag is consistent
and REAL -- it is not the existence of the lag that fails, it is monetising it.

## Why the naive chase dies (confirmed)
Taker-chasing HL after a Binance move pays ~11 bps to capture a small lagged
move => net negative, EXCEPT when the followed move is large. Cross-exchange arb
at retail lives at the ~100ms native tick; finer just forward-fills stale prices.

## What we already have
`lead-lag.ts` causally estimates the Binance->HL lead and converts the recent
Binance move into a PREDICTED HL move (with a lead gate + threshold). So the
surviving variant is mostly wiring.

## Verdicts (three variants)
1. Naive taker chase: DROP (cost-killed, confirmed).
2. BIG-MOVE-ONLY chase: PURSUE-conditional, NO new data. Gate the lead-lag
   signal by predicted-move-bps > 11 bps + margin (the Q2 size estimator). Only
   chase when the anticipated HL move dwarfs fees. Sweep the threshold + hold.
3. Multi-venue OFI CONFLUENCE (Binance + Bybit/OKX agree): PARK -- needs new
   capture (Bybit/OKX via Tardis or native). Higher-conviction directional prior.
4. Maker capture on HL ahead of the move: PARK -- needs HL queue position (we
   lack it) and must model maker adverse selection (arXiv:2502.18625).

Success bar (variant 2): beats no-chase on net-per-trade OOS; only fires on moves
empirically clearing 11 bps.

================================================================================
STATUS: charter complete. Q1,Q3,Q4,Q6 closed here; Q5 + Q2 + Volume-Profile/
Liquidations in their own notes. See research-README.md for the full ledger.