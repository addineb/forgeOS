---
tags: [moc, decisions]
type: map
---
# MOC - Decisions log (why we chose what we chose)

Append new decisions at the top with a date. The "why" matters as much as the what.

## 2026-06-11 (CORRECTION - read this first)
- The "BASIS-REVERSION = KILLED" entry below was WRONG: a timestamp bug in my study
  (divided Binance trade ts by 1e6 though it was already ms) froze the Binance leg
  to one price, so it tested HL-vs-its-own-mean, not basis. Caught it building the
  liquidation study. CORRECTED + validated out-of-sample across 6 days / 7 months:
  thr>8bps stretch, ~2min hold = ~9 trades/day, gross +17.8bps, NET +6.75bps after
  11bps, 84% win, POSITIVE EVERY DAY. Momentum mirror negative; small stretches
  (thr>5) net-negative -> only big dislocations snap back (coherent). Remaining
  risk = idealized fills (spread-aware re-test running). Lag-subspace engine build
  is back on the table pending the fill haircut + sign-off. [[lead-lag-study]]
## 2026-06-11
- BASIS-REVERSION = KILLED on dense data. Re-ran the spot-perp study on REAL HL
  full-depth L2 (fresh 20-level snapshot every ~0.55s; 52,492 microprice pts/8h vs
  the old 57). Edge VANISHES: gross -2.6 to -5.3bps hold-to-flip; both reversion
  AND momentum net-negative at every thr/horizon; win 45-59% (coinflip). The old
  "23-30bps, 80-93% win" pulse was a pure ARTIFACT of the BBO-collapsed hlquote
  feed (the 5-10s sparsity quantized price into fake clean round-trips). Same class
  of lie as the first TS engine, from data sparsity not lookahead. [[lead-lag-study]]
- LAG-SUBSPACE ENGINE CLONE = CANCELLED. It was justified only by a trusted pulse;
  there is none. Engine stays untouched. No churn - the lead was tested properly
  (dense, both directions, multiple exits) and is dead.
- Lesson reinforced: NEVER trust a python pre-study built on a thin/derived feed.
  Re-confirm any pulse on raw full-depth data BEFORE believing it or building.
- LAG AVENUES = CLOSED after a big external research pass. Two independent
  large-scale studies (Arrakis Hayashi-Yoshida on 29 assets incl. our Binance-HL
  pair; Microstructure Lab 657M snapshots) both show: lead-lag is real but lives
  in SUB-SECOND latency races (Binance leads HL ~700ms, structural = 2 HyperBFT
  blocks) and the persistent cross-venue gaps are fee/funding EQUILIBRIA that do
  not revert. Untradeable at retail speed + 11bps. No own scan run (would only
  re-derive the untradeable 700ms). [[lag-avenues-study]]
- GO TYPE C: liquidation cascades (forced flow). Aftermath plays out over
  seconds-to-minutes -> inside our speed/cost budget, no latency race.
## 2026-06-10
- DATA PROVIDER = cryptohftdata (CHD), the one we ALREADY use. FREE (no limits
  now; free tier forever). Covers the FULL spec: 9+ spot+deriv venues incl.
  Hyperliquid, FULL-DEPTH tick L2 (snapshots+deltas), trades, funding,
  LIQUIDATIONS; REST API or bulk files; normalized. All paid feeds
  (Tardis/Kiyotaka/CCT/TapeSurf) SUPERSEDED - not needed. The real gap was OUR
  converter pulling only Binance bookDelta+trade + HL thin hlquote -> extend it to
  pull HL FULL L2 (fixes basis-reversion sparsity) + multi-venue L2 +
  liquidations/funding. [[data-apis]]
- CCT (Cloud Craft Terminal) evaluated -> REJECTED for engine: look-only
  terminal, NO data export/API ("No API Key Required"), closed proprietary
  signals = the dead-indicator trap. Useful at most as personal eyes.
- DECISION: the lag/basis CLONE engine will use AGGREGATED GRANULAR L2
  (multi-venue book depth) + funding + liquidations. Data sourcing in
  [[data-apis]]. Key leads: Hyperliquid S3 free L2 snapshots (may unblock
  basis-reversion), CoinGlass ~$35/mo for forced-flow, self-capture multi-venue
  L2 on the box (free). No vendor SIGNALS - raw data only, then our gates.
- CONCLUSION (evidence): plain orderflow INDICATORS (OFI, CVD, imbalance,
  wall-flow, absorption) all = no statistical edge (~15k configs, 0 promote).
  Lesson: orderflow is not the SIGNAL - it is the CONFIRMATION/truth-test on a
  pattern or a structural/forced-flow setup. Stop backtesting plain indicators.
- Brain->strategy Phase A live: drop setup notes in docs/research/labels/
  (template + example there). No-lookahead trigger rule enforced.
- Basis-reversion / cross-venue work will be done on a GIT BRANCH (the "lag
  subspace" = a safe copy of the engine), so we can test different venues,
  settings, and ways of trading the edge WITHOUT touching the proven main engine.
- Obsidian vault = this repo; second brain = MOC method (hubs + links + graph) +
  NotebookLM ([[PROJECT-CONTEXT]]) + always-on Kiro memory (.kiro/steering/state.md).
- Lead-lag (HFT) shelved (HL feed too coarse); reframed to spot-perp BASIS
  reversion - first pulse, must be built + gated honestly. [[lead-lag-study]].
- Public indicators (OFI/wall/CVD) declared dead after ~12k configs, 0 promote.
- Regime is attribution + an opt-in gate, not a hard block (specialization lens).
- Threshold scale fix: imbalance/CVD signals are [-1,1] -> sub-1 thresholds.
- Compute: coarse-then-fine only once a candidate shows a pulse; rent big box by
  the hour for heavy hunts.
- Engine core is SACRED - changes need explicit sign-off + a test.

## Standing rules (never loosen)
- Null-edge: a coinflip must lose ~fees.
- Knob-bite: a "no edge" verdict only counts if the dial moved trades.
- No-lookahead: a trigger uses only data <= now; P&L outcome is scored, not input.
- Metrics in percent; 20x / 20% size / EUR500 paper gate; low win-rate OK if
  expectancy positive; drawdown reported, not gated.
- Clean-room: external repos are reference only; re-derive + test.
- Never trust a pre-study number; only a fully-gated + paper result counts.

## Connected
[[ForgeOS-HOME]] | [[MOC-Engine]] | [[MOC-Strategies]] | [[MOC-Edge-Hunt]]