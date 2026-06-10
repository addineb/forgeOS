# Research Engine Map (2026-06-09)

One job per module. The CORE is faithful and tested — do not rebuild it. The
slop Dan fears lives in the orchestration/CLI layer and the dual bot-lifecycle.

## Core data path (faithful, tested — DO NOT rebuild)
- `parquet-source.ts` — merged ordered event stream from R2 Parquet (ts asc; tiebreak hlquote<depth<trade).
- `book-reconstructor.ts` — canonical L2 book from bookSnapshot/bookDelta (feature seam only).
- `harness.ts` — the driver. Runs the EXACT live BotEngine path; only the edges differ (virtual clock,
  injected HL quotes, mocked Redis, DB disabled). Seams (feature engine, primitive bots, regime labeler,
  signal recorder, fill mode) are OFF BY DEFAULT and proven byte-identical.
- `../bot/engine.ts` — BotEngine: the same code live and in replay. Wall path (live 11) and primitive path
  (research) share ONE open/close/P&L implementation (openTrade / checkAndClosePositions / closePosition / calcNetPnl).
- `fill-engine.ts` — market / postOnly-maker / cross-spread / stop fills off the captured tape.
  Default synthetic (live byte-identical); realistic in research.
- `feature-engine.ts` + `primitives/` — primitive signal values (CVD/OFI/leadLag/VPIN/...). Research only.
- `regime-labeler.ts` — causal regime tag per instant; tags each trade by its entry regime.
- `signal-recorder.ts` — records FIRED primitive decisions (features+regime) for the artifact bundle. Observation only.

## Measurement (pure post-processing over the trade list)
- `metrics.ts` — BotMetrics (winrate / PnL / drawdown / Sharpe x2 / edgeBps / fees / histogram / returnStats) + per-regime. Pure.
- `gates.ts` — GatesEvaluator: sample-size / DSR / walk-forward / regime-stability / OOS / loser-category. Pure.
- `compare.ts` — replay vs live closed_trades (decision + fill bands).
- `meta.ts` — determinism hash (JSON round-trip normalized; excludes ackLatencyMs).

## Lifecycle (THE overlap — to be merged, PROMPT-007 Phase 3)
- `gulag.ts` — sweep-result triage: promote / gulag / retire + roster persistence.
- `promotion.ts` — tier formation rules (T1->T2->T3) + the final human live sign-off gate.
  -> merge into ONE state machine: candidate -> probation -> paper -> small_live -> full_live -> retired.
     The merge MUST keep BOTH jobs (triage AND tier-formation + sign-off), not just rename states.

## Orchestration (THE slop — to be collapsed, Phase 1)
- ~10 CLIs: cli-replay, cli-backtest, cli-research, cli-research-sweep, cli-sweep, cli-parallel,
  cli-worker, cli-calibrate, cli-sync, cli-common.
- Several overlapping "run many backtests" runners: sweep-runner, parallel-executor, worker-queue,
  cli-parallel / cli-worker / cli-sweep / cli-research-sweep.
  -> Target: 3 honest entry points (replay / backtest / sweep). cli-common stays as shared helpers.

## What is missing (Phase 1 build — in progress)
- Trade-analysis: MFE / MAE / time-to-MFE / exit-efficiency / duration — NONE today.
  Added research-only: engine excursion tracking behind a flag (off in live) -> `trade-analysis.ts` (one module, one schema).
