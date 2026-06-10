# ForgeOS — Engine Design (v0)

## 0. Why this exists (the lesson)
The prior TS engine (`wall-bot-tournament`) showed BTC wall bots at 85-97% win
and +60..+140 bps edge AFTER fees on clean data — and the numbers got BETTER
when we made fills more adversarial. That is mathematically impossible and is the
signature of lookahead and/or P&L-accounting corruption, plus an idealized fill
model (entry on the HL touch, exit on the raw mid — the exit-side spread and
cross-venue gap silently skipped). The defect was buried under layers of
overlapping additive changes and could not be trusted out.

ForgeOS replaces the *executing* core clean-room. We keep only non-executing
assets (data feed, converter, research, methodology, business-rule specs).

## 1. Non-negotiable invariants
1. **Null-edge guardrail (the lie-detector).** A random/coinflip strategy MUST
   net approximately `-(round-trip fees + spread)` over large N, with no positive
   edge and a realistic loss distribution. Shipped as a CI test. No metric from
   the engine is believed until this passes and stays passing.
2. **No-lookahead by construction.** Two timestamps per event (hftbacktest model):
   `exch_ts` (when it happened at the venue) and `local_ts` (when our strategy
   could first SEE it = exch_ts + feed_latency). A decision made at local time T
   may read only data with `local_ts <= T`. An order submitted at T reaches the
   matching engine at `T + order_latency`. Latencies are explicit, configurable,
   and part of the determinism contract.
3. **Deterministic event-driven core.** One simulation = one thread. The virtual
   clock is the event stream's time; `now() == current_event.local_ts`. No
   wall-clock, no RNG (except an explicitly-seeded PRNG for the null-edge bot),
   no I/O inside the hot loop.
4. **Market-data replay, honest fills.** Our orders never move the replayed
   market (no impact). But fills DO model queue position + adverse selection:
   a resting maker post fills only once the tape trades through our price AND
   clears the volume resting ahead of us; a taker crosses the spread + slippage.
   No fill at the mid, ever.
5. **Fail-fast on bad data.** NaN/Inf prices, negative or non-monotonic
   timestamps, overflow -> panic in debug/test, hard error at ingest. Corrupt
   data is worse than no data (nautilus principle).
6. **Backtest == live core.** The matching/fill/accounting code is environment-
   agnostic. Live and backtest differ only at the edges (data source, clock,
   venue adapter). Parity is enforced by GOLDEN VECTORS: a fixed input stream
   must produce a byte-identical trade list + P&L in both.

## 2. Architecture (crate layers, dependencies point downward)
```
forge-sweep      (rayon combo x window parallelism over shared mmap)
forge-metrics    (metrics + PBO/CPCV/DSR + null-edge harness)
forge-strategy   (Strategy trait; features/primitives: OFI/CVD/VPIN, re-derived)
forge-sim        (virtual clock, two-clock latency, matching engine, fills, P&L)
forge-book       (L2 reconstruction: deltas + periodic snapshots, deterministic)
forge-data       (normalized event stream: writer + zero-copy mmap reader)
forge-core       (UnixNanos, Price, Qty, Side, Event, errors; fail-fast types)
```
- **forge-core**: fixed-point `Price`/`Qty` (no float P&L), `UnixNanos`, the
  `Event` enum (Trade / BookDelta / BookSnapshot / Quote), checked arithmetic.
- **forge-data**: the hot path (see section 3).
- **forge-book**: pure fold of deltas onto snapshots -> top-N book view; desync
  detection + resync; identical to live book maintenance.
- **forge-sim**: the SimExchange. Owns the matching engine, the two latency
  queues (inbound data, outbound orders), queue-position fill model, fee
  schedule, positions, and P&L. This is the component the old engine got wrong;
  it is the most heavily tested.
- **forge-strategy**: `trait Strategy { fn on_event(&mut self, ctx) -> Vec<Intent> }`
  pure over (event, book_view, ctx). Features/primitives are pure functions of
  the same inputs, RE-DERIVED from `docs/research/`, not ported from TS.
- **forge-metrics / forge-sweep**: see sections 5 and 4.

## 3. Data format (hot path)
- **Cold / interchange:** keep cryptohftdata parquet (+ `tools/chd-to-parquet.py`).
  Language-agnostic, inspectable, byte-verified against Binance official data.
- **Hot replay format:** preprocess ONCE into a normalized, time-sorted,
  fixed-width **packed binary event stream** (`*.forge`). Each record is a
  `#[repr(C)]` POD struct (exch_ts:u64, local_ts:u64, kind:u8, side:u8,
  price:i64 fixed-point, qty:i64 fixed-point, flags...). The engine `mmap`s the
  file and reinterprets the bytes as `&[EventRecord]` via `bytemuck`/`zerocopy`:
  **zero decode, zero allocation, pure sequential scan** — eliminating the
  Parquet-decode cost that crippled the TS harness. Arrow IPC/Feather is the
  self-describing fallback for debugging.
- Preprocessing (Python or Rust) merges Binance DOM+trades + HL quotes into one
  ts-sorted stream, assigns `local_ts = exch_ts + feed_latency`, and writes the
  packed file. Deterministic and cached.

## 4. Parallel sweeps
- The `*.forge` stream is `mmap`'d READ-ONLY, so the OS page cache serves the
  same bytes to every worker thread with zero copy — no per-process re-read /
  re-decode (the TS fork model's main waste).
- `rayon` fans the grid across `(combo x window)`: each task is an independent,
  single-threaded, deterministic sim with its own state, reading the shared
  `&[EventRecord]`. Near-linear scaling on Hetzner cores; rent a bigger box for
  occasional brute force.
- **Shared feature frame (optional, big win on trade-management grids):** compute
  the primitive/feature series ONCE per window (immutable, shared), then fan out
  only the cheap execution shell per combo. Faithful because each shell still
  runs the real capacity/cooldown gates over the shared signal.
- Determinism: per-sim single-threaded; results aggregated by sorted combo id ->
  identical artifact regardless of thread scheduling.

## 5. Validation (re-coded fresh, pure math)
- Per-config: net edge after fees, Deflated Sharpe (deflated by trial count),
  in-sample/out-of-sample split, % windows positive.
- Across the sweep: **PBO** (CSCV) + trial count, and **CPCV** (combinatorial
  purged CV with embargo). Same methodology as the prior project, re-implemented
  in Rust (each is ~100 lines, pure, property-tested).
- **Null-edge harness** gates everything (invariant #1).
- Knob-bite rule retained: a "no edge" verdict is only valid if the swept
  parameter actually moved trade behavior (trade counts vary across cells).

## 6. Determinism contract (every strategy/primitive)
Pure over `(event, book_view, ctx)`; time is `event.local_ts` only; state is
explicit and serializable; no clock, no I/O, no unseeded RNG. Two runs over the
same `*.forge` stream + config produce identical trades, P&L, and a matching
determinism hash.

## 7. Live (deferred until a validated edge exists)
A venue adapter feeds real events into the SAME sim/strategy core with a live
clock; orders route to the exchange instead of the SimExchange. Parity to
backtest is checked by replaying the live session's captured stream through the
backtest and diffing trades (golden vectors). No live work begins before an edge
clears net-of-fees + PBO/CPCV on out-of-sample data.

## 8. References (untrusted; design inspiration only, never copied/run)
- nautilus_trader: deterministic single-threaded event core, research↔live
  parity, fail-fast data integrity, crash-only recovery.
- hftbacktest: two-clock feed/order latency, queue-position fill models,
  market-data-replay (no impact), L2 reconstruction.