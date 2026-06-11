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
## REAL HL LATENCY (researched) + the deployable sliver (2026-06-11)
HL real order-to-fill latency (independent: Glassnode + HL docs + our ping):
- ~884ms median from AWS Tokyo, ~1079ms Ashburn VA; ~5ms network, rest server-side.
- HL docs: COLOCATED client median 0.2s, p99 0.9s (200ms only if physically colocated).
- Our box (Germany): ~7ms network but HL server-side dominates -> expect ~900-1100ms.
=> The FREQUENT small-stretch edge (thr10) is GONE/negative at real latency.

## BIG-DISLOCATION variant SURVIVES real latency (36 days, Feb+May-Jun)
thr>=20bps, ~3min hold, TAKER:
- @884ms: t=2.99, n=198 (~5.5/day), +7.7bps/trade, win 49.5%, RR 1.89, paper +80%, DD 10%.
- @1000ms: t=2.36, n=199, +6.1bps, win 47%, RR 1.86, paper +59%, DD 10.5%.
Why: a >=20bps gap reverts slow/far enough that ~900ms latency eats only a fraction.
Win rate ~48% (coinflip); profit is from RR ~1.9 (wins ~2x losses) = positive expectancy.

## VERDICT (honest): there IS a deployable sliver at REAL latency = the SLOW big-
## dislocation variant (>=20bps, 3min hold, ~5-6 trades/day). t~2.4-3.0 = significant
## but borderline (vs t~5 at 300ms fantasy). Needs: more periods to firm up + live
## validation. The small/frequent version is NOT deployable (latency kills it).
## IDEA MENU (latency is the binding constraint -> prioritise ideas that DON'T need speed)
1. ANTICIPATE not react: trigger on basis VELOCITY (stretching fast), so our order
   lands ~900ms later AT the extreme. Leads our own latency. [highest value]
2. Z-SCORE trigger: threshold = k * recent basis-vol (not fixed bps) -> steady trade
   frequency across calm/volatile regimes; may give thr10 frequency w/ thr20 survival.
3. EXIT on revert-to-mean (not fixed time); asymmetric: tight stop if gap widens
   (structural), let reverters run.
4. CONDITION entry: funding regime (extreme funding = crowded = snaps back) and/or
   book-pressure confirm (the Type A dead-indicator idea); time-of-day filter.
