---
tags: [strategy, lagshot, basis, spec]
type: strategy-spec
---
# LAGSHOT - strategy spec

Cross-venue basis/lag reversion. The name: LAG (the edge - Hyperliquid lags spot
price discovery) + SHOT (the slingshot - the basis stretches past normal, then
snaps back). Runs on the forgelag engine (branch `forgelag`). Engine core SACRED /
null-edge gated. This doc = the canonical spec; results live in [[forgelag-hunt]].

## One-liner
Fade the gap between Hyperliquid's perp microprice and OKX spot when it stretches
too far from its recent baseline; hold minutes; close when it reverts. Taker-only.

## Spec sheet
| field            | value |
|------------------|-------|
| Type             | cross-venue basis / lag reversion (perp vs spot) |
| Execution venue  | Hyperliquid perp, TAKER (must cross spread; maker fails = adverse selection) |
| Reference anchor | OKX spot (beat Binance/Bybit/Kraken; aggregation rejected) |
| Assets           | ETH (primary engine), BTC (diversifier). SOL/HYPE excluded. |
| Signal           | HL microprice dislocation vs OKX, measured vs a rolling baseline (window 500, 0.5s samples) |
| Entry            | fade when |dev| >= threshold. ETH thr16 (max profit) / thr19 (best risk-adj); BTC thr15-16 |
| Exit             | REVERT-TO-MEAN (close when |dev| <= ~2bps); 10m hold timeout; 30s cooldown |
| Hold horizon     | minutes (NOT sub-second) - this is why it survives latency |
| Latency model    | 884ms real HL order-to-fill (the binding constraint; <~500ms = much stronger, >1s = inverts) |
| Account model    | EUR500, 20x leverage, 20% sizing, 5% daily-loss halt |
| Rejected knobs   | maker entries, stop-loss, aggregated ref, funding-conditioning, cross-asset lead, z-score, velocity gate, magnitude sizing, book-confirm(on OKX), regime filter(redundant) |

