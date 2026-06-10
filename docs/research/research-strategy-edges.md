# Research Note -- Strategy Families With a Documented Edge ("what works") (2026-06-09)

Brief: survey strategy families with a documented, COST-SURVIVING edge beyond the
order-flow microstructure already covered (Q1-Q6), filtered to OUR reality:
single asset (BTCUSDT signal), Hyperliquid execution, ~11 bps round-trip, retail
latency (100ms-seconds), paper today. Same rules: additive, backtest before live.

## Meta-lesson up front (read this first)
Most published crypto-strategy results are IN-SAMPLE, PRE-COST, or overfit. The
ones that survive REALISTIC costs cluster in two places:
  (1) LOWER-FREQUENCY directional/structural edges (trend, carry) where ~11 bps
      is noise against the move, and
  (2) STAT-ARB with EXPLICIT cost accounting and many names.
Our 11 bps wall kills almost everything HFT-flavoured (consistent with Q1). So
"what works for us" skews to slower, structural edges -- which also COMPLEMENT
(rather than compete with) the fast microstructure work.

================================================================================
RANKED VERDICTS
================================================================================

## 1. Time-series momentum / trend-following -- PURSUE (best single-asset fit)
- Evidence: one of the most robust documented anomalies across all asset classes
  (Moskowitz, Ooi & Pedersen, "Time Series Momentum", JFE 2012). Crypto/BTC is
  especially momentum-driven and reflexive (Grayscale "The Trend is Your Friend";
  multiple practitioner + academic confirmations).
- Why it fits us: works at HOURS-to-DAYS horizon, so ~11 bps is negligible vs the
  captured move; trades infrequently (low cost drag); single-asset BTC is fine;
  long-only or long/short both documented.
- Data: price only (have it) -- BUT trend needs WEEKS-MONTHS of history to
  validate; our ~7h window is far too short. This is capture-gated for backtest
  confidence, not for the idea.
- Spec: classic TSMOM -- sign/strength of past return over lookback L drives
  position; vol-target the size. Grid: L {1h,4h,12h,1d,3d}; vol-target window;
  with/without a trend filter. Success bar: positive net after costs + DSR/OOS on
  a LONG history (needs Tardis-style backfill).

## 2. Momentum/regime as a FILTER on the microstructure bots -- PURSUE (synergy)
- Insight: trend and order-flow live at different timescales and combine well.
  Use the slower trend/regime read as a GATE: take OFI/sweep/absorption entries
  only WITH the higher-timeframe trend (or only mean-reversion entries in
  balance regimes). This is the cheapest way to lift the existing PURSUE bots.
- Data: have it. Ties directly to the Q5/Q2 bots (adds a regime gate param).
- Spec: add a `trendRegime` gate (sign of N-hour return / above-below a slow MA)
  to the OFI and sweep bots; sweep gate on/off. Success bar: improves
  net-per-trade or hit-rate on OOS vs ungated.

## 3. Funding-rate carry / basis (delta-neutral) -- PARK (data-gated, different machine)
- Evidence: documented and sizable. Cross-sectional crypto carry returns ~43.4%
  annualised, Sharpe ~0.74 (Fan, Jiao, Lu & Tong, "The Risk and Return of
  Cryptocurrency Carry Trade", SSRN 4666425). BTC perp funding ran ~8-15% ann in
  2023-24, compressing to ~4-10% by 2026. Harvest via spot-long + perp-short to
  collect positive funding (market-neutral INCOME, not directional).
- Why PARK: it is a DIFFERENT bot type (delta-neutral, two legs, funding capture)
  than our directional signal bots; needs funding-rate + spot data we do NOT
  capture; and the edge is an income stream, not a price call.
- Unlock: capture funding rate + spot/perp basis. Then it is a clean, low-
  variance income overlay worth having. Recommend as a separate track.

## 4. Statistical arbitrage / cointegration pairs -- PARK (needs multi-asset universe)
- Evidence: notably COST-ROBUST. One study reports +7.1 bps/day AFTER 15 bps
  half-turn costs over 100k+ trades (Statistical Arbitrage in Cryptocurrency
  Markets, MDPI/JRFM 2019); dynamic cointegration pairs beat buy-and-hold under
  realistic best-bid/ask fills (Tadi & Kortchmeski, arXiv:2109.10662).
- Why PARK: it is inherently MULTI-ASSET (pairs/baskets, mean-reversion of a
  cointegrated spread). We are single-asset BTCUSDT -> blocked.
- Caveat: simple BTC/ETH pairs are now saturated/unprofitable after costs;
  edge is in larger baskets + dynamic re-estimation.
- Unlock: capture a universe (ETH/SOL/major perps) via Tardis or native feeds.
  High-potential IF we expand beyond one asset.

## 5. Grid trading -- CAUTION / PARK (no inherent edge)
- Evidence: the TRADITIONAL grid has ~ZERO expected return by construction;
  dynamic-grid variants can outperform but are essentially disguised SHORT-VOL /
  mean-reversion and blow up in trends (Chen et al., "Dynamic Grid Trading",
  arXiv:2506.11921). Not a real alpha source -- a payoff-shaping scheme.
- Verdict: not an edge; skip unless explicitly running a range/short-vol regime
  with hard risk control.

## 6. ML price-direction classifiers -- PARK (overfit risk, low SNR)
- Evidence: promising in-sample (e.g. profit factor ~1.6 on short unseen spans,
  Asgari & Khasteh, arXiv:2105.06827), but order-book/price alphas have extremely
  low signal-to-noise and overfit easily; reference catalog: Kakushadze & Serur,
  "151 Trading Strategies", arXiv:1912.04492.
- Verdict: not a standalone edge for us yet; if used, it is a COMBINER over the
  validated primitives, with heavy DSR/OOS discipline -- not a from-scratch model.

================================================================================
SUMMARY TABLE
================================================================================
| Family                         | Verdict                  | Blocker / fit       |
|--------------------------------|--------------------------|---------------------|
| Time-series momentum (trend)   | PURSUE                   | needs long history  |
| Trend/regime FILTER on micro   | PURSUE (synergy)         | none -- cheap        |
| Funding carry / basis          | PARK (data-gated)        | funding+spot capture|
| Stat-arb / cointegration pairs | PARK (needs universe)    | multi-asset capture |
| Grid trading                   | CAUTION (no edge)        | --                   |
| ML direction classifiers       | PARK (overfit/low SNR)   | --                   |

## Honest take
For a SINGLE-asset bot today, the two real, do-able edges are TREND (standalone,
slower timescale where fees are noise) and TREND-AS-A-FILTER on the microstructure
bots we already researched. Carry and stat-arb are genuinely documented and
cost-robust but need data/universe we do not yet capture -- they are the strongest
arguments for the capture build (funding + a multi-asset universe via Tardis or
native feeds). Everything fast enough to ignore trend (grid, ML scalping) does not
clear 11 bps on its own.