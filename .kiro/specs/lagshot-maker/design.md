# Design Document: lagshot-maker

## Overview

This feature builds a MAKER (resting-limit-order) variant of the validated Lagshot basis-reversion edge, inside the `forgelag` crate. The taker version passed every backtest gate but proved uncapturable live: Hyperliquid's ~880ms consensus-floor latency means a taker fills after the gap has already reverted. A maker does not race to take - it rests and gets crossed to - so entry latency stops being the binding constraint and fees drop. The maker's enemy is instead ADVERSE SELECTION: a resting order tends to fill exactly when the market is about to move against it. A naive maker (rest at touch) already lost hard (t=-8..-11, win ~10%). So the whole job is solving WHERE to rest (around an OKX-anchored fair value, not at touch) and WHEN to pull/reprice (before a widening move runs the quote over).

The single most important deliverable is an HONEST maker fill model. The prior project lied via idealized fills. The `forgelag` engine already contains the conservative core of an honest model (`Resting.queue_ahead` + `process_trade` fills only on a through-trade after FIFO queue consumption, via `forge-sim::maker_fill`/`Account::apply_maker`). This design extends that with a cancel/reprice path (with its own Cancel_Latency) and the strategy layer that drives it, WITHOUT touching the sacred core crates.

### What already exists (reused, not rebuilt)
- `forgelag::engine::LagEngine` - event-driven, single-thread, virtual-clock sim with two-clock latency (`order_latency_ns`, `set_latency_samples`/`next_latency`), a `pending` min-heap delivering orders at `submit + latency`, and `Resting` maker orders with `queue_ahead`.
- `process_trade` - the honest maker fill: a resting order fills only when an HL trade prints THROUGH its price, after the trade volume first consumes `queue_ahead` (FIFO); residual fills the order; partial fills supported; never fills at mid; uses `forge-sim::maker_fill` + `Account::apply_maker` with the `FeeSchedule`.
- `forgelag::strategy::BasisSignal` - already computes the HL microprice, the basis gap vs the reference (`cur_gap`), the rolling baseline, and the deviation `cur_dev`; this is the fair-value/dev machinery the oracle reuses.
- `forgelag::feed` - loads HL book deltas + HL trades (needed to fill makers) + OKX reference trades + funding, merged on a deterministic time-ordered stream. The HL-trades-into-the-exec-stream wiring the maker model depends on is already present.
- `forge-sim` Account/FeeSchedule/fill math, `forge-book::OrderBook`, `forge-core` fixed-point + time. (Sacred - reused, never modified.)

