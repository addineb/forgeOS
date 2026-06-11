---
tags: [research, lag, improvements]
type: research
---
# Lag-venue HARD RESEARCH (2026-06-11): improvements to add to forgelag basis-reversion

Sources: themicrostructurelab (cross-exchange fair value, arb study, alpha atlas),
SSRN lead-lag microstructure, ml4trading/OneKey funding arb, Hyperliquid docs.

## Ranked improvement candidates (untested unless noted)
1. AGGREGATED MULTI-VENUE FAIR VALUE [TOP -> TESTED 2026-06-11: REJECTED for BTC/ETH; t-stat flat-to-down vs single Binance, only adds drawdown. Kept as option for thin-ref assets (HYPE). See [[forgelag-hunt]]]. "Combining Multiple Orderbooks" study:
   a GLOBAL fair price from 6 exchanges predicts next move better than any single
   venue; cross-exchange dislocation carries info. We only tested single-venue refs
   (Binance spot > Binance futures). ADD: reference = VWAP/weighted mid across
   Binance+OKX+Bybit spot. Cleaner anchor -> better/more dislocations. Needs OKX/
   Bybit spot data per asset.
2. FUNDING-CONDITIONING. Extreme funding -> predictable mean-reversion (ml4trading,
   OneKey). Our basis IS funding-driven. ADD: gate/boost entries on funding extremes.
   HL funding settles hourly (easy pull).
3. CROSS-ASSET LEAD. SSRN: one asset's trades+imbalance predict another's midpoint;
   BTC leads alts. ADD: condition ETH/alt basis on BTC move.
4. VALIDATION: atlas confirms microprice + book imbalance are THE top short-horizon
   features = exactly what forgelag already uses. Core is well-founded.

## Reconfirmed limits
- Lead-lag is a latency race (Binance leads HL ~700ms; HL order-to-fill ~884-1080ms).
  Our edge survives ONLY because basis reversion is minutes-scale, not a sub-second race.
- Spot is a better anchor than perp (perp-perp basis is funding equilibrium, tighter/noisier).

## HYPE data (for the next test)
- HL-perp HYPE: book+trades YES. HL-SPOT HYPE: not in CHD under that name.
- Binance SPOT HYPE: NO. Binance FUTURES HYPE: yes. Bybit/OKX SPOT HYPE: yes (trades).
- So HYPE reference = Bybit/OKX spot (forces the aggregated-ref path).
- HYPOTHESIS TO TEST: HYPE is HL's OWN token -> HL may LEAD price discovery, so the
  basis-reversion (HL reverts to external) could be WEAK or REVERSE. Empirical test.

## Connected
[[forgelag-hunt]] | [[lag-avenues-study]] | [[MOC-Edge-Hunt]]