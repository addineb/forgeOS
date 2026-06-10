---
tags: [research, data, infra]
type: note
---
# Data APIs - raw feeds for the lag/basis engine + forced-flow

## STATUS: NOT DECIDED yet. Budget reality = ~EUR 500 account -> data spend ~$0.
- Tardis.dev = most COMPLETE (all specs, one schema, all venues) BUT ~$300/mo
  -> RULED OUT on cost (60% of the account/mo for data is absurd).
- No single CHEAP provider covers all specs. So the leaning is a FREE OWN STACK:
  * Coinalyze API (FREE) -> funding / liquidations / OI / basis (aggregated).
  * Hyperliquid S3 (FREE) -> HL L2 history (may revive basis-reversion).
  * Binance/Bitget free archives -> historical backfill.
  * SELF-CAPTURE multi-venue L2 on the box (FREE) -> ongoing aggregated granular
    L2 (forward-only; we already have the converter + .forge pipeline).
- Tradeoff of free stack: more engineering + forward-only for live-captured L2.
- Crypto Lake = cheap paid fallback for specific historical L2 slices if needed.
- DECISION IS THE TRADER'S. Nothing committed. Pick when ready.


DECISION: the lag/basis-reversion CLONE engine (its own git branch) will consume
AGGREGATED GRANULAR L2 (multi-venue book depth, not sparse quotes) + funding +
liquidations. This note is the sourcing research. (Compliance: vendor claims
paraphrased; verify pricing/tiers before paying.)

## Honest reality
Granular, historical, MULTI-VENUE L2 is the most expensive data type in crypto.
Truly free + granular + historical + multi-venue at quality does not exist. The
realistic path is: SELF-CAPTURE live (free, forward-only) + free archives for
backfill + one cheap paid API for forced-flow.

## L2 ORDER BOOK (granular) - for the basis/lag engine
- SELF-CAPTURE (FREE, best long-term): record Binance/Bybit/OKX/Hyperliquid
  public L2 websockets on the box. $0, but forward-only (no backfill) and we
  build/maintain the recorder. This is how we get OUR own aggregated L2.
- Hyperliquid S3 (FREE): HL publishes historical L2 BOOK SNAPSHOTS (market_data)
  + asset contexts via S3. Likely FAR denser than the cryptohftdata hlquote we
  used (~1/10-50s) -> may UNBLOCK proper basis-reversion data. CHECK cadence.
  (hyperliquid docs: historical-data / S3.)
- data.binance.vision (FREE): historical dumps - trades, aggTrades, klines, and
  futures bookTicker / bookDepth / metrics. Binance-only, depth is limited/
  snapshotted not full tick. Good for trades + as Binance backfill.
- Bitget data-download (FREE): candles, ORDER BOOK DEPTH, trades historical.
- Tardis.dev (FREEMIUM): incremental L2 as deep as the venue feed, 50k+
  instruments, normalized. FIRST DAY OF EACH MONTH free per exchange, no key ->
  great for cheap validation slices. Full history = paid (per-exchange, not cheap).
- 0xarchive.io (PAID?, promising): L2/L4 order books, trades, liquidations,
  funding, OI; PARQUET exports + WebSocket replay + REST. Parquet = our format.
  Pricing unverified - check; strong candidate if affordable.
- CoinGlass API: also exposes L2/L3 order books (see below).
- Heavy/institutional (expensive): Kaiko, Amberdata, CoinDesk Data (1-min book
  snapshots only - not granular), Laevitas (bulk parquet tick incl. L2).
- cryptohftdata: what we already use (byte-verified Binance bookDelta+trade + HL
  quote). Could extend, but HL quote was too sparse - prefer HL S3 for HL L2.

## LIQUIDATION / FUNDING / OI - for the FORCED-FLOW (Type C) leads
- CoinGlass API (CHEAP, recommended): liquidation history + heatmaps, funding
  rates, open interest, long/short ratio, aggregated across major exchanges;
  also L2/L3 books + websocket. From ~$35/mo (verify which tier includes
  liquidation history + L2 - cheap tiers are often limited). The single best
  cheap source for forced-flow data. (coinglass.com/pricing, docs.coinglass.com.)
