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
- BASIS REVERSION = KILLED (2026-06-11). Re-ran on REAL HL full-depth L2 (20-level
  snapshot every ~0.55s; 52,492 pts/8h vs old 57). Edge GONE: gross -2.6..-5.3bps,
  both reversion+momentum net-negative every thr/horizon, win 45-59% (coinflip).
  Old "23-30bps/80-93% win" = ARTIFACT of the BBO-collapsed hlquote sparsity (the
  5-10s gaps WERE the fake edge). docs/research/lead-lag-study.md.
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