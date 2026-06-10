---
tags: [research, type-c, forced-flow, liquidations, weak]
type: research
---
# Liquidation cascade fade (Type C) - WEAK (2026-06-11 pre-study)

Data: Binance futures BTCUSDT liquidations (forceOrder stream), ~1,893/day over 8
days. SELL liq = forced selling (longs blown), BUY = shorts squeezed. Signal =
trailing same-side liquidation qty over W sec crosses a high quantile (cascade);
fade it (buy forced selling). Price = Binance trades. Idealized fills.

## Verdict: marginal. Only the biggest bursts, long hold, pay - on tiny samples.
- Fade is the right side (momentum mirror is net-negative everywhere).
- Most thr/horizon: gross +1-4bps but NET-NEGATIVE after 11bps.
- ONLY net-positive corner: top-1% bursts (q0.99, ~16-25 BTC in 5-10s) faded over
  H=300s (5min): net +3.9 to +4.6bps, win 59-63% - but N=22-27 only.
- Could be confounded: huge liqs cluster in big directional moves; the 5min "fade"
  may be capturing general mean-reversion, not the liquidation mechanism.

## Compared to basis-reversion (same day)
Basis-reversion is FAR stronger: net +8.55bps/trade AFTER spread+fees, ~9/day,
positive ALL 6 days, clean knob-bite. Cascade fade is a faint maybe on tiny N.

## If revisited later
- HL/Bybit/OKX liq streams 404'd under guessed names; Binance-only here. Multi-venue
  aggregated liq cascades (true forced-flow size) might be stronger - needs converter
  work + correct stream names. Park unless basis-reversion stalls.
- Better trigger: liq burst CONDITIONED on book state (thin book + cascade) ->
  Type B x C combo. Park as an idea.

## Connected
[[lag-avenues-study]] | [[lead-lag-study]] | [[MOC-Edge-Hunt]] | [[MOC-Decisions]]