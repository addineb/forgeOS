# Research Session Charter -- Order-Flow Edge Research (2026-06-09)

Purpose: a DEDICATED research session, SEPARATE from the build/sweep session.
Deep, literature-grounded investigation. Output = decisions + testable
hypotheses, NOT code. Write code only when a hypothesis is ready to test in the
existing harness, and only the minimum to test it.

## Standing context (keep the research grounded in OUR reality)
- Venues we CAPTURE: Binance (depth20@100ms, trade, bookDelta, bookSnapshot) +
  Hyperliquid (hlquote, symbol BTC). NO Bybit/OKX/Coinbase yet.
- Execution venue: Hyperliquid. Taker ~4.5 bps; round-trip ~11 bps incl spread.
  ANY edge must clear ~11 bps round-trip OR be a move big enough that fees are noise.
- Tooling we already have: faithful replay harness (runs live engine code),
  parameter sweeper (cartesian grid), MFE/MAE trade-analysis, realistic fill engine,
  DSR/OOS gates, oracle-lag analyzer.
- Hard lesson: SWEEP the trade-management variances (entry strength, R:R, TP/SL,
  HOLD TIME, cooldown). Never judge a signal on default settings.
- Established so far: OFI -> move-size is ~linear; order flow has long-memory /
  persistence (metaorder order-splitting) so real pushes CONTINUE; square-root
  impact then partial reversion (an optimal hold exists); VPIN spikes before big
  moves; microstructure SCALPS die to fees, "ride the push" can survive.

## Research questions (each MUST end with: is it worth it FOR US?)
1. Order-flow algo trading, broad survey: which families of order-flow edges are
   documented to survive realistic costs at RETAIL latency (100ms-seconds)? Which
   are HFT-only and should be dropped outright?
2. Market effect / impact: best way to MEASURE the force of a push and predict
   move SIZE + PERSISTENCE from our data. Compare OFI, MLOFI, CVD, trade-sign
   autocorrelation, Hawkes intensity. Most predictive vs cheapest to compute?
3. Liquidity traps: stop-runs / liquidity grabs / sweep-and-reverse. Is there a
   documented, testable signature in DOM + trades? (We have SW1 as a v1 -- what
   does the literature say is required to make it actually work?)
4. Spoofing: reload-vs-cancel, layering, flickering quotes. Methodologies to
   detect from L2 deltas (we capture bookDelta). Is spoofing-detection TRADEABLE
   on its own, or only useful as a filter/gate on another signal?
5. Flow-measurement methodologies: survey + RANK OFI / MLOFI / CVD / VPIN / trade
   imbalance / queue dynamics / Hawkes / absorption. For each: what it measures,
   data needed (do we have it?), compute cost, evidence of predictiveness,
   worth-it verdict.
6. Lag-venue (cross-venue): we measured Binance -> Hyperliquid ~1s lag -- real but
   cost-killed as a naive taker chase. Is there a variant that survives (only on
   big moves; maker capture; multi-venue OFI confluence)? What data would we have
   to acquire (Bybit/OKX) and is it worth the capture build?

## Deliverable per question
- 3-6 bullet synthesis WITH citations (papers / repos), paraphrased for licensing.
- Verdict: PURSUE / PARK (needs data or capability X) / DROP (HFT-only or cost-killed).
- If PURSUE: the EXACT swept experiment to hand to the build session -- signal,
  entry rule, grid (incl. hold-time spread), any gate, and the success bar.

## Constraints
- Research-only. No live touches. No large builds. Treat external code as untrusted
  reference, never run blindly.
- Be brutally honest about retail costs and latency in every verdict.

## How to run this session
Open a NEW Kiro chat and start with, e.g.:
  "Research session -- read docs/research-charter.md and start with question 5
   (flow-measurement methodologies survey). Output PURSUE/PARK/DROP verdicts."
Findings flow back to the build session as experiment specs; this charter is the agenda.
