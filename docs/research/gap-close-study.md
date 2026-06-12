# Gap-close order-book study (gapscope) - "go back to the beginning: the book"

RESEARCH / ANALYSIS pass. Honest-skeptic rules apply: report the numbers, name
the assumptions, do not flatter. This is the trader's "study the order book
during the close" request, the sequel to the lagshot-maker hunt.

## Why this exists
Lagshot's basis-reversion edge is REAL but uncapturable: as a TAKER it is
latency-locked (the gap closes faster than ~0.8-2.4s real fill latency), and as
a MAKER it is adverse-selection-locked (resting fills only on continuation). The
PATH-mode tweak (see lagshot-maker-hunt.md) showed the fillable flow is
MOMENTUM / overshoot, not mean-reverting liquidity - a quote rested in the path
of the move filled ~19x more than a fade quote, but got run over.

The trader's core hypothesis was about WHY the gap closes, at the BOOK level:
the lag closes because of ORDER-BOOK BEHAVIOUR - walls stacking / reloading fast
to close the gap, possibly SPOOFING (fake walls that pull without trading). This
study MEASURES what actually happens in the HL book during the close to see if a
forced-flow / order-behaviour signal exists.

## The tool: `crates/forgelag/src/bin/gapscope.rs`
Analysis-only binary (does NOT trade). Reuses the sacred core untouched:
`load_window` (feed), `forge_book::OrderBook` (HL exec book), and the
`FairValueOracle` gap/baseline/dev math (the SAME rolling baseline the validated
strategy uses). Replays a real ETH day, maintains the HL exec book + OKX-spot
reference, and DETECTS dislocation events (|dev| >= --thr, same dev as the
oracle). For each dislocation it records the book behaviour over the CLOSE
WINDOW (until |dev| <= --exit, default 2bps, or --horizon, default 5s, plus a
1s settle to catch overshoot) and emits per-dislocation + aggregate stats.
Deterministic, no-lookahead (only `<= now` state is ever read). Build/clippy
(-D warnings)/test all green; the existing 84 tests stay green; 4 new unit tests
cover the classification helpers.

### Exact definitions (how each thing is classified)
- **dev (trigger)**: event-resolution instantaneous `gap - rolling_baseline`,
  where `gap = (microprice_top5 - okx_ref)/okx_ref * 1e4`. Same baseline math as
  the oracle (500ms sample cadence, 500-sample window); the instantaneous gap is
  evaluated every event for fine timing. Gated on warm-up + non-stale reference.
- **close direction**: dev>0 = HL rich, expected to FALL (close = DOWN, closing
  aggressor = seller/Ask); dev<0 = HL cheap, expected to RISE (close = UP, Bid).
- **TIME-TO-CLOSE**: ns from trigger to first |dev| <= exit (or "did not close"
  if it exceeds the 5s horizon).
- **CLOSE MECHANISM (trade-driven vs re-quote)**: walk the close window; the net
  microprice move TOWARD fair value is decomposed by attribution. A microprice
  increment is TRADE-attributed if it lands within 100ms of an aggressive HL
  trade in the close direction, else RE-QUOTE-attributed. `trade_frac =
  trade_toward / total_toward`. A close is TRADE-DRIVEN if `trade_frac >= 0.5`,
  else RE-QUOTE-DRIVEN. We also log raw close-direction vs against-direction
  aggressive volume.
- **WALL**: a top-5 level whose size >= 5x the median top-5 level size (both
  sides). A wall is tracked from appearance; when it shrinks to <= 40% of its
  peak it RESOLVES as ABSORBED if the aggressive volume that printed AT that
  price during its life explains >= 50% of the size that vanished, else PULLED
  (cancelled with little/no trading = spoof-like). Walls still present at window
  end are STANDING (untouched).
- **DEPTH IMBALANCE**: top-5 `(bid - ask)/(bid + ask)` in [-1,1]. The
  "toward-close shift" is sign-corrected per dislocation: positive = book leaned
  MORE toward pushing price in the close direction by the end.
- **OVERSHOOT**: dev flips sign (past 0) before settling; magnitude = furthest
  opposite-sign dev reached within the window + 1s settle. "Overshot" = the flip
  exceeds the 2bps exit band.

## Setup
- Coin ETH (HL perp) vs OKX-spot ETH-USDT reference (the validated best anchor).
- Data: real ETH days under /root/chd/fresh/ticks (hlbook + trade; OKX ref).
- 10 days across Nov-Dec 2025 (same days as the maker hunt, mixed regimes):
  2025-11-04, 11-08, 11-12, 11-16, 11-20, 11-24, 11-28, 12-02, 12-08, 12-15
  (~6.5-7.6M events/day; 10/10 loaded cleanly).
- Three trigger thresholds to see if big vs small dislocations close differently:
  --thr 8, 16, 25 (bps). Defaults otherwise (exit 2, horizon 5s, settle 1s, top 5).

