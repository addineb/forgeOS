# ForgeOS - Project Context (second-brain pack)

Self-contained context for NotebookLM (upload this file) and for reading in
Obsidian. Plain language. If something here conflicts with code, code wins; tell
Kiro to update this doc.

## 1. What this is, in one breath
ForgeOS is a from-scratch, Rust backtesting engine for finding a real trading edge
in crypto market microstructure (order book + trade tape), built so it CANNOT lie
to us. When an edge survives every honesty check and a paper-trading test, it gets
deployed live on Hyperliquid.

## 2. Why it exists (the origin story)
The first attempt was a TypeScript engine. After 3 sleepless days it reported a
strategy that won 100% of trades, made money on every variation, and never drew
down. That is impossible - it was lying: it peeked at the future (lookahead), kept
the books wrong, and filled orders at prices that do not exist. A backtester that
flatters you is worse than useless. ForgeOS is the clean rebuild whose whole job is
to tell the truth, especially "no".

## 3. Who is building it
A trader with ~10 years of discretionary microstructure experience - reads the
book and tape by eye. Not a coder. Real trading capital around EUR 500. The edge
lives in his intuition; the engine's job is to extract it into explicit, testable
rules without the lookahead/curve-fit rot that killed the first project.

## 4. The unbreakable rule (null-edge gate)
A seeded coinflip strategy must LOSE money (about the cost of fees + spread) every
time. Proven on real data: a coinflip over 1.74M BTC events lost, with the loss
being entirely fees and ~zero directional P&L - exactly what honesty looks like.
If a coinflip ever profits, we stop everything and fix the engine.

## 5. How the engine works (plain terms)
- DATA: real Binance order book + trades (the signal) and Hyperliquid quotes
  (execution venue), pulled from a verified feed, packed into compact *.forge
  files the engine replays fast. 15 windows across ~6 months.
- ORDER BOOK: rebuilt tick by tick from the diffs (depth 20).
- SIM + FILLS: replays each event with realistic delay and HONEST fills - no
  magic mid-price fills; models queue position and getting picked off.
- EXECUTION SHELL: the shared, tested order/risk plumbing every bot uses (one
  order at a time, hold/cooldown in real time, take-profit/stop, optional regime
  gate). Each bot only writes its "which way do I go?" signal.
- SWEEP: tests thousands of setting combinations across all windows in parallel,
  scores each honestly, and gives a verdict: promote / park / retire.
- GATES before any number is trusted: null-edge, shuffled-direction control,
  knob-bite, Deflated Sharpe + PBO (overfit guards), then the EUR500 paper gate.
- DETERMINISM: same input -> identical output, down to a hash. Reproducible.

## 6. The bots (roster)
- Tested, no edge: OFI momentum, wall/imbalance, CVD (all crowded public signals).
- Built: regime classifier + attribution (labels Trending/Sideways/Neutral and
  splits each bot's P&L by regime), opt-in regime gate, results ledger, wall-flow
  (real-vs-fake wall: eaten vs pulled/spoof), absorption (price holds under fire).
- Reframed/promising: spot-perp basis reversion (was "lead-lag").
- Not built: liquidity-sweep, LVN, lead-lag (HFT version - data too coarse).

## 7. What we have learned
- Plain public indicators are arbitraged to ~zero on liquid BTC. Confirmed across
  ~12,000 swept configs: 0 promote. A trustworthy "no".
- Regime matters: momentum bots bleed in sideways/chop, do least-bad in calm.
- FIRST PULSE: when the Hyperliquid perp stretches far from Binance spot, it snaps
  back over ~20 min (80-93% of the time, ~40 setups/day, ~23-30 bps gross). This
  is a structural/funding (basis) effect, not an HFT race. NOT yet trusted -
  measured with idealized fills on a tiny sample; must be built and gated honestly.

## 8. Where real edge probably lives (the approach)
Not in the indicator itself, but in: CONTEXT (a weak signal that only works in a
specific situation), HARD-TO-COMPUTE signals (e.g. real-vs-fake walls - needs
matching trades to book diffs; no platform ships it), and FORCED FLOW
(liquidations, funding/basis, stop runs - someone is forced to be the dumb side).
Trader method 1: use a CHART PATTERN as the trigger (where to look) and the
ORDERFLOW as the confirmation (is this one real or a fakeout?). Rule: the trigger
must be detectable from price up to NOW only (no peeking ahead); the P&L outcome
is the future thing we score, never an input.

## 9. Glossary (trader language)
- Order flow: the live stream of orders/trades; what actually moves price.
- OFI: order-flow imbalance - net change in resting size at the touch.
- CVD: cumulative volume delta - aggressive buys minus sells.
- Imbalance / wall: standing resting size; a big order = a wall.
- Wall-flow (real vs fake): did a wall vanish because it was EATEN (real) or
  PULLED/cancelled (spoof)?
- Absorption: aggressive orders keep hitting a level but it won't break - a big
  passive player is soaking it up.
- Regime: is the market Trending or Sideways/choppy.
- Basis: the price gap between the perp (Hyperliquid) and spot (Binance).
- Knob / dial: an adjustable setting on a strategy. Knob-bite: a "no edge" result
  only counts if changing the dial actually changed the trades.
- Null-edge: a coinflip must lose; proves the engine is honest.
- Lookahead: using future data to make a past decision - the cardinal sin.
- DSR (Deflated Sharpe): chance the edge is real after accounting for how many
  things we tried. PBO: probability the backtest is overfit. Both punish luck.
- Paper gate: simulate a real EUR500 account before risking money.

## 10. Standing decisions
- Report metrics in percent. Leverage 20x, position 20% of account, EUR500 paper
  account, 5% daily-loss limit. Pass = profitable + not ruined (drawdown reported,
  not a hard gate). Low win rate is fine if expectancy is positive.
- Engine core is sacred - changes need explicit sign-off + a test.
- Compute: brute-force is fine while things are dead; switch to coarse-then-fine
  and rent a big box by the hour once a candidate shows a pulse.
- Tools: Obsidian (this repo as the vault) for notes/map/graph; NotebookLM as the
  queryable second brain; a future book/tape visualizer to capture his setups.

## 11. Current plan
1. Tooling pass (this): Obsidian vault + NotebookLM second brain + always-on memory.
2. Then: either build the basis-reversion edge HONESTLY (real HL-quote fills,
   spread+fee+funding, swept knobs, gates, paper) - the only live lead - or stand
   up the brain->strategy pipeline (annotator + chart-trigger x orderflow).
- Live ConfigStrategy on Hyperliquid only after a config clears sweep + paper.

## 12. Using this in NotebookLM
Upload this file (and optionally docs/TODO.md, docs/research/*.md) as sources. Ask
it things like "what edges have we ruled out and why", "what is the basis-reversion
pulse and why don't we trust it", "what are the honesty gates". Keep this doc
updated as the project moves so the second brain stays accurate.