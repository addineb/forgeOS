---
tags: [design, subspace, basis-reversion, engine]
type: design
---
# Lag-subspace design + DATA CONTRACT (branch: lag-subspace)

Goal: test the basis-reversion pulse (HL perp vs Binance, net +8.55bps/trade in the
spread-aware pre-study) under ENGINE-GRADE fills (real HL spread/slippage + order
latency + adverse selection), then run the honesty gates. Built on a git BRANCH so
the proven main engine is untouched until this earns its place.

## DATA CONTRACT (ground truth - the thing I got wrong before; never assume again)
cryptohftdata raw parquet timestamp scales, VERIFIED by inspection:
- trade_time    = MILLISECONDS (Binance trade venue time)
- event_time    = MILLISECONDS (orderbook / liquidation venue time)
- received_time = NANOSECONDS  (our capture time)
Converter rule (already correct in chd-to-parquet.py + forge-data convert.rs):
- ms fields used as-is into the ms parquet; forge-data multiplies ms*1e6 -> ns
  (engine is nanosecond-native). received_time //1e6 -> ms for capture/local.
- MY PYTHON BUG was dividing the already-ms trade ts by 1e6. The ENGINE PATH never
  had this bug (convert.rs exch_from_ms multiplies; null-edge + det-hash prove it).

## HL orderbook reality (verified)
- Every row is event_type="snapshot"; 40 rows per event_time = full top-20 bid +
  top-20 ask; ~6,555 snapshots/hour (~0.55s apart); $1 tick (~0.13bps).
- The OLD conv_hlquote collapsed this to BBO-on-change (the sparse 5-10s feed that
  caused all the confusion) AND had a stale-accumulation bug (in_snap never resets
  because all rows are snapshots). We ABANDON hlquote for the subspace.

## Engine integration (minimal core touch)
The engine carries ONE OrderBook in Ctx + every raw event via observe(). So:
- HL full L2  -> the engine BOOK (the instrument we TRADE). Fed as a correct
  incremental DELTA stream (see below) so OrderBook reconstructs HL top-of-book.
- Binance trades -> Trade events = the REFERENCE leg. The basis signal reads the
  last Trade price as Binance fair value (taker-only strategy, so Binance trades
  harmlessly pass process_trade with no resting makers).
- New strategy = BasisReversion EntrySignal in forge-strategy: observe() tracks
  ref_px (Binance) + HL microprice (top-N depth from book), keeps a rolling-N gap
  baseline, dev = gap - baseline; entry() fires reversion side when |dev|>thr.
  Execution (hold_ns, taker fills, fees, latency) handled by the existing shell.
- order_latency_ns models the HL execution delay (adverse selection) the Python
  study could NOT - THE key new realism. feed_latency_ns models feed delay.

## Converting HL snapshots -> a correct delta stream (avoids the staleness bug)
For each event_time (a full fresh top-20 book): diff vs the carried book ->
emit change for new/changed levels and qty=0 remove for levels that dropped out.
Result: the engine incremental book == the latest HL top-20 at all times. Emitted
in the existing bookDelta schema (venueTs=event_time ms, captureTs=received_time
//1e6 ms) under ticks/<coin>/hlbook/<date>/<hh>.parquet.

## Build order (each step verified before the next)
1. [data] chd-to-parquet.py: add hlbook delta stage. VERIFY reconstructed top-5
   microprice == the snapshot microprice from basis_multiday.py (ties engine data
   to the validated pre-study).
2. [data] forge-data convert.rs: add hl_book stream (BookDelta events from the
   hlbook dir) + Binance trade; build a subspace *.forge. Roundtrip/checksum gate.
3. [strategy] BasisReversion EntrySignal + bot in forge-strategy (+ unit tests).
4. [gate] null-edge on the basis stream (coinflip must lose); knob-bite (thr moves
   trades); shuffled-direction control.
5. [sweep] wire into forge-sweep; sweep thr/hold/latency across all days; DSR/PBO.
6. [gate] EUR500 paper gate (20x, 20% size) with realistic latency.
A net-positive verdict counts ONLY after engine-grade fills + these gates.

## Open realism flags (track, don't hand-wave)
- Single feed_latency applied to both venues; real Binance vs HL feed latencies
  differ. Refine later with per-venue latency.
- HL top-20 depth cap: size impact for ~0.13 BTC (EUR500x20x) is within L1 usually;
  the engine walks the book so impact is modelled to the depth we have.
- Funding negligible at ~2min holds; revisit if holds lengthen.

## Connected
[[lead-lag-study]] | [[lag-avenues-study]] | [[MOC-Engine]] | [[MOC-Decisions]]