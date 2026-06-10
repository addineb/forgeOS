---
tags: [research, lag, killed]
type: research
---
# Lag avenues - big research (2026-06-11) -> NO tradeable lag for us

Triggered after basis-reversion was killed. Question: is there ANY cross-venue
lag/lead avenue (different exchange combos, spot/perp, aggregated) that we can
trade at retail (~EUR500, order round-trip >> 100ms) net of ~11bps round-trip?

## VERDICT: NO. Lead-lag in crypto is a colocation/latency business. Cost+speed killed.
Two independent large-scale third-party studies (one on our EXACT Binance-HL pair)
agree, and they line up with our own dense-data basis kill from the same day.

## Evidence 1 - Arrakis lead-lag study (Hayashi-Yoshida, 29 assets, 16d to 2026-02-26)
Source: chaincatcher / weex (Arrakis, "Who is leading price discovery").
- Binance LEADS Hyperliquid by ~700ms - consistent across ALL 29 assets.
- Lighter leads HL by ~700ms too; Binance leads Lighter by only ~100ms (transitive,
  median residual -33ms -> it is a real structural ordering, not an artifact).
- The 700ms HL lag is STRUCTURAL: two HyperBFT consensus cycles (block N = maker
  re-quote, block N+1 = taker fill). HL official single-block finality ~200ms; the
  extra ~500ms is the maker->taker round-trip across two blocks.
- WHO captures it: colocated bots racing IOC orders into HL stale liquidity inside
  those 700ms (the article describes the mempool race explicitly). A retail order
  round-trip is far slower than 700ms -> we arrive after the lead is gone. Plus
  11bps cost on a lead worth a fraction of that. NOT tradeable by us.

## Evidence 2 - Microstructure Lab (657M L1 snapshots; Binance/GateIO/OKX spot + Binance-fut vs HL perp)
Source: themicrostructurelab.substack (Jan-Feb 2026, 100ms grid).
- Cross-exchange mid gaps close in a FEW HUNDRED ms (BTC) -> latency arb.
- Executable arb (bid_A>ask_B) exists ~99.9% for BTC/ETH but ONLY at the ~4.6bps
  STRUCTURAL offset (e.g. persistent OKX discount). Round-trip taker fee ~20bps
  base, ~8bps VIP6; need VIP8+ ($250M+/mo) maker-taker to beat 4.6bps. The gap
  PERSISTS because all-in cost to arb it ~= the gap. No free money.
- Perp-to-perp (Binance-fut vs HL): dislocations DO NOT decay (half-life pinned at
  the 10s cap) - persistent basis driven by funding, "not latency arb, it's basis
  trading". BTC perp basis ~2-3bps, no reversion to zero. (Confirms our kill.)
- Only transient capturable lag = THIN coins (DOGE/SOL/SHIB/AVAX) in bursts of
  5-20bps - still a latency-desk game, wide spreads, slippage.

## Why we did NOT run our own multi-venue lag scan
It would only re-derive the Arrakis result (Binance leads HL ~700ms) which we
already cannot trade. Spending box time to re-confirm an untradeable number is
churn. If we ever want it for completeness it is a half-day job on CHD trade data.

## Decision
Close the lag/lead avenue. Pivot to TYPE C (forced flow) = liquidation cascades,
which do NOT require winning a latency race - a liquidation is a forced market
order whose AFTERMATH (overshoot + snap-back) plays out over seconds-to-minutes,
inside our cost/speed budget.

## Connected
[[lead-lag-study]] | [[MOC-Edge-Hunt]] | [[MOC-Decisions]] | [[data-apis]]