- Binance (FREE): funding-rate history + OI via public API and
  data.binance.vision metrics; live liquidations via the forceOrder websocket
  (forward-only, Binance-only).
- Hyperliquid (FREE): funding + asset contexts via API/S3.
- 0xarchive.io / Laevitas: liquidations + funding in their parquet/REST products.
- Coinalyze, Velo, Amberdata: funding/OI/liq aggregators (freemium to paid).

## Recommendation (cheap-first stack)
1. FORCED-FLOW now: CoinGlass API (~$35/mo) for liquidations + funding + OI
   history (aggregated). Cheapest serious source; unblocks Type-C research.
2. HL L2: pull Hyperliquid S3 historical L2 snapshots (FREE) - test if dense
   enough to re-run the basis-reversion study properly (vs the sparse hlquote).
3. AGGREGATED L2: start SELF-CAPTURING multi-venue L2 on the box (free, forward)
   so history accrues; use Tardis free monthly samples + Binance/Bitget free
   archives for backfill/validation. Evaluate 0xarchive (parquet) if we need
   dense history without waiting.
4. Keep everything raw + replayable + gated. No vendor SIGNALS, only raw data.

## Next concrete step (when we open the lag branch)
- Verify HL S3 L2 snapshot cadence (could re-open basis-reversion immediately).
- Price-check CoinGlass tier (liq history + L2) and 0xarchive.
- Build a small multi-venue L2 websocket recorder on the box (free capture).

## ROUND 2 - cheapest options with all the specs
### Granular L2 history (multi-venue), cheap
- Crypto Lake (crypto-lake.com): historical ORDER BOOK (20 levels/side) + tick
  trades + OHLCV + OI + funding; 10 exchanges, top tokens; delivered as PARQUET
  on S3 (our format). Pay-per-use / relatively affordable. STRONG cheap candidate
  for granular L2 + funding/OI in one. (verify current pricing)
- Databento (databento.com): usage-based $/GB, ~$125 free credit, MBO/L3
  nanosecond. BUT crypto venue coverage is limited (built for futures/equities) -
  check if it covers our crypto venues before counting on it.
- 0xarchive.io: L2/L4 + liq + funding, parquet/REST (pricing unverified).
- Tardis.dev: free first-day-of-month samples; paid full history.
- Kuru.io: FREE daily L2 parquet snapshots on S3 (but Kuru = one DEX, niche).

### Forced-flow (liquidations / funding / OI / BASIS) - cheap/FREE
- Coinalyze (coinalyze.net): aggregated OI, FUNDING, LIQUIDATIONS, long/short,
  and BASIS across many exchanges, with an API. Free/generous-free tier (verify
  limits). CHEAPEST path to forced-flow + basis - start here at $0. Data is
  aggregated/candle-level, not tick L2.
- CoinGlass (~$35/mo): more granular liq history + heatmaps + funding + OI + L2/L3.
- Velo (velodata.app): clean per-exchange funding/OI/liquidations + premium/BASIS
  (incl. 3m annualised basis for BTC/ETH); API $199/mo - good but not cheap.

## REVISED cheapest stack (covers all specs)
1. FORCED-FLOW + BASIS, now, $0: Coinalyze API (funding/OI/liq/basis aggregated).
   Lets us study liquidation-cascade + basis context immediately for free.
2. HL execution-venue L2: Hyperliquid S3 (FREE) - check density (may revive the
   basis-reversion pulse with proper data).
3. GRANULAR multi-venue L2 history: Crypto Lake (cheap parquet) as primary; Tardis
   free samples + Binance/Bitget free archives for validation; self-capture on the
   box for ongoing aggregated L2.
4. Only pay for CoinGlass / Velo / Databento if a specific gap demands it.
Net: we can start the forced-flow + basis research for ~$0 (Coinalyze + HL S3 +
Binance archives), and add cheap Crypto Lake for granular L2 only when needed.