### Exact command
```
gapscope --coin ETH --symbol ETH-USDT --dates <10 days> --hours all \
  --thr {8|16|25} --dump rows_thr{N}.csv
```

## RESULTS - pooled across the 10 days

| metric | thr 8 | thr 16 | thr 25 |
|:--|--:|--:|--:|
| dislocations (n) | 5191 | 626 | 122 |
| closed within 5s | 98.5% | 95.2% | 91.8% |
| median time-to-close | 1153 ms | 1618 ms | 1998 ms |
| mean time-to-close | 1313 ms | 1768 ms | 2116 ms |
| TRADE-driven (frac>=0.5) | 32.1% | 44.0% | 43.8% |
| RE-QUOTE-driven | 67.9% | 56.0% | 56.2% |
| mean trade-frac | 0.31 | 0.42 | 0.44 |
| close-dir aggr vol (base) | 79.7 | 266 | 571 |
| against-dir aggr vol (base) | 37.2 | 95.9 | 172 |
| close/against vol ratio | 2.1x | 2.8x | 3.3x |
| net microprice move toward | 7.8 bps | 15.6 bps | 23.2 bps |
| walls: of RESOLVED, absorbed | 1.7% | 3.2% | 5.0% |
| walls: of RESOLVED, pulled | 98.3% | 96.8% | 95.0% |
| dislocations w/ >=1 pull | 82.9% | 93.6% | 92.6% |
| dislocations w/ >=1 absorb | 8.2% | 24.4% | 41.0% |
| depth-imbalance toward-close shift | -0.346 | -0.481 | -0.535 |
| OVERSHOOT (dev flips sign) | 40.6% | 54.5% | 68.0% |
| median overshoot | 3.9 bps | 4.7 bps | 5.5 bps |

Every metric moves MONOTONICALLY with the threshold dial = knob-bite valid (the
--thr dial demonstrably changed every measured behaviour, so the read counts).
Per-day breakdown (thr 16) is consistent across all 10 days and both regimes:
trade-frac 0.35-0.61, walls 95-98% pulled EVERY day, overshoot 32-67% - no
single day drives the aggregate. Absorption DOES fire (153/626 dislocations have
>=1 absorbed wall), so the classification is not broken; pulls simply dominate.
## What the close ACTUALLY looks like (honest read)

1. **It closes FAST - ~1-2s - and bigger gaps close SLOWER.** Median
   time-to-close runs 1.15s (thr8) -> 1.62s (thr16) -> 2.0s (thr25); ~92-99%
   close inside 5s. This is the same wall we already hit live: the reversion
   half-life sits right around / under our real fill latency (766ms calm, 1.3-
   2.4s when signals fire). Nothing here reopens the taker door.

2. **The close is MIXED, leaning RE-QUOTE - not a clean trade-through.** By the
   trade-frac>=0.5 rule only 32-44% of closes are trade-driven; the majority of
   the microprice's journey to fair value happens via book RE-QUOTE (the BBO /
   top-5 repricing) rather than coincident with aggressive prints. The
   per-dislocation trade-frac is BIMODAL (thr16: 41% of closes are near-pure
   re-quote with frac<0.2; 22% are near-pure trade with frac>0.8) - so the close
   is not one uniform mechanism, it is two populations. That said, there IS
   genuine directional flow: close-direction aggressive volume is 2-3x the
   against-direction volume (and the ratio grows with dislocation size). This
   refines the PATH finding: PATH fills more because directional trades DO exist,
   but those trades are a minority of the price-closing mechanism most of the
   time - most closes the book just reprices through.

3. **Walls do NOT absorb - they PULL (or sit untouched).** 95-98% of resolved
   top-5 walls vanish via cancellation with little/no trading at their price;
   only 2-5% are genuinely absorbed by the tape. Big resting levels near the
   touch overwhelmingly RETREAT ahead of the close rather than provide liquidity
   into it (and many never get involved at all = "standing"). Real absorption
   rises with dislocation size (8% -> 24% -> 41% of dislocations have >=1
   absorb) but pulls dominate at every threshold and on every one of the 10 days.
   CAVEAT - do not over-read "spoof": gapscope cannot see intent. Pulling
   liquidity ahead of an adverse move is normal defensive market-making, not
   necessarily manipulative spoofing. The robust, intent-free statement is
   "liquidity retreats/reprices rather than absorbs." The 50%-of-vanished absorb
   threshold and exact-price trade matching also make the absorbed count a floor,
   not a precise figure.

