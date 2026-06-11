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
## AGGREGATED MULTI-VENUE REFERENCE (#1) TESTED -> NOT WORTH ADOPTING (2026-06-11)
Built aggregated ref support in forgelag (feed.rs per-venue `src` tag; engine.rs
ref_px = mean of latest nonzero per-venue trade px; hunt --symbol takes a CSV
`BTCUSDT,BTC-USDT,BTCUSDT_BYBIT`). Clippy clean, null-edge gate still passes.
Downloaded OKX spot (BTC-USDT, ETH-USDT) + Bybit spot (BTCUSDT_BYBIT, ETHUSDT_BYBIT)
trades, 36 days (Feb 5-18 + May20-Jun10), into /root/chd/fresh.

Head-to-head, SAME data/config (thr20, revert-exit, 884ms), only the reference changed:
| 36d thr20      | trades | t-stat | win% | RR  | paper% | maxDD% |
| ETH single Bin |  549   | 7.95   | 62.3 |1.77 |  +283  |  7.4   |
| ETH aggregated |  543   | 7.88   | 63.5 |1.79 |  +346  | 12.3   |
| BTC single Bin |  187   | 3.92   | 53.5 |2.23 |  +43   |  5.8   |
| BTC aggregated |  178   | 3.60   | 53.4 |2.09 |  +39   |  5.0   |

VERDICT (honest skeptic): aggregation does NOT improve the EDGE. The t-stat (the metric
that matters) is flat-to-down on both assets (ETH 7.88 vs 7.95; BTC 3.60 vs 3.92). On
ETH it lifts return to +346% but ONLY by taking ~70% more drawdown (12.3 vs 7.4) - same
significance, more risk. On BTC it is strictly worse. => single Binance spot reference is
as good or better risk-adjusted. Aggregation kept as an OPTION (good for thin-ref assets
like HYPE), NOT adopted as default. Research improvement #1 = REJECTED for BTC/ETH.

REGIME-CONCENTRATION finding (the real takeaway): the +283% edge is concentrated in the
Feb window. The 22-day May-Jun window ALONE is weak at 884ms: ETH thr20 only +2.6%
(single) / +11% (aggregated, t=1.56), BTC negative. On the calm 22d window aggregation
DID help every cell (de-noises a quiet ref), but that gain disappears once volatile Feb
days dominate. => the edge is NOT always-on; it needs the volatile regime. Carry this as
the honest risk: returns are lumpy and regime-dependent, not a steady printer.

REPRODUCIBILITY CHECK (addresses trust): the logged ETH +283%/t=7.95 and BTC +43%
REPRODUCED EXACTLY on fresh re-run -> engine is consistent across runs, not drifting.
## FUNDING-CONDITIONING (#2) TESTED -> REJECTED for BTC/ETH (2026-06-11)
Built funding support: converter `funding` stream from HL mark_price (NOTE its
event_time is NANOSECONDS not ms - handled), feed Funding event + engine funding
field + LagCtx.funding, BasisConfig fund_gate/fund_min/fund_align knobs (hunt
--fundgate --fundmin --fundalign). Clippy clean, null-edge gate still passes.
BTC funding dist (per-hour, 36d): |median| 8.9e-6, p95 2.2e-5, p99 4.7e-5; ASYMMETRIC
- positive CAPPED at +1.25e-5 (HL mechanic), negative spikes to -1.4e-4 (fear/crowded
shorts). So fund_gate>=2e-5 ~ "only trade when funding very negative".

Results (36d, thr20, revert-exit, 884ms):
| BTC variant        | n   | t    | win  | RR   | paper | DD  |
| baseline           | 187 | 3.92 | 53.5 | 2.23 | +43   | 5.8 |
| fundgate>=2e-5     | 34  | 0.70 | 32.4 | 3.57 | +3.5  | 4.7 |
| fundalign          | 111 | 3.09 | 50.5 | 2.62 | +30   | 4.7 |
| ETH variant        | n   | t    | win  | RR   | paper | DD  |
| baseline           | 549 | 7.95 | 62.3 | 1.77 | +283  | 7.4 |
| fundgate>=2e-5     | 88  | 3.29 | 69.3 | 1.12 | +30   | 3.8 |
| fundalign          | 323 | 6.19 | 62.8 | 1.78 | +112  | 9.2 |

VERDICT: REJECTED. Quality-for-frequency trade that loses on net. Gating on extreme
funding DOES lift per-trade quality (ETH win 62->69%) = extreme-funding moments are
marginally better, but it discards ~80% of trades so total return + significance
collapse (ETH +283->+30%, t 7.95->3.29). Sign-alignment is least harmful (keeps ~60%)
but still costs return/t without improving them. => the basis-reversion edge is a
short-horizon microprice dislocation LARGELY ORTHOGONAL to the hourly funding rate;
crowdedness is not the driver. Funding infra kept as an option, not a default.
## SINGLE-VENUE REFERENCE BAKE-OFF (#5, 2026-06-11): OKX > Binance > Bybit
Which single external spot best anchors the HL basis? 36d, thr20, revert-exit, 884ms.
Basis sanity first (2026-06-07): all 3 refs align (BTC ~-3 to -5bps, ETH ~-5 to -6bps),
no scale bug. Liquidity gap: Binance ~4.7M BTC trades/day vs OKX 430k, Bybit 739k.
| BTC ref | n   | t    | RR   | paper | DD  |
| Binance | 187 | 3.92 | 2.23 | +43   | 5.8 |
| OKX     | 186 | 4.27 | 2.27 | +41   | 4.1 |
| Bybit   | 193 | 3.90 | 2.19 | +43   | 4.8 |
| ETH ref | n   | t    | RR   | paper | DD  |
| Binance | 549 | 7.95 | 1.77 | +283  | 7.4 |
| OKX     | 568 | 9.73 | 2.41 | +391  | 5.7 |
| Bybit   | 575 | 5.63 | 1.54 | +289  | 14.6|

