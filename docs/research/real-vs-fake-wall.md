# Research: Real-vs-Fake Wall + Liquidity Sweep

Status: DESIGN (build-ready). Engine untouched; this is a new EntrySignal that
plugs into the shared ExecutionShell like every other bot.

## The idea (plain terms)
A big resting order (a "wall") can disappear two opposite ways, and our old
engine could not tell them apart:
- EATEN: aggressive orders traded through it. Real liquidity got absorbed.
  Often continuation - the side that ate it has conviction.
- PULLED: the owner cancelled it, no trades. A spoof / fake. The level it was
  "defending" tends to give way or snap back - it was never real support.

We can finally separate these because the feed gives us BOTH the order book
(resting size per price) AND the trade tape (price, size, aggressor side).

## Why it is buildable (data check - done)
forge-core Event carries: kind (Trade | BookDelta | Quote), side, price, qty,
exch_ts/local_ts. So when a level's resting size drops, we can ask the trade
tape "how much actually traded at that price right then?" and split the drop
into executed vs cancelled. No new data needed.

## The attribution (the core primitive: WallFlow)
Maintain, per side at/near the touch:
- last seen (price, resting_qty);
- a running bucket of trade volume that hit that side's touch since the last
  book change (sells hitting the bid -> bid-side execution; buys lifting the ask
  -> ask-side execution).

On a BookDelta that REDUCES resting size at a touch price by D:
- executed = min(D, trade_volume_at_that_touch_since_last_change)
- cancelled = D - executed
- add to rolling-window tallies (executed_bid/ask, cancelled_bid/ask);
- consume the trade bucket by `executed`.

From the tallies over a window of book updates we get:
- cancel_ratio = cancelled / (cancelled + executed)  in [0, 1]
- which side is being pulled (cancelled_bid vs cancelled_ask).

## The entries (direction)
- Pulled support (bid cancelled heavily) -> the floor is fake -> bearish:
  enter Ask (price likely drops through the vacated bids).
- Pulled resistance (ask cancelled heavily) -> ceiling fake -> bullish: Bid.
- EATEN through -> real pressure -> lean with the aggressor (continuation).
A `mode` dial chooses spoof-fade vs absorption-follow so the data decides.

## Settings (dials) - all sweepable
- wall_min: how big a resting size counts as a "wall" (ignore noise).
- window: how many book updates the executed/cancelled tally rolls over.
- cancel_ratio_min: how one-sided the pulling must be to act.
- mode: spoof-fade (pull -> fade) vs absorption-follow (eaten -> follow).
- standard shell dials: size, hold_ns, cooldown_ns, tp/sl, regime_filter.

## Honesty controls (mandatory, same as every thesis)
- Shuffled-direction control: same trade cadence, random side -> must collapse
  to ~fees. Proves the edge is in the CLASSIFICATION, not the trade frequency.
- Null-edge gate stays green (coinflip loses).
- Knob-bite: a "no edge" verdict only counts if a dial actually moved trades.

## Liquidity sweep (SW1) - shares the same machinery
Sequence: a wall sits -> it gets PULLED as price approaches (WallFlow flags the
cancel) -> price sweeps the now-thin book and runs the stops -> exhausts ->
fades back toward the old wall price. So SW1 = WallFlow(pull detected) + a
short exhaustion/return entry. Build WallFlow first; SW1 is a thin layer on top.

## Build order
1. WallFlow primitive + unit tests (executed-vs-cancelled is deterministic).
2. WallFlowSignal + WallFlowBot on the shell; wire into the sweep grid.
3. Sweep across the 15 windows with the shuffled control; read regime split.
4. If a pulse survives DSR/PBO + knob-bite -> SW1 exhaustion layer -> paper gate.