### What is new (all in `crates/forgelag/`)
1. A cancel/reprice path in the forgelag sandbox engine (order IDs, a `Cancel` action, a configurable Cancel_Latency).
2. `FairValueOracle` - OKX reference + rolling basis baseline -> fair value, with a staleness guard.
3. `QuoteManager` - decides quote price (offset from fair value, reversion side), keep/reprice/pull, emergency flatten on pull-fail.
4. `InventoryController` - signed position cap + skew + suppression.
5. `MakerQuoter` strategy - the state machine that ties these together, driven by exchange-truth position (mirrors the live bot's design).
6. Maker metrics in `LagReport` (fill rate, max abs inventory, max drawdown) + hunt CLI knobs.
7. A maker null-edge gate test + fill-model unit tests + a determinism/replay test.

## Architecture

### Data flow (one event)
```
LagEvent (HL book delta | HL trade | OKX ref trade | funding)
   |
   v
LagEngine.step(ev):
   1. advance virtual clock to ev.local_ts (reject non-monotonic -> Req 9.1/9.2)
   2. drain_pending(now): deliver Place/Cancel actions whose arrival <= now
        - Place  -> rest in book if non-marketable (else reject; a maker never crosses)
        - Cancel -> remove the resting order with that id (no-op if already filled/gone)
   3. apply the event:
        - HL BookDelta -> OrderBook.apply
        - HL Trade     -> process_trade(): fill resting makers the tape ran through (HONEST fill, Req 6)
        - OKX Trade    -> update ref_px
        - Funding      -> update funding
   4. build LagCtx (now, exec_book, ref_px, position_qty, ...) - read-only, <= now (no lookahead, Req 9.3)
   5. strat.on_event(ctx, &mut actions)   // MakerQuoter decides
   6. enqueue each action into pending at now + latency(Place=order_latency, Cancel=cancel_latency)
```

### Component boundaries
```
MakerQuoter (LagStrategy)
  |- FairValueOracle   : ref_px + rolling basis baseline -> fv, dev, staleness
  |- QuoteManager      : fv + dev + current resting quote -> Place / Cancel / keep
  |- InventoryController: position_qty + cap + skew -> allow/suppress, skew offset
  -> emits Vec<LagAction>  (Place{id,..} | Cancel{id} | Market{..} for exits/flatten)

LagEngine (forgelag sandbox - extended)
  |- pending heap (order + cancel latency)   |- OrderBook (HL)   |- Resting[] (+id)
  |- process_trade (HONEST maker fill)        |- Account (forge-sim)  |- metrics counters
```

The MakerQuoter implements the existing `LagStrategy` trait (`on_event(&LagCtx, &mut Vec<LagAction>)`). It is a peer of the existing taker `Managed` shell, not a modification of it.

## Components and Interfaces

### FairValueOracle (Req 2)
Computes where HL is about to trade, from OKX + the running basis. Reuses `BasisSignal`'s sampling math (gap = `(micro - ref_px)/ref_px * 1e4`; baseline = rolling mean over `window`; `dev = gap - baseline`).

State: rolling ring buffer of gap samples (mean), last ref update timestamp, last good fair value.

Interface (conceptual):
```
fn observe(&mut self, ctx: &LagCtx)            // sample gap on cadence; track last_ref_ts
fn fair_value(&self) -> Option<f64>            // ref_px adjusted by baseline; None if no ref yet (Req 2.4)
fn dev_bps(&self) -> Option<f64>               // current deviation in bps
fn is_stale(&self, now: u64) -> bool           // now - last_ref_ts > staleness_ns (Req 2.5)
```
- Fair value = `ref_px * (1 + baseline_bps/1e4)` (the price HL is expected to revert toward). (Req 2.1)
- Updated only from data with `ts <= now` (Req 2.2, 9.3).
- If stale (default 1000ms): oracle reports stale; QuoteManager pulls all quotes and places none, last fv frozen (Req 2.5). This directly encodes the live lesson where a frozen feed must not trade.

### QuoteManager (Req 1, 3)
Owns the single live entry quote and its lifecycle.

State: `Option<RestingQuote { id, side, price }>`, reprice/danger/cancel-latency config.

Decision each event (flat, not stale, have fv):
- Target quote price = `fair_value` offset by `quote_offset_bps` on the REVERSION side (bid when HL below fv, ask when HL above), plus the inventory skew (below). (Req 1.2, 2.3)
- If `|dev| < entry_threshold`: no entry quote wanted -> if one rests, Cancel it (Req 1.4).
- If `|dev| >= entry_threshold` and no quote rests: emit `Place{new id}` at target (Req 1.2); record id+price.
- If a quote rests and `|target - resting_price| <= reprice_tol`: keep it, emit nothing, do NOT duplicate (Req 1.3, 3.1).
- If a quote rests and fv moved beyond `reprice_tol`: emit `Cancel{id}` + `Place{new id}` at the new target (reprice = cancel-then-place within one cycle) (Req 3.2).
- If the basis WIDENS beyond `danger_threshold` in the run-over direction: emit `Cancel{id}` and place nothing (pull) (Req 3.3).
- Pull-fail / emergency flatten: if a Cancel was issued while in danger and the position is still non-zero after the cancel-latency + ack-timeout window (i.e., we got run over and filled before the cancel landed), emit a `Market` flatten of the resulting position (Req 3.7). Tracked by an in-flight cancel deadline.

Reprice/pull travel through the engine `pending` queue with `cancel_latency_ns`; during that in-flight window the order remains in `resting` and therefore remains fillable by `process_trade` (Req 3.5, 6.7).

### InventoryController (Req 4)
State: signed position read from `ctx.position_qty` (exchange truth), `cap`, `skew_bps`.
- Position update is implicit (Account tracks it); controller reads `ctx.position_qty`.
- If a new quote would push `|position|` strictly above `cap`: suppress that quote (Req 4.2, 4.7 when cap < one quote size).
- While holding inventory: shift the quote so the position-REDUCING side rests closer to mid and the increasing side rests further, offset growing monotonically with `|position|` (Req 4.3). At zero inventory: symmetric, no skew (Req 4.4).
- Reports `max_abs_inventory` (tracked in the engine each step). (Req 4.5)

### MakerQuoter state machine (driven by exchange-truth position)
Mirrors the live bot's exchange-truth design (we learned the hard way that tracking internal belief desyncs). The strategy acts on `ctx.position_qty`, not on an assumed fill.

```
on_event(ctx):
  oracle.observe(ctx)
  if oracle stale or no fv: cancel any resting quote; if pos != 0 keep managing exit; return
  pos = ctx.position_qty
  if pos == 0:                       // FLAT: run QuoteManager entry logic
      manage_entry_quote(ctx)        // place/keep/reprice/pull the single entry quote
  else:                              // IN POSITION (a quote filled)
      cancel any still-resting entry quote (we are filled)
      if |dev| <= exit_bps (reverted): emit Market flatten (taker exit, Req 5.2)
      else if danger/pull-fail: emit Market flatten (Req 3.7)
      // else hold; revert-to-mean exit
```
Exit is a TAKER market order (we want out now that the gap reverted; the entry latency race does not apply to exits, and taker-exit matches the validated revert-to-mean logic). Maker fee on the entry fill, taker fee on the market exit (Req 5.1, 5.2).
## The honest maker fill model (the prime gate, Req 6)

This is the component most able to LIE, so it is specified conservatively and matches what the engine already does, extended only for the cancel window.

### Queue-position model
- When a `Place` arrives and is NOT marketable, it rests. `queue_ahead` is set to the visible book quantity already resting at that exact price (`OrderBook.qty_at(side, price)`) at placement time. This is the honest assumption that we join the BACK of the queue at our level. (engine `place_limit`, today.)
- `queue_ahead` is only ever DECREMENTED, by trades that print through our price (FIFO). It is never refreshed downward when other orders cancel ahead of us (we cannot see individual cancels, and assuming queue improvements would be optimistic). This makes the model conservative by construction.

### Fill trigger (the only way a maker fills)
On an HL `Trade` event with aggressor side `aggr`, price `tprice`, qty `tqty`, for each resting order `o`:
- It is eligible only if the trade crosses our resting side: a resting Bid fills from Ask-aggressor trades at `tprice <= o.price`; a resting Ask fills from Bid-aggressor trades at `tprice >= o.price`. (Req 6.1; "prints through" = trade at or beyond our price in the executing direction.)
- The trade volume first consumes `queue_ahead`: if `queue_ahead >= tqty`, decrement and NO fill (someone ahead of us got it). (Req 6.3)
- Only the residual `tqty - queue_ahead`, capped at `qty_remaining`, fills our order, at OUR resting price (never mid, never price-improved). (Req 6.2, 6.3)
- If residual < `qty_remaining`: partial fill, remainder keeps resting with `queue_ahead = 0`. (Req 6.4)
- If no trade prints through our price (the classic case: the basis snaps back without trading to our level), we DO NOT fill. (Req 6.5 - this is exactly the adverse-selection mechanism that killed the naive maker, now modeled honestly.)
- Adverse selection emerges naturally: we fill preferentially when the tape KEEPS trading through us (continuation / toxic flow), and miss the clean reversions. (Req 6.4)

### Cancel-latency window (new)
- A `Cancel{id}` is delivered through the `pending` heap at `decision_ts + cancel_latency_ns`. Until it arrives, the order stays in `resting` and remains fully fillable by `process_trade`. (Req 6.7, 3.5)
- When the cancel arrives, the resting order with that id is removed; if it already filled or partially filled, the cancel applies to the remainder (or is a no-op). (Req 6.8)
- Cancel latency is configurable (`cancel_latency_ns`, range 0-5000ms) and can later be driven by a measured live distribution, exactly as order latency already is via `set_latency_samples`. (Req 3.4, 12.5)

### Determinism (Req 6.9, 9.5)
All fills are a pure function of the (already deterministic, time-sorted) event stream + config. The only randomness is the latency RNG (xorshift `lat_rng`, fixed seed) and the coinflip seed - both seeded. Identical inputs -> byte-identical fills and report + identical determinism hash.

## Engine extensions (forgelag sandbox engine.rs - NOT sacred core, Req 11)

### Action type
Replace the place-only `LagOrder` usage with an action enum (kept in `forgelag::engine`):
```
pub enum LagAction {
    Market { side: Side, qty: Qty },          // taker (exits, flatten)
    Place  { id: u64, side: Side, price: Price, qty: Qty }, // resting maker
    Cancel { id: u64 },                        // pull a resting maker
}
```
`Resting` gains `id: u64`. The strategy assigns monotonically increasing client ids. (Backwards compat: the existing taker `Managed` shell keeps working by emitting `Market`/`Place` with a dummy id; or we keep `LagOrder` and add `Cancel` alongside - implementation detail for the task phase. The existing taker path and tests must stay green - Req 11.4.)

### Pending delivery
`drain_pending` handles all three:
- `Market` -> `price_market` (taker) as today.
- `Place`  -> `place_limit`; if marketable on arrival, REJECT (a maker never crosses; count `orders_rejected`) rather than silently take. This keeps "maker-only" honest.
- `Cancel` -> remove `resting` entry by id (no-op if absent).
Cancel uses `cancel_latency_ns`; Place/Market use `order_latency_ns` or the sampled distribution.

### Metrics added to LagReport (Req 10)
- `quotes_submitted` (Place count), `quotes_filled` (maker fills of entry quotes) -> reporter computes `fill_rate_pct = filled/submitted*100`, or 0 if none submitted (Req 10.2, 10.3).
- `max_abs_inventory` (raw qty) - tracked each step as `max(|acct.net_qty()|)` (Req 4.5, 10.4).
- `max_drawdown_pct` - tracked from the realized-equity curve (running peak of cumulative realized P&L vs trough), expressed as percent of the EUR500-scaled equity peak (Req 10.5).
- Net expectancy in percent reuses the existing `trip_returns` (per-trip return %) -> mean (Req 10.1).

## Data Models / Config (extend BasisConfig + LagConfig + hunt CLI)

New maker knobs (added to a maker-specific config, or `BasisConfig` + a `MakerConfig`):
```
quote_offset_bps : f64   // 0-100; rest this far from fair value on reversion side (Req 2.3)
entry_threshold  : f64   // 1-100 bps; |dev| to want a quote (Req 1.2)
exit_bps         : f64   // revert-to-mean taker exit (reuse existing exit_revert/exit_bps)
reprice_tol_bps  : f64   // 0.1-100; keep-vs-reprice band (Req 3.1/3.2/3.6)
danger_bps       : f64   // 1-500; widen-pull threshold (Req 3.3/3.6)
cancel_latency_ns: u64   // 0-5000ms; pull/reprice delay (Req 3.4)
ack_timeout_ns   : u64   // 0-5000ms; pull-fail -> emergency flatten (Req 3.7)
pos_cap          : i64   // raw qty inventory cap (Req 4.2)
inv_skew_bps     : f64   // >=0; skew per unit inventory (Req 4.3)
maker_fee/rebate : via FeeSchedule (Req 5)
staleness_ns     : u64   // ref staleness guard, default 1000ms (Req 2.5)
```
Config validation: out-of-range values are rejected at construction with an error naming the bad parameter (Req 3.6). Missing maker fee in the schedule -> fail-fast at first maker fill (Req 5.6).

`hunt.rs` gains `--maker`, `--quote-offset`, `--reprice-tol`, `--danger`, `--cancel-lat`, `--pos-cap`, `--inv-skew`, `--staleness` flags, mirroring how taker knobs are already exposed, so we can sweep WHERE/WHEN coarse-then-fine.

## Error Handling
- Non-monotonic `local_ts` -> halt + error, no report (existing `step` check; Req 9.2).
- Lookahead (strategy reading ts > now) -> impossible by construction (ctx only exposes <= now state); asserted in tests (Req 9.4).
- Bad data (NaN/Inf/neg-ts/overflow) -> fail-fast in feed (existing `ms_to_ns`, `Price::from_f64`).
- Config out of range -> reject at construction naming the param (Req 3.6).
- Missing maker fee -> halt at first maker fill (Req 5.6).
- Order arrival overflow -> error (existing `checked_add`).

## Testing Strategy

### Null-edge maker gate (Req 7) - the gate that must come first
`tests/null_edge_maker.rs`: a CoinSignal-style RANDOM-side maker (rest random side at the offset, same cadence) over both a synthetic stream and a real day. Assert net P&L strictly < 0 per stream over >= configured min round trips, after maker+taker fees and the honest fills. If it nets >= 0, the fill model is manufacturing edge -> fail the suite, block promotion. Seeded -> identical across runs.

### Fill-model unit tests (Req 6) - the anti-lie tests
- through-trade fills: a resting bid + an Ask-aggressor trade through it after queue cleared -> fills at resting price, not mid.
- queue-ahead protects: trade qty <= queue_ahead -> NO fill; queue decremented.
- partial fill: residual < qty_remaining -> partial, remainder rests with queue_ahead 0.
- clean reversion NO fill: dev stretches then the gap reverts with NO HL trade printing through the resting price -> order stays unfilled (the killer case).
- cancel-latency window: order pulled but an in-window through-trade still fills it; a post-window trade does not (Req 6.7/6.8).
- no price improvement / no mid fill ever.

### Behavior tests
- reprice keeps within tol; pull on danger; emergency flatten on pull-fail-while-in-danger (Req 3).
- inventory cap suppresses; skew leans against position; symmetric at flat (Req 4).
- staleness guard pulls + places nothing (Req 2.5).

### Determinism / replay (Req 9.5)
Run the same real day twice -> identical LagReport + determinism hash.

### Knob-bite (Req 8)
Sweep harness records the signal/quote sequence per sweep point; a "no edge" verdict is valid only if quotes/fills changed between adjacent points; report trade count + fill rate per point.

### Validation order (how we will actually use it)
1. clippy + null-edge maker gate green (engine not lying).
2. fill-model unit tests green.
3. coarse sweep of quote_offset x entry_threshold x reprice_tol x danger on one real day -> look for ANY net-positive-after-fees region with a believable fill rate.
4. if a pulse appears: fine sweep + multi-day + OOS + shuffle control + knob-bite + DSR/PBO.
5. only then: live cancel-latency measurement (tiny size) -> blended re-run -> decision (Req 12).

No live order until 1-4 pass. This is the same honest ladder that correctly killed the taker version.