5. REFERENCE leg: Binance FUTURES (perp-perp) vs spot; aggregated multi-venue VWAP.
6. SIZE proportional to dislocation magnitude (more $ on fattest/highest-expectancy).
Picks to spark: #1 (defeats latency) + #2 (restores frequency to latency-proof trades).
## VARIANCE TESTS (2026-06-11, @884ms real latency, 36 days)
- VELOCITY GATE (#1): no effect (small dislocations decay < latency; not a timing problem). REJECTED.
- REVERT-TO-MEAN EXIT (#3): WINNER. Exit when basis returns to ~mean (|dev|<=2bps)
  instead of fixed hold. thr15: t=4.01, 446 trades (~12/day), win 48%, RR 2.00,
  paper +57%, maxDD 6.9%. thr20: t=3.92, RR 2.23, maxDD 5.8%. Cuts losers early
  (avgL 0.20->0.06) + stops giving back profit -> higher t, more trades, lower DD
  than fixed-hold. BEST deployable config at real latency.
- Z-SCORE TRIGGER (#2) k=3: DISASTER (t=-37, ~360 trades/day on noise). Bar too low;
  recreates the latency-killed small-dislocation problem. REJECTED (as-is).
- CAVEAT: entry refactor to sampled-dev (for velocity work) shifted baseline numbers
  -> signal has some implementation sensitivity; revert-exit result is robust though.

## CURRENT BEST DEPLOYABLE: big-dislocation (>=15bps) + REVERT-TO-MEAN EXIT.
@ real HL latency (884ms): t~4, ~12 trades/day, RR 2.0-2.2, +43-57% paper/36d, DD ~6-7%.
Next variances to try: funding/book CONDITION (#4), magnitude SIZING (#6), ref leg (#5).
## BOOK-PRESSURE CONFIRM (#4, Type-A) WORKS on the selective variant (@884ms, 36d)
Only fade when HL book AGREES (bid-heavy to buy dip / ask-heavy to sell rip).
thr20 + revert-exit: no-confirm t=3.92/win53.5/RR2.23/DD5.8 -> +confirm(imb>=0.2)
t=4.81/win57.9/RR2.31/DD5.3 (~43% paper either way; filters ~15% lower-quality trades).
Helps SELECTIVE (thr20) not FREQUENT (thr15 slightly worse). The dead imbalance
indicator EARNS ITS KEEP as a confirm on a structural trigger (validates method-1).
=> Quality win (higher t/win, lower DD), not a return boost. Euro ~715 on 500/20x/36d.

## BEST CONFIG SO FAR (real latency, honest): thr>=20bps + revert-to-mean exit +
## book-confirm(imb>=0.2): t=4.81, ~4-5 trades/day, win 58%, RR 2.3, DD ~5%, +43%/36d.
Next: magnitude SIZING (#6, weight fattest gaps -> boost euro), ref-leg swap (#5).
## MAGNITUDE SIZING (#6): NEGLIGIBLE. paper 42.9->41.7% (thr20), t 4.81->4.75; within
## the big-dislocation bucket trades are similar size so weighting barely redistributes,
## and bigger orders eat a touch more slippage. REJECTED. Euro is FREQUENCY-capped, not
## sizing-capped. Real scaling levers = breadth (multi-asset) + ref-leg (#5), not sizing.
## MULTI-ASSET (#breadth): ETH is MUCH STRONGER than BTC (2026-06-11, @884ms real latency, 36d)
Same config (spot ref, revert-to-mean exit), HL-ETH-perp vs Binance-ETH-spot:
- thr20, no confirm: t=7.95, n=549 (~15/day), win 62%, RR 1.77, paper +283% (EUR500->~1915), DD 7.4%.
- thr15: t=6.97, n=1191 (~33/day), win 54%, +353% (~2265), DD 12.8%.
- thr30: t=5.12, win 66%, +122%, DD 10.2%.
- + book confirm: slightly LOWERS return (thr20 +205%) - same quality/euro tradeoff as BTC.
vs BTC thr20 (t=4.81, +43%): ETH ~6-8x the return - more dislocations (thinner on HL,
lags Binance more) + higher per-trade edge + higher win. Sanity-checked: +283% =
frequency x edge x 20x compounding (not a bug).
=> BREADTH is the real euro-scaling lever. Best ETH: thr20+revert-exit (t~8, +283%, DD7%).
SOL pending (downloading). NEXT: run SOL; consider a multi-asset portfolio (BTC+ETH+SOL).
## SOL: weak/risky (basis sane -7bps, prices align). Best thr15 t=2.16 +78% but DD 27.8%.
## SOL too volatile -> directional noise swamps reversion at latency. Exclude (or tiny weight).

## CROSS-ASSET + PORTFOLIO (36d, 884ms real latency, revert-exit thr20, no confirm)
| asset | trades | return | maxDD | EUR500-> |
| BTC   |  187   |  +43%  |  4.8% |  715 |
| ETH   |  549   | +283%  |  7.4% | 1915 |
| BTC+ETH 20% each | 736 | +448% | 9.6% | ~2742 |
| BTC+ETH 10% each | 736 | +135% | 4.9% | ~1177 |
ETH is the engine; BTC adds diversification (DD rises only 7.4->9.6 combined = trades
largely independent). CAVEAT: portfolio paper compounds trips SEQUENTIALLY, does NOT
model concurrent open positions -> at 20% each, two simultaneous = 40%+ margin, real
concurrent DD higher than shown. PRUDENT read = 10% each (+135%, DD 4.9%). Still
backtest @884ms idealized fills. lag-hunt has --dumptrips for portfolio export.
## HYPE (HL native token) test (18 days so far, @884ms real latency)
Basis correctly computed (mean -13bps, prices align HL~$50 vs Bybit~$50). Edge WEAK:
- Only thr30 + reversion positive: t=2.29, n=202 (~11/day), win 50.5%, RR 1.59, +25%, DD 5.6%.
- All smaller thresholds NEGATIVE; momentum side strongly negative (t=-15..-86).
Causes: (1) Bybit-spot HYPE reference THIN (~10.9k trades vs BTC ~480k) -> noisy anchor,
small gaps = noise; (2) HL likely LEADS price discovery for its OWN token (basis
persistently -13bps, reversion-to-external weak) = hypothesis partly confirmed.
ASSET RANKING (884ms real latency): ETH(t~8) > BTC(t~4.8) >> SOL(t~2,DD28) ~ HYPE(t~2,thin).
Edge is strongest where HL LAGS a LIQUID external spot; weak where ref is thin or HL leads.
=> HYPE is the prime case for improvement #1 (AGGREGATED Bybit+OKX spot reference) to
de-noise the anchor. Next build: aggregated multi-venue reference.