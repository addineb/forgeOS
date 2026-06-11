# ForgeOS - living state + who I work with (always-on memory)

Auto-loaded every session. Keep it current. Full narrative: docs/PROJECT-CONTEXT.md.

## Who the trader is (talk to him this way)
- 10y DISCRETIONARY microstructure trader. NOT a coder. Explain in simple TRADING
  terms; avoid jargon ("knob" = setting/dial). He understands trading logic deeply
  - never condescend.
- Burned by his first project (a TS engine that LIED: fake 100% win from lookahead).
  So: be the HONEST SKEPTIC. Correct him when wrong. Never flatter a result.
- Real account ~ EUR 500. Deploys on Hyperliquid.
- Works STEP BY STEP; gets uneasy if overwhelmed or if things get messy. He keeps
  adding ideas - CAPTURE them in docs so nothing is lost.

## What ForgeOS is
Clean-room Rust backtest engine for crypto microstructure edge discovery. The #1
rule: a seeded COINFLIP must LOSE ~fees (null-edge gate). If it ever profits, stop
and fix - the engine is lying.

## Engine = SACRED (do not touch without explicit sign-off)
engine.rs, account.rs, fills.rs, forge-book, forge-core, forge-data. New work goes
in the strategy/shell/sweep/tooling layers ABOVE it. Verified honest: det-hash
reproducible, coinflip loses on real data.

## Current state (update as we go)
- Public indicators (OFI / wall / CVD), slow+fast + regime-gate: ALL no-edge
  (~12k configs). Confirms: crowded signals are arbitraged out.
- Built: regime classifier + attribution + opt-in gate; results ledger.
- wall-flow (real-vs-fake wall): SWEPT slow+fast -> 0 promote (no edge, but valid).
- absorption: validated (clippy+test green) + SWEPT (in progress / read result).
- BASIS REVERSION = RE-OPENED (2026-06-11). My morning "KILLED" was a TIMESTAMP BUG
  (divided Binance trade ts by 1e6 though it was already ms -> Binance leg frozen to
  one price; accidentally tested HL-vs-its-own-mean). Corrected single day
  (2026-02-01): thr>8bps, ~3min hold = ~39 trades/day, gross +15bps, NET +4.2bps
  after 11bps, 92% win; momentum side a clean mirror. THIN (11-14 trades, idealized
  fills, funding ignored). Multi-day out-of-sample validation running.
  docs/research/lead-lag-study.md.
- Tooling DONE: Obsidian vault (MOC + styled graph) + NotebookLM pack + this
  always-on memory + tools/sync-vault.ps1 (repo->vault). Vault at
  C:\Users\User\Desktop\obsidian\forgeos.
- DATA = cryptohftdata (CHD, already in use, FREE no limits). Covers full spec:
  9+ venues incl Hyperliquid, FULL-DEPTH tick L2, trades, funding, LIQUIDATIONS;
  REST/bulk. Paid feeds (Tardis/Kiyotaka/CCT) NOT needed. Gap was OUR converter
  (only pulled Binance bookDelta+trade + thin HL hlquote). FIX: extend
  chd-to-parquet.py to pull HL FULL L2 (unblocks basis-reversion) + multi-venue L2
  + liquidations/funding. Do not re-shop providers.
- LAG-SUBSPACE engine clone = CANCELLED (no pulse to justify touching the engine).
- NEXT: pick a fresh lead from the UNTESTED high-value buckets - Type C forced-flow
  (LIQUIDATION cascades, via CHD liquidations) or Type B chart-trigger + orderflow-
  CONFIRM (the labels/ on-ramp). No engine touch needed for the studies.

## Standing decisions / rules
- Metrics in PERCENT, not bps (in reports). Leverage 20x, size 20%, EUR500 paper
  gate (profitable + not ruined; drawdown reported, not gated).
- Verdict gates on net/expectancy/DSR/PBO + KNOB-BITE (a "no edge" only counts if
  the dial changed trades). Low win rate is fine if expectancy is positive.