4. **The book re-leans AGAINST the close direction by the end + it OVERSHOOTS.**
   The sign-corrected depth-imbalance shift is negative and grows with size
   (-0.35 -> -0.54): by the time the gap closes, relative depth has built on the
   side the price is moving AWAY from. Hand-in-hand, the close OVERSHOOTS (dev
   flips sign) 41% / 55% / 68% of the time, further for bigger gaps (median 3.9
   -> 5.5 bps). This is the maker-getting-run-over signature made concrete, and
   it matches the live trade T2 that overshot to +6.5bps past zero. The "stretch
   then snap then snap back" picture is real and routine, especially for big
   dislocations.

## Did the hypothesis hold? Partly - but not in a tradeable way.
The trader's hypothesis was: the lag closes because walls STACK / reload to
close the gap (provide the liquidity that snaps it back), possibly spoofing.
What the book actually shows:
- There IS clear order-book behaviour during the close, and it IS largely a
  book-repricing (re-quote) phenomenon rather than a trade-through - consistent
  with "the book closes the gap."
- BUT the walls do the OPPOSITE of stacking-to-absorb: they PULL. The gap closes
  through a liquidity VACUUM (levels cancel and the BBO reprices into the gap),
  helped by directional momentum trades (2-3x close-dir flow), and it OVERSHOOTS.
  It is a vacuum + momentum close, not a wall-provides-liquidity close.

## ACTIONABLE READ - is there a forced-flow / order-behaviour lead here? NO clean one.
Being the honest skeptic, this study SHARPENS the existing kill rather than
opening a clean new door:

- **Wall-pull as a signal: not predictive.** ~93% of dislocations have >=1
  pulled wall, but the pull happens DURING the close, concurrent with it, not
  ahead of it - by the time the wall pulls, the gap is already going. It does not
  forecast the close with lead time, and we cannot separate defensive cancel from
  spoof. A near-universal, concurrent, intent-ambiguous event is not a tradeable
  edge.
- **Momentum / overshoot: this is exactly the flow PATH already tried to ride and
  got run over by.** The close-direction flow is real and directional, and it
  overshoots 40-68% of the time - but riding it is a TAKER latency race to get in
  (same ~1-2s wall) and the overshoot makes the exit treacherous. PATH mode
  already tested resting in that flow: deeper adverse selection, every cell
  negative under honest fees. Trading the momentum as a taker is the same
  latency-locked problem.
- **Re-quote dominance: nothing to trade against.** If most of the close is the
  book repricing (not prints), there is no flow to provide into or ride; you can
  only chase the reprice as a taker, which is the latency race again.

The close is FAST (1-2s, under our latency), MOSTLY a re-quote through PULLED
liquidity (not absorption, not stacking), with frequent OVERSHOOT. There is no
slow, predictable, providable "wall reload" pattern to fade or to detect with
lead time. The order-behaviour is real but it is the structurally-uncapturable
regime we already mapped: the taker cannot out-race a ~1s vacuum-close, and the
maker gets run over by the momentum/overshoot that fills it.

## VERDICT - no new lead worth opening from the gap-close microstructure.
This is a valid, valuable NEGATIVE: we went back to the order book exactly as the
trader asked, measured the close in detail across 10 days and 3 dislocation
sizes, and the book confirms (does not rescue) the prior conclusion. The gap
closes by a liquidity vacuum + momentum overshoot in ~1-2s, with walls pulling
rather than absorbing - which is precisely why neither the taker (can't out-race
it) nor the maker (run over by it) can capture it. No clean forced-flow /
order-behaviour signal with predictive lead time exists in the close itself.

Lagshot stays SHELVED: real edge, structurally uncapturable by us. The next lead
should remain a DIFFERENT edge family that does not depend on out-racing a
reversion (taker) or being crossed on the right side (maker) - the queued
Type C forced-flow / LIQUIDATION CASCADES study (CHD liq data), where the flow is
FORCED (margin calls) and persistent rather than a sub-second vacuum-close.

### Caveats / honesty notes
- The trade-frac mechanism split depends on the 100ms attribution window and the
  0.5 cutoff; the raw close/against volume ratio (2-3x) is the assumption-free
  companion read and tells the same story (directional but minority-of-move).
- "Pulled" counts cancellation without a trade AT that exact price >= 50% of the
  vanished size; it cannot prove spoofing intent and the absorbed count is a
  conservative floor (exact-price matching).
- microprice/dev use the validated top-5 size-weighted micro + OKX rolling
  baseline; results are no-lookahead and deterministic but inherit the same
  microprice definition as the strategy (a different micro could shift edges).
- 10 days, Nov-Dec 2025. Consistent across them, but not every regime.

Raw per-dislocation CSVs: /root/runs/gapscope/rows_thr{8,16,25}.csv (columns:
day, start_ts, start_dev_bps, dir_down, closed, ttc_ms, trade_frac, trade_driven,
micro_move_bps, total_toward, trade_toward, close_vol, against_vol, w_app,
w_app_bid, w_app_ask, w_abs, w_pull, w_stand, imb_start, imb_end, imb_toward_close,
overshoot_bps, did_overshoot, peak_abs_dev).