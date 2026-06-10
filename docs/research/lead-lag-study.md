# Lead-lag study - VERDICT: BLOCKED on data (do not build yet)

Question: does Binance lead Hyperliquid by enough (after latency + fees) to trade?
Method: before any engine work, check the raw HL quote density vs Binance.

## Finding (decisive)
- Binance BTCUSDT trades: ~200k rows/hour (continuous, sub-second).
- Hyperliquid hlquote: ~0.02-0.10 updates/SECOND = one quote every ~10-50s,
  SYSTEMIC across all dates/hours checked (/root/chd/data/ticks/BTC/hlquote).

A lead-lag edge lives at 50ms-few seconds. With HL sampled every ~20s we cannot
measure or trade it. The cross-venue execution build (engine/fill surgery) is NOT
justified on this data.

## Why HL is sparse
Our captured stream is `hlquote` = reconstructed BBO, and the source delivers it
sparsely. We have no dense HL trades / book-delta feed.

## To unblock later (only if we still want lead-lag)
- Find a DENSE Hyperliquid feed: HL trades and/or book deltas (check if
  cryptohftdata offers them; our converter currently only does hlquote BBO), OR
- Capture HL live ourselves via websocket going forward (new data project).
- Then re-run this density check; if HL is sub-second, do the study, THEN build.

## Status
SHELVED. Pivot to the brain->strategy engine (does not need HL data). The cheap
study did its job: killed a hard build before it started.