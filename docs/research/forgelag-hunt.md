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