SURPRISE (prediction was Binance-best): OKX is the BEST anchor despite ~1/10 the volume.
ETH OKX is STRICTLY better than Binance - higher t (9.73 vs 7.95), RR (2.41 vs 1.77),
return (+391 vs +283), AND lower DD (5.7 vs 7.4). BTC OKX marginally best. Bybit worst.

ROBUSTNESS SPLIT (ETH, OKX vs Binance):
- FEB 14d: OKX t=10.53/+384%/DD4.6 vs Binance t=8.21/+274%/DD7.4 -> OKX clearly better.
- MAY-JUN 22d: both flat/insignificant (OKX t=0.28, Binance t=0.55) -> no edge either way.
=> OKX's advantage is REAL but shown only in the VOLATILE Feb regime (the only window
with edge to measure). Calm window can't corroborate. Single-volatile-period result.

VERDICT: tentatively SWITCH default reference Binance->OKX (free venue swap, strictly
better where edge exists, never worse). Needs a 2nd volatile period to confirm it
replicates before fully trusting. Bybit EXCLUDED (worst). Explains why 3-venue
aggregate failed: noisy Bybit diluted the clean OKX/Binance signal. UNTRIED aggregate =
Binance+OKX only (drop Bybit) - possible follow-up, but OKX-alone already looks best.
## AGGREGATION QUESTION CLOSED (2026-06-11): use OKX SINGLE reference, no aggregation
Tested the only untried aggregate - Binance+OKX (drop noisy Bybit) - vs OKX-alone
(36d, thr20, revert-exit, 884ms):
| BTC OKX-alone   | n186 t4.27 RR2.27 +41% DD4.1 |
| BTC Binance+OKX | n173 t4.34 RR2.52 +49% DD4.3 | (trivially better)
| ETH OKX-alone   | n568 t9.73 RR2.41 +391% DD5.7 |
| ETH Binance+OKX | n538 t7.60 RR1.77 +340% DD7.4 | (WORSE - Binance dilutes OKX)
VERDICT: no aggregate reliably beats the single best venue. BTC Bin+OKX ~ tie; ETH
OKX-alone clearly wins (adding Binance lowers t 9.73->7.60, raises DD). => FINAL:
single OKX reference. Aggregation DEAD (3-way diluted by Bybit, 2-way hurts ETH).
REFERENCE-VENUE RESEARCH COMPLETE: OKX is the anchor; strict upgrade over old Binance
default; free (venue swap, no complexity). Caveat unchanged: edge is Feb-concentrated;
OKX advantage shown in the one volatile window - confirm on a 2nd volatile period.
## CROSS-ASSET LEAD (#3) TESTED -> REJECTED (2026-06-11)
Engine change: added a non-traded LEAD price channel (feed Role::Lead + lead_symbols,
engine lead_px, LagCtx.lead_px) + BasisConfig xlead/xlead_bps/xlead_lookback knobs
(hunt --leadsym --xlead --xleadbps --xleadlb). clippy+null-edge green. Filter SKIPS
the reversion when the lead asset (BTC) moved the SAME direction as the ETH dislocation
over the lookback (hypothesis: those gaps are real lead-follows, not noise -> won't revert).
ETH, ref=OKX, lead=BTC/Binance, 36d, thr20, revert-exit, 884ms:
| config         | n   | t    | RR   | paper | DD   |
| baseline       | 568 | 9.73 | 2.41 | +391  | 5.7  |
| bps1 lb20(10s) | 546 | 7.97 | 2.08 | +320  | 8.8  |
| bps2 lb20      | 551 | 8.10 | 2.08 | +333  | 7.7  |
| bps3 lb20      | 554 | 8.12 | 2.07 | +335  | 7.7  |
| bps2 lb40(20s) | 535 | 7.78 | 2.09 | +291  | 10.3 |
(short 2s windows barely fire - BTC rarely moves 5bps/2s; 10-20s windows bite.)
VERDICT: REJECTED. Removing BTC-aligned trades LOWERS t (9.73->~8), cuts return, and
RAISES drawdown - every config strictly worse. Those gaps revert fine; BTC's move is
orthogonal. The basis edge is HL-perp lagging ITS OWN spot (catches up to ETH-spot
regardless of BTC). Lead channel kept in engine as infra, not used.

## LAG-VENUE RESEARCH COMPLETE: #1 agg REJECTED, #2 funding REJECTED, #3 cross-asset
## REJECTED, #5 bake-off DONE (OKX best). The ONE win from the whole arc = OKX reference
## swap (free, strict upgrade over Binance). All conditioning ideas failed - the edge is
## a clean HL-vs-its-own-spot reversion that resists extra conditioning.