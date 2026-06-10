# ForgeOS - TODO / Research Backlog

Living task list. Roadmap phases live in `docs/roadmap.md`; this is the
actionable backlog. Check items off as they land.

## Status (2026-06-10)
- Phases 0-5 built, green, pushed. Engine complete + honest end to end:
  data -> book -> honest fills -> strategy (shell) -> sweep -> scorecard -> paper gate.
- Gates green: null-edge (coinflip loses), no-fake-edge (shuffled collapses),
  determinism (identical across thread counts), 78 tests, clippy -D warnings.
- 3 theses sweepable (`--strategy ofi|wall|cvd`); 15 *.forge windows (~6 months);
  EUR500 / 20x / 20% / 5%-daily paper gate.
- Phase 6 (live) is gated behind a config clearing BOTH the sweep bars and the
  paper gate.

## Now (active)
- [ ] Read the slow-horizon hunt: `tools/sweep-wait.sh hunt` -> ofi + wall verdicts.
- [ ] Sweep CVD (fast + slow) across all 15 windows; compare to ofi/wall.

## Strategy harnessing (port onto the execution shell - each ~1 day now)
- [ ] Liquidity-sweep (SW1): watch wall -> pulled (cancel, not eaten) -> sweep ->
      exhaust -> fade back to the wall price.
- [ ] Lead-lag (Binance -> Hyperliquid): big-move-only variant, gated by
      predicted-move > cost.
- [ ] Trend / TSMOM: sign/strength of past return over a lookback; also as a
      higher-timeframe FILTER on the microstructure bots.
- [ ] Absorption: signed (dominant aggressor) + reload/executed confirmation.

## Sweep / research refinements
- [ ] TP/SL + reversion grids on any config that shows a pulse (Park).
- [ ] Time-based holds (ns) instead of event-count -> true slow horizon
      (minutes-hours), robust to event clustering.
- [ ] Longer windows (6h / full-day) - needs streaming or zero-copy EventRecord
      iteration so multiple big windows fit in 7.6 GB RAM.
- [ ] Real-vs-fake wall classification using the trade stream (legacy open-thread
      #1: split a size decrease into executed-vs-cancelled).

## Data
- [ ] Pull more days / longer windows for OOS depth (API works; run in tmux via
      tools/pull-data.sh). More data makes DSR/PBO fairer.

## Validation / metrics
- [ ] CPCV (combinatorial purged CV with embargo) to complement PBO.
- [ ] Persist scorecards to disk + a verdict ledger (promote/park/retire history
      per bot) so the hunt is a tracked process, not re-grepped logs.

## Gates before live (hard order)
- [ ] 1. Sweep: clears net>0, knob-bite, DSR + PBO (park then promote bars).
- [ ] 2. Paper gate: EUR500 / 20x / 20% size / 5% daily limit -> profitable,
         not ruined.
- [ ] 3. Only then Phase 6.

## Phase 6 - live (deferred until a validated edge)
- [ ] Hyperliquid venue adapter feeding the SAME sim/strategy core; live clock;
      orders route to the exchange.
- [ ] Engine hooks for guards: heartbeat, kill-flag check, metrics line.
- [ ] tmux live cockpit (fills / position+P&L / book / log) + status bar.
- [ ] Watchdog + stale-feed alarm + kill-switch (tmux trips a flag; engine
      enforces flatten+halt).
- [ ] Golden-vector parity: replay the captured live stream through backtest,
      diff trades.
- [ ] Progression: paper -> tiny live -> scale, daily loss limit on.

## Deferred / someday
- [ ] Phone-accessible PAPER-TRADING monitor (personal use, when live).
- [ ] Product dashboard (commercial - sweep results + bot tracking as a service).

## Hard rules (never loosen)
- Null-edge gate stays green (a coinflip must lose ~fees).
- Every change touching fills/P&L carries a test that catches its failure mode.
- Knob-bite: a "no edge" verdict is valid only if the swept param moved trade
  behaviour.
- Clean-room: external repos are reference only; re-derive + test.