- Edge lives in: CONTEXT (conditioning) / HARD-TO-COMPUTE / FORCED-FLOW - not plain
  public indicators. Trader method 1 = chart pattern as TRIGGER, orderflow as
  real/fake CONFIRM. No-lookahead: trigger uses only data <= now.
- Compute: coarse-then-fine sweeps when a candidate shows a pulse; rent a big box
  BY THE HOUR for heavy hunts; current box for daily.
- NEVER trust a python pre-study number; only a fully-gated + paper result counts.

## Workflow norms (see .kiro/steering/environment.md for box/shell gotchas)
- Build/test on the Hetzner box in tmux; commit small + push + verify each step.
- File writes via execute_pwsh WriteAllText (no-BOM); heavy/box work detached in
  tmux (never block on ssh - it orphans).
- BASIS update: survives spread (HL spread ~0.13bps); REAL fills+9bps fee = NET
  +8.55bps/trade, ~9/day, positive all 6 days. Only unmodelled risks left = LATENCY/
  adverse-selection + impact = the ENGINE's job. NEXT = lag-subspace engine build
  (needs SIGN-OFF). Type C cascade study coded + bug-fixed, ready to run in parallel.

- SUBSPACE BUILD (branch lag-subspace, signed off): data contract documented
  (docs/research/subspace-design.md). Step 1 DONE+VERIFIED: converter hlbook stage
  (HL full-L2 -> correct delta stream) reconstructs IDENTICAL top-5 microprice
  (max|diff|=0.0 over 6539 snaps). NEXT: forge-data convert.rs hl_book stream ->
  *.forge; then BasisReversion strategy; then gates (null-edge/knob-bite/DSR/PBO/paper).

- SUBSPACE COMPLETE (steps 1-6, branch lag-subspace): hlbook stream (verified
  exact) + BasisReversion strategy + latency-ladder sweep + paper gate; build/clippy/
  test green; engine core UNTOUCHED. ENGINE-GRADE VERDICT: basis edge is REAL
  (shuffle control loses), degrades smoothly with latency (9bps@0ms -> 3bps@700ms),
  lives in SIDEWAYS regime, PAPER GATE PASSES (EUR500/20x +5.7-7.8% @300ms). BUT
  DSR~=0 -> 0 promote (too thin on 6 days). NOT trusted live. NEXT: many more days
  for DSR/PBO power + sideways-only regime gate. (Fixed a fill_timeout<latency
  order-flood artifact mid-run.)

- 13-DAY HUNT done: PBO 0.002 (OOS-stable), edge real @0ms (t~2.0-2.25) but DROPS
  below significance @300ms latency (t~0.9-1.4). Paper +5.8-6.6%@300ms not trusted
  (t~1). DSR gate mis-calibrated for sparse strategy (bucket-Sharpe~0.017) -> use
  per-trade t-stat. VERDICT: promising+OOS-stable but UNPROVEN at realistic latency.
  NEXT: more days (sqrt(N): ~26+ days to reach t~2 @300ms if edge holds) - CHD has
  more HL days to pull. Viability hinges on execution latency (<150ms = t~2).

- FORGELAG ENGINE (branch forgelag, dedicated basis engine, null-edge passes):
  22-CONSECUTIVE-DAY hunt on FRESH data = edge SIGNIFICANT at realistic latency.
  per-trade t-stat: 300ms->t=4.95 (+5.7bps,742 trades), 500ms->3.31, 700ms->1.35,
  1s->-2.44 (inverts). Shuffle control t=-6.70 (negative). LATENCY CLIFF: tradeable
  only <~500ms exec to HL. Best lead by far. NEXT gates: robustness across OTHER
  months (these 22d = one regime), multi-venue/aggregated ref, drawdown/ruin check,
  real exec-latency measurement. docs/research/forgelag-hunt.md.

- OOS CONFIRMED: basis edge HOLDS on Feb (14d, independent of May-Jun): 300ms
  t=4.77 vs May-Jun t=4.95. Two independent periods significant + shuffle negative
  + no ruin. Validated across regimes - strongest result yet. NEXT: expand
  (venues/variances/dead-indicator confirm) + measure REAL HL exec latency + live paper.
