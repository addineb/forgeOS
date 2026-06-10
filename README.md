# ForgeOS

Clean-room, Rust-native research/backtest engine for crypto microstructure edge
discovery. Successor to `wall-bot-tournament` (frozen) after its TS engine was
found to produce impossible results (100% win, positive edge, zero drawdown on
clean data even with realistic fills) — a lookahead/accounting defect that made
every verdict untrustworthy.

ForgeOS is a from-scratch rebuild with one rule above all others:

> **No number is trusted until the engine passes the null-edge test** — a
> coinflip/random strategy MUST net approximately negative-fees over a large
> sample. If a coinflip ever "wins", the engine is lying and we stop.

## Principles
- No-lookahead by construction (two-clock latency model: feed latency + order
  latency, hftbacktest-style).
- Deterministic, event-driven, single sim = single thread, virtual clock = event
  ts (nautilus-style research↔live parity).
- Market-data replay: our orders never move the market; fills model queue
  position + adverse selection, never idealized mid fills.
- Fail-fast on bad data (NaN / negative ts / overflow).
- Backtest and live share a strategy spec; parity enforced by golden vectors.

## Layout
- `crates/` — the Rust engine workspace (see docs/roadmap.md for build order).
- `tools/` — data preprocessing (cryptohftdata -> normalized event stream).
- `docs/engine-design.md` — architecture + the no-lookahead/fill/parallel design.
- `docs/roadmap.md` — phased build plan with per-phase gates.
- `docs/migration-from-wallbot.md` — what was kept, referenced, and dropped.
- `docs/research/` — preserved research/findings from the prior project.
- `docs/legacy/` — read-only business-rule + workflow reference (do NOT port code).

## Run target
All sweeps/backtests run on the Hetzner box (100 GB), launched inside `tmux`
sessions teed to logfiles. Railway is retired.