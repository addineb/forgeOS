---
tags: [moc, engine]
type: map
---
# MOC - Engine (the core, SACRED)

The deterministic, honest replay core. Do not change without explicit sign-off.

## What it is
- [[PROJECT-CONTEXT]] - section 5 "how the engine works" in plain terms.
- [[engine-design]] - no-lookahead, two-clock latency, honest fill model.
- [[roadmap]] - phase plan the engine was built against.
- [[migration-from-wallbot]] - what went wrong in the first (lying) engine.

## The pieces (code lives in crates/)
- forge-core - types, fixed-point money/price, Event.
- forge-data - the *.forge packed stream + reader.
- forge-book - L2 order-book reconstruction (depth 20).
- forge-sim - the sim engine + honest fills + Account (P&L) + execution shell.
- forge-metrics - Sharpe, Deflated Sharpe, PBO.
- forge-sweep - parallel (config x window) sweep + scorecard + ledger.

## Honesty guarantees
- NULL-EDGE gate: a coinflip must lose ~fees (proven on real data).
- Determinism: same input -> identical output + hash; reproducible across threads.
- No mid-fills; queue position + adverse selection modelled.

## Connected
[[ForgeOS-HOME]] | [[MOC-Strategies]] | [[MOC-Edge-Hunt]] | [[MOC-Decisions]]