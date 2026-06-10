---
tags: [moc, strategies]
type: map
---
# MOC - Strategies (the bots)

Each bot is just an EntrySignal ("which way?") on the shared execution shell.
All go through the same gates: shuffled control, null-edge, knob-bite, DSR/PBO,
paper.

## Tested - NO edge (crowded public indicators)
- OFI momentum, wall/imbalance, CVD - slow+fast + regime-gate, ~12k configs,
  0 promote. See [[edge-directions]] for why (arbitraged out).

## Built
- Regime classifier + attribution + opt-in gate (Trending/Sideways/Neutral).
- wall-flow ([[real-vs-fake-wall]]) - eaten vs pulled/spoof walls.
- absorption - price holds under aggressive fire (pending box validation).
- Results ledger - per-run scorecard CSV + verdict history.

## Promising (first pulse, NOT trusted)
- Spot-perp basis reversion - [[lead-lag-study]].

## Roster not built yet
- Liquidity-sweep, LVN, lead-lag (HFT - data too coarse).

## Connected
[[ForgeOS-HOME]] | [[MOC-Engine]] | [[MOC-Edge-Hunt]] | [[MOC-Decisions]]