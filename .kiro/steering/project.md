# ForgeOS — project guide

Always-on context. Read before acting.

## Team
- **Trader**: direction, priorities, final call on go/no-go. 10y discretionary
  macrostructure trader. NOT a coder. Prefers 15min-4hr holds, orderflow-driven.
- **Copilot (GLM-5.1)**: engine architecture, Rust code, honesty gates, null-edge
  validation, documentation. The skeptic.
- **DeepSeek V4 Pro**: data analysis, feature prototyping, ML/statistical work,
  Python scripts, number crunching at scale.

## What this is
Clean-room Rust engine for crypto edge discovery. NEW DIRECTION (2026-06-15):
macrostructure depth-pattern study (15min-4hr holds, orderflow-driven entries),
NOT tick-level microstructure racing. The old Lagshot/sweep/OI studies are CLOSED
(edge real but uncapturable at retail latency/fees). The binding constraint was
~9bps taker fee vs ~5bps microstructure edge. New leads must clear >>9bps per trade.

Successor to the archived `wall-bot-tournament`, whose TS engine produced
impossible results (100% win, +edge, 0 DD) from lookahead/accounting rot.

## The one rule above all
NO number is trusted until the engine passes the NULL-EDGE TEST: a seeded
coinflip strategy must net ~ -(fees + spread) over large N, never positive edge.
It is a CI gate (roadmap Phase 2). If a coinflip "wins", stop and fix the engine.

## Principles (see docs/engine-design.md)
- No-lookahead by construction: two-clock model (exch_ts + feed/order latency).
- Deterministic, event-driven; one sim = one thread; clock = event.local_ts.
- Market-data replay: orders never move the market; fills model queue position +
  adverse selection (no mid fills, ever).
- Fail-fast on bad data (NaN/Inf/neg-ts/overflow).
- Backtest == live core; parity via golden vectors.

## Layout
- `crates/` Rust workspace (forge-core, -data, -book, -sim, -strategy, -metrics,
  -sweep, -depth). Build order + gates in `docs/roadmap.md`.
- `crates/forge-depth/` NEW: depth-pattern feature computation (full L2 book shape,
  CVD, volume profile, wall tracking, multi-timeframe aggregation).
- `tools/` data preprocessing (cryptohftdata -> normalized *.forge event stream).
- `docs/engine-design.md`, `docs/roadmap.md`, `docs/migration-from-wallbot.md`.
- `docs/research/` preserved research. `docs/legacy/` read-only business-rule ref
  (NEVER port code from it; re-derive + test).

## Data
- Signal venue Binance (DOM+trades), execution Hyperliquid quotes. Feed =
  cryptohftdata (byte-verified). Cold = parquet; hot replay = mmap'd packed
  `*.forge` event stream (zero-copy). Clean feed lives on the box at
  /root/chd/data/ticks (BTCUSDT bookDelta+trade, BTC hlquote; 2025-12-01,
  2026-04-01, 2026-06-07 to start).

## Commands (Rust)
- Build: `cargo build --release`   Test: `cargo test`   Lint: `cargo clippy`
- Sweeps/backtests run on the Hetzner box inside tmux (see environment.md).

## Hard rules
- Everything that executes is clean-room, deterministic, tested, and null-edge
  gated. External repos (nautilus_trader, hftbacktest) are REFERENCE ONLY -
  re-implement in our own primitives, never copy/run.
- Commit small; any change touching fills/P&L carries a test that catches its
  failure mode. A "no edge" verdict is valid only if the swept param moved trade
  behavior (knob-bite rule).