## Performance (backtest, two INDEPENDENT periods, 884ms)
| period                 | ETH thr16            | BTC thr15-16        |
|------------------------|----------------------|---------------------|
| Train (Feb+MayJun '26) | t9.35 +521% DD10 win55| t5 +67-69% DD5.5    |
| OOS (Nov-Dec '25, 61d) | t13.31 +542% DD4.2 win62.5 | t3.3 +35-37% DD~4 |
Frequency: ETH ~16-28 trades/day, BTC ~5-13.
Best risk-adjusted single: ETH thr19 (t10.13, +453%, DD5.0).
Portfolio BTC+ETH OKX (20% each, thr20): EUR500->3468 (+594%, DD7.6).

## Validation passed (the anti-lie checks the last project never had)
- NULL-EDGE gate: seeded coinflip nets NEGATIVE (engine doesn't manufacture edge).
- SHUFFLE control: randomizing direction goes hard negative (t -6 to -13) in BOTH periods.
- OUT-OF-SAMPLE: replicates on data never tuned on (Nov-Dec 2025), in fact stronger + lower DD.
- No-lookahead by construction (two-clock model); deterministic; reproducible to the decimal.
- No ruin in any tested config.

## The ONE open gate (everything else is validated)
REAL EXECUTION LATENCY. All numbers assume 884ms and idealized fills. Not a single live
order placed. The strategy is TAKER and latency-sensitive -> the only honest next step is a
tiny funded HL order to measure true signal->fill latency. That converts Lagshot from
"strong backtest" to "tradeable or not". Until then: a validated hypothesis, NOT money.

## Honest caveats kept attached
- Idealized fills (sim models queue+slippage, reality has more friction).
- Edge is regime-dependent (concentrates in volatile stretches; calm windows ~flat).
- Decay risk (a clean edge invites competition).
## REAL HL EXECUTION LATENCY MEASURED (2026-06-11) - the gate
Setup: AWS Tokyo box (ap-northeast-1), live HL account ($13 test), official python SDK,
20 tiny ETH marketable IOC orders alternating buy/sell (ended flat, residual 0). TCP to
api.hyperliquid.xyz = 2-4ms (confirms Tokyo-adjacent placement).
SIGNAL->FILL latency (ms): min 690 / p25 722 / MEDIAN 766 / p75 874 / p90 1101 / max 1107 / mean 806.
=> Real median 766ms is UNDER the 884ms Lagshot was validated at (ETH t9-13). LATENCY GATE
GREEN - the binding constraint came back favorable. This is BASELINE (no priority fees, no
own node; both are downside-latency levers we haven't used).
CAVEATS: n=20, single time window + market load (need a bigger sample across hours/regimes);
p90 tail ~1.1s where the edge weakens; this measured LATENCY only - NOT fill-price/slippage
(orders filled ok but expected-vs-actual px not yet compared). NEXT: (1) larger latency
sample over a longer window; (2) slippage check (real fill px vs mid at send); (3) live paper
run of the actual Lagshot decision loop (detect dislocation -> taker -> revert exit) tiny size.
## PRIORITY-FEE worth-it analysis + LATENCY SENSITIVITY (2026-06-11)
ETH thr16 latency sweep (OKX, revert-exit, 36d) around real op point:
| latency | per-trade | t    | win  | paper% | DD%  |
| 640ms   | 7.6bps    | 15.9 | 70%  | +2116  | 5.8  |
| 700ms   | 6.8bps    | 14.3 | 66%  | +1521  | 5.9  |
| 766ms*  | 6.0bps    | 12.4 | 62%  | +1056  | 5.9  | (*our REAL measured median)
| 810ms   | 5.5bps    | 11.3 | 60%  | +823   | 6.0  |
| 884ms   | 4.5bps    | 9.4  | 55%  | +521   | 10.0 | (what we'd been quoting)
| 950ms   | 3.8bps    | 8.0  | 53%  | +368   | 12.1 |
PRIORITY FEE = NOT WORTH IT: costs 1bp per 45ms; 45ms only recovers ~0.5bp of per-trade
edge. Pay 1 to gain 0.5 = net loser on the median. Skip it.
BIG NOTE: at real 766ms median the edge is ~2x the 884ms number we'd been quoting. BUT the
curve is VIOLENTLY steep (640->950ms = +2116%->+368%) and real latency is a DISTRIBUTION
(median 766, p90 1101, tail to 1.1s). So realized return is a BLEND dragged down by slow
fills - NOT the clean 766 number. Latency CONSISTENCY (killing the tail) matters as much as
median -> argues for own-node (steadier) eventually, NOT priority fees. NEXT: feed the
measured latency DISTRIBUTION into the sim (sample per-order) for the honest blended number.
## BLENDED REAL-LATENCY-DISTRIBUTION result (2026-06-11) - the honest number
Wired the 20 live latency samples (690-1107ms) into the engine: each order samples its
submit->execute delay from the empirical distribution (engine set_latency_samples; hunt
--latdist real). clippy+null-edge green. 36d, OKX, revert-exit:
| config    | per-trade | t    | win | DD%  | EUR500-> |
| ETH thr16 | 5.5bps    | 11.5 | 60% | 5.9  | 4723 (+845%) |
| ETH thr19 | 7.6bps    | 11.8 | 66% | 4.3  | 3672 (+634%) |
| BTC thr16 | 4.2bps    | 6.4  | 57% | 5.0  | 937 (+88%)   |
| BTC thr15 | 3.5bps    | 5.9  | 55% | 5.9  | 936 (+87%)   |
Lands BETWEEN the 766ms point (+1056%) and 884ms (+521%) - blend ~= mean-latency (806ms)
perf with the tail dragging it. THIS is the number to quote (latency reality baked in, not
assumed). ETH thr16 ~EUR4.7k / thr19 ~EUR3.7k (lower DD 4.3). BTC ~2x its old 884ms number.
CAVEATS (real): (1) latency dist is n=20, ONE CALM window - latency worsens in volatility =
exactly when signals fire; this model samples latency INDEPENDENT of market state, so if
latency is CORRELATED with vol (slower when we trade), true number is LOWER. Next: measure
latency during a VOLATILE stretch. (2) fills still idealized - slippage (real fill px vs
expected) NOT yet checked. NEXT: vol-period latency sample + slippage check + live paper loop.