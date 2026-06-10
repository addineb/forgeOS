# ForgeOS — Roadmap

Each phase has an explicit GATE that must pass before the next begins. The
null-edge harness (Phase 2 gate) is the hard line: no strategy work happens
until a coinflip provably loses ~fees in this engine.

## Phase 0 — Foundations
- Cargo workspace, CI (build + clippy + test), `forge-core` types (UnixNanos,
  fixed-point Price/Qty, Event, fail-fast arithmetic).
- `forge-data`: define the `*.forge` packed record; writer + zero-copy mmap
  reader; a converter step cryptohftdata parquet -> `*.forge` (reuse
  `tools/chd-to-parquet.py` knowledge).
- GATE: round-trip a known window parquet -> `*.forge` -> read back; event count,
  ts-monotonicity, and a checksum match. Loader does zero allocation in the loop.

## Phase 1 — Deterministic replay core
- `forge-book`: L2 reconstruction (deltas + periodic snapshots), desync/resync.
- `forge-sim` skeleton: virtual clock, event loop, two-clock latency queues.
- GATE: book state at sampled timestamps is correct; two runs over the same
  stream produce an identical determinism hash; a decision at T provably cannot
  read any event with local_ts > T (lookahead unit test).

## Phase 2 — SimExchange + honest fills  [THE GATE]
- Matching engine: maker post fills only after the tape clears the queue ahead;
  taker crosses spread + slippage; fee schedule; positions; fixed-point P&L.
- `forge-metrics` minimal: net edge after fees, P&L distribution.
- **GATE (non-negotiable): the null-edge harness.** A seeded coinflip strategy
  over real data nets ~ -(fees + spread), never positive edge, with a sane loss
  distribution. If it "wins", stop and fix the engine. Nothing proceeds until
  this is green and wired into CI.

## Phase 3 — Strategy layer + first thesis
- `Strategy` trait; ctx/book_view; capacity/cooldown gates.
- Re-derive ONE flow primitive (OFI or CVD) from `docs/research/`, pure + tested.
- Port ONE business thesis clean (a single wall/flow strategy) from the legacy
  spec — re-implemented, not copied.
- GATE: the strategy passes determinism + the null-edge harness still green;
  a deliberately shuffled-signal variant collapses to ~fees (no edge from
  structure alone).

## Phase 4 — Validation + sweep engine
- `forge-metrics`: Deflated Sharpe, IS/OOS, PBO (CSCV), CPCV (purged+embargo),
  all pure + property-tested.
- `forge-sweep`: rayon `(combo x window)` over shared mmap; optional shared
  feature frame; deterministic aggregation by combo id.
- GATE: a sweep reproduces identical artifacts across thread counts; PBO/CPCV
  numbers sane on a synthetic overfit matrix (re-use prior test fixtures).

## Phase 5 — Edge hunt (the actual goal)
- Run real sweeps on Hetzner inside tmux (detached, teed to logfiles).
- A bot is a candidate ONLY if: net positive after ~11 bps round-trip, DSR > 1,
  OOS > 0, PBO < 0.5, CPCV oosMean > 0, and the knob-bite rule holds.
- Honest reporting: candidates are few; most theses will die here. That is the
  point.

## Phase 6 — Live (only after a validated edge)
- Venue adapter feeds real events into the same sim/strategy core; live clock;
  orders route to the exchange.
- Parity via golden vectors: replay the captured live stream through backtest,
  diff trades. Start paper, tiny size, daily loss limit re-enabled.

## Operating rules
- Hetzner-only compute (100 GB), tmux sessions, logfiles. Railway retired.
- Additive, tested, deterministic. External repos are reference-only.
- Commit small; every executing change carries a test that would catch its
  failure mode (especially anything touching fills or P&L).