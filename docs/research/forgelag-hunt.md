---
tags: [forgelag, basis, result, milestone]
type: research
---
# forgelag 22-day hunt (2026-06-11): edge SIGNIFICANT at realistic latency

Dedicated forgelag engine (null-edge gate passes). 22 CONSECUTIVE days (2026-05-20
..06-10), full 24h, FRESH data, per-trade t-stat (the correct lens), EUR500/20x paper.

## Latency ladder (best config by |t|)  -- order latency = our exec delay to HL
| latency | t-stat | mean/trade | win% | trades | paper% |
|--------:|-------:|-----------:|-----:|-------:|-------:|
|   0ms   |  6.99  |  +5.1bps   | 59%  |  742   | +347   |
| 150ms   |  6.05  |  +6.8bps   | 60%  |  405   | +195   |
| 300ms   |  4.95  |  +5.7bps   | 58%  |  379   | +133   |
| 500ms   |  3.31  |  +6.0bps   | 56%  |  208   |  +63   |
| 700ms   |  1.35  |  +1.7bps   | 51%  |  343   |  +24   |
| 1000ms  | -2.44  |  -1.9bps   | 40%  |  685   |  -34   |

SHUFFLE control @300ms: t=-6.70 (decisively negative) -> direction carries real
information; no-fake-edge gate passes.

## Verdict: REAL + SIGNIFICANT at realistic latency, with a LATENCY CLIFF.
- t~5 at 300ms on 742 trades is strongly significant (vs the scattered 13-day
  sample's borderline t~1.3). More + consecutive + fresh data + clean engine.
- LATENCY IS MAKE-OR-BREAK: significant <=500ms (t>=3), gone by 700ms, INVERTS at
  1s. Tradeable only if we execute within ~300-500ms of Hyperliquid.

## Honest remaining gates (finite, not endless)
1. ROBUSTNESS ACROSS PERIODS: 22 days = ONE continuous regime. Re-run on different
   months (Dec/Jan/Feb) - independent samples, not one block.
2. MULTI-VENUE REFERENCE: this is vs Binance-spot only. Test okx/bybit + aggregated.
3. DRAWDOWN: the +133%@300ms is 20x leverage amplifying +5.7bps/trade; verify worst
   drawdown / ruin before trusting the leveraged number (t-stat is the honest core).
4. EXECUTION LATENCY in practice: measure our real round-trip to HL; the edge lives
   or dies on staying under ~500ms.

## Status: BEST LEAD BY FAR. Significant, direction-confirmed, engine-grade. Next =
robustness across periods + the latency-reality check.
## Planned expansions (ONLY after it holds OOS + drawdown is survivable)
- Variances: z-score trigger (dev/rolling-std) vs bps; exits = revert-to-mean vs
  fixed-hold vs TP/SL; baseline window/EWMA; microprice depth.
- Venues: HL vs OKX / Bybit / Binance-futures; AGGREGATED multi-venue reference
  (VWAP across venues) as fair value.
- CONDITIONING with the "dead" indicators (Type A): OFI/CVD/imbalance were no-edge
  ALONE, but may work as a CONFIRM/filter on the basis trigger (only fire the
  reversion when orderflow agrees / when the book isn't being run over). This is
  the trader-method-1 idea: structural trigger + orderflow confirm.
- Gate order stays: prove HOLDS (OOS months + drawdown/ruin) FIRST, then expand.
## OUT-OF-SAMPLE CONFIRMATION (2026-06-11): HOLDS on Feb (independent period)
Feb 5-18 (14 days), fresh, SEPARATE from the May-Jun training block:
- 300ms: thr10/2m t=4.77 (+4.3bps, 940 trades, win 53%, no ruin) - SAME significance
  as May-Jun (t=4.95). 0ms t=6.22; 500ms t=3.98.
- Two INDEPENDENT periods now significant at realistic latency + shuffle control
  negative + no ruin => edge is NOT a one-regime artifact.
- Feb more volatile: maxDD 18-26% (vs May-Jun 4.5%), still no ruin. Win 51-53%
  (wins on win-size, not hit-rate).
VERDICT: validated across periods. Strongest result in the project. Now justified
to expand (venues / variances / dead-indicator conditioning) + measure REAL exec
latency to HL (the deploy make-or-break) + consider live paper.
## MAKER (limit) entries TESTED -> FAIL (adverse selection). Edge is TAKER-ONLY.
Added proper maker fills to forgelag (queue position, fill only when HL tape trades
through; null-edge still passes). Re-ran market vs limit @ latency ladder (22d):
- TAKER (lim=false): t=6.8/4.97/3.59/1.36 @ 0/300/500/700ms (as before, positive).
- MAKER (lim=true): t=-8 to -11, win ~10%, NEGATIVE at EVERY latency.
Why: resting a bid to "catch the dip" fills ONLY when price keeps trading through it
(continuation) and misses the immediate bounces (the winners) = classic adverse
selection on a reversion. Providing liquidity = being on the wrong side here.
CONSEQUENCE: the edge REQUIRES crossing the spread (taker). Maker does NOT dodge the
latency problem; latency is the binding constraint. No free lunch.
REMAINING unknown = REAL taker signal->fill latency on HL (needs a tiny funded order
to measure; REST read-proxy ~230ms floor + block finality puts us on the cliff edge).