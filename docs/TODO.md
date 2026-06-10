# ForgeOS - TODO / Research Backlog

Living task list. Roadmap phases live in `docs/roadmap.md`; this is the
actionable backlog. Check items off as they land.

## Status (2026-06-10)
- Phases 0-5 built, green, pushed. Engine complete + honest end to end:
  data -> book -> honest fills -> strategy (shell) -> sweep -> scorecard -> paper gate.
- Gates green: null-edge (coinflip loses on real data), no-fake-edge (shuffled
  collapses), determinism (byte-identical hash across runs + thread counts),
  clippy -D warnings. Engine core (engine/account/fills/book/core/data) frozen.
- 3 theses sweepable (`--strategy ofi|wall|cvd`); 15 *.forge windows (~6 months);
  EUR500 / 20x / 20% / 5%-daily paper gate.

## Done since last status
- [x] Time-based holds (`hold_ns`/`cooldown_ns`) - holds in real time (s/m/h),
      robust to event clustering. Duration parser (30s/5m/2h) in the CLIs.
- [x] Regime classifier (Kaufman efficiency ratio): labels Trending/Sideways/
      Neutral. Sweep ATTRIBUTES each config's P&L by regime (trendN/sideN/neutN)
      so we see which variant earns where.
- [x] Opt-in regime GATE in the shell (`--regimes any|trend|side|neutral`),
      default Any so baseline is unchanged. Exits never gated.
- [x] Threshold-scale fix: imbalance/CVD signals are bounded [-1,1]; their sweep
      thresholds are now sub-1 (were 1-4 = never traded = invalid).
- [x] First full hunt (6 sweeps): OFI valid (no edge, dies in chop); wall/CVD
      were invalid (threshold bug) - re-running.
- [x] Results ledger: `--out <dir>` writes a per-run scorecard CSV + appends a
      summary row to ledger.csv (tracked verdict history, not grepped logs).

## Now (active - ordered, one step at a time)
- [x] Public-indicator hunts done (OFI/wall/CVD + regime-gate): all no-edge.
- [~] LEAD-LAG -> reframed as SPOT-PERP BASIS REVERSION: FIRST PULSE found
      (~40 trades/day, ~23-30bps gross, 80-93% win, ~20min holds, net-positive
      even @taker on 3 days). NOT trusted yet (idealized fills, tiny sample,
      hand-picked knobs, funding ignored). See docs/research/lead-lag-study.md.
- [ ] 1. BUILD basis-reversion strategy honestly (HL-quote fills, spread+fee+
      funding) and run gates (shuffled control, sweep, DSR/PBO, paper).
- [ ] 2. BRAIN -> STRATEGY engine (does not need HL data).
- [ ] 2. BRAIN -> STRATEGY engine: annotator/visualizer + chart-trigger x
      orderflow harness. Spec: docs/research/annotator-visualizer.md. Phase A
      (label-note convention, ~zero build) first; Phase B (book/tape viewer) when
      an idea needs the orderflow truth-view.
- Pending validation (built, not yet compiled on box): absorption bot.
- Wall-flow hunt finishing on its own; NO new hunts started (hunting paused).
- Paused roster: liquidity-sweep, LVN (resume after the above).

## Edge approach (READ before picking the next thesis)
- Public indicators used plainly = no edge (OFI/wall/CVD confirmed). Edge lives
  in CONTEXT (conditioning), HARD-TO-COMPUTE signals, or FORCED FLOW (liqs /
  funding / stops). Ranked hit-list + plan: docs/research/edge-directions.md.
- Plan: finish testing current bots, then pivot to type B/C theses.
- Chart-trigger x orderflow-confirm (trader method 1): chart pattern = TRIGGER,
  orderflow = real/fake CONFIRM filter. Real-time-only trigger (no lookahead).
  Full plan in docs/research/edge-directions.md.
- [ ] Book/tape VISUALIZER (PROMOTED - the unlock): render book+tape around a
      timestamp so the trader sees what the engine sees + can label setups.

## Strategy harnessing (port onto the execution shell - each ~1 day now)
- [ ] Real-vs-fake wall classification (executed-vs-cancelled from the trade
      stream). The legacy open-thread; likely the biggest edge.
- [ ] Liquidity-sweep (SW1): wall -> pulled (cancel) -> sweep -> exhaust -> fade.
- [ ] Lead-lag (Binance -> Hyperliquid): big-move-only, gated by move > cost.
- [ ] Trend / TSMOM: also as a higher-timeframe filter (overlaps regime layer).
- [ ] Absorption: dominant aggressor + reload/executed confirmation.

## Sweep / research refinements
- [ ] CPCV (combinatorial purged CV with embargo) to complement PBO. Treat as a
      correctness-critical stat: implement carefully WITH tests.
- [ ] Longer windows (6h / full-day): needs streaming / zero-copy EventRecord
      iteration so multiple big windows fit in 7.6 GB RAM.
- [ ] TP/SL + reversion grids on any config that shows a pulse (Park). (Not code -
      just run a sweep with --tps/--sls/--reversions set.)

## Compute / cost (when hunts get heavy)
- [ ] Coarse-then-fine sweeps: cheap wide grid to find the promising region,
      then a fine grid only around winners (~10-20x less compute). Add WHEN a
      candidate shows a pulse, not for blind first looks.
- [ ] On-demand compute: rent a high-core box BY THE HOUR for big hunts (tmux,
      pull results, destroy). Do not pay monthly for idle cores. Current box is
      fine for day-to-day.

## Data
- [ ] Pull more days / longer windows for OOS depth (tools/pull-data.sh in tmux).

## Gates before live (hard order)
- [ ] 1. Sweep: net>0, knob-bite, DSR + PBO (park then promote bars).
- [ ] 2. Paper gate: EUR500 / 20x / 20% size / 5% daily limit -> profitable, not ruined.
- [ ] 3. Only then Phase 6.

## Phase 6 - live (deferred until a validated edge)
- [ ] Hyperliquid venue adapter feeding the SAME sim/strategy core; live clock.
- [ ] Engine guards: heartbeat, kill-flag, metrics line; watchdog + stale-feed alarm.
- [ ] tmux live cockpit (fills / position+P&L / book / log).
- [ ] Golden-vector parity: replay captured live stream through backtest, diff trades.
- [ ] Progression: paper -> tiny live -> scale, daily loss limit on.

## Deferred / someday
- [ ] Phone-accessible PAPER-TRADING monitor (personal use, when live).
- [ ] Product dashboard (commercial - sweep results + bot tracking as a service).

## Hard rules (never loosen)
- Null-edge gate stays green (a coinflip must lose ~fees).
- Every change touching fills/P&L carries a test that catches its failure mode.
- Knob-bite: a "no edge" verdict is valid only if the swept param moved trades.
- Clean-room: external repos are reference only; re-derive + test.