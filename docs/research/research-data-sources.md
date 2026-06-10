# Research Note -- Data Sources (capture & vendors)

The standing dependency across the lab: most verdicts are capped by ~7h of
3-stream history (2026-06-06 05:00-12:00 UTC). More multi-stream data is the
single biggest unblock. Options:

## Historical L2 depth + trades (fixes the backtest-capture gap)
- Tardis.dev -- primary. Full-depth book + trades, tick-level, Binance/Bybit/
  OKX/etc; replay-friendly (suits our harness). Best fit.
- Kaiko -- institutional, broad, pricier.
- Amberdata -- enterprise; derivatives + DeFi.
- CoinAPI / CryptoLake -- cheaper alternatives, coverage varies.

## Liquidations / open interest / derivatives (for the liquidation bot, L0)
- Coinglass -- liquidation heatmap + aggregated liquidation/OI (estimated-cluster
  source).
- Hyblock Capital -- liquidation API + leverage/positioning.
- purrdata.io -- Hyperliquid-specific liquidation data (our execution venue).
- NATIVE (free, = a capture build not a subscription):
  Binance `btcusdt@forceOrder` + open-interest endpoints; Hyperliquid liquidation
  feed. This is the L0 capture task.

## Recommendation
- Tardis.dev to backfill multi-stream history (lifts the DSR/OOS ceiling).
- Native Binance/HL feeds (or Coinglass/Hyblock) for the liquidation bot.
- If avoiding spend: native exchange streams cover both gaps -- it becomes a
  capture build instead of a subscription.