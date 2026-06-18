# ForgeOS - living state + who I work with (always-on memory)

Auto-loaded every session. Keep it current. Full narrative: docs/PROJECT-CONTEXT.md.

## Who the trader is (talk to him this way)
- 10y DISCRETIONARY microstructure trader. NOT a coder. Explain in simple TRADING
  terms; avoid jargon ("knob" = setting/dial). He understands trading logic deeply
  - never condescend.
- Burned by his first project (a TS engine that LIED: fake 100% win from lookahead).
  So: be the HONEST SKEPTIC. Correct him when wrong. Never flatter a result.
- Real account ~ EUR 500. Deploys on Hyperliquid.
- Works STEP BY STEP; gets uneasy if overwhelmed or if things get messy. He keeps
  adding ideas - CAPTURE them in docs so nothing is lost.

## What ForgeOS is
Clean-room Rust backtest engine for crypto microstructure edge discovery. The #1
rule: a seeded COINFLIP must LOSE ~fees (null-edge gate). If it ever profits, stop
and fix - the engine is lying.

## Engine = SACRED (do not touch without explicit sign-off)
engine.rs, account.rs, fills.rs, forge-book, forge-core, forge-data. New work goes
in the strategy/shell/sweep/tooling layers ABOVE it. Verified honest: det-hash
reproducible, coinflip loses on real data.

## Current state (update as we go)
- Public indicators (OFI / wall / CVD), slow+fast + regime-gate: ALL no-edge
  (~12k configs). Confirms: crowded signals are arbitraged out.
- Built: regime classifier + attribution + opt-in gate; results ledger.
- wall-flow (real-vs-fake wall): SWEPT slow+fast -> 0 promote (no edge, but valid).
- absorption: validated (clippy+test green) + SWEPT (in progress / read result).
- BASIS REVERSION = RE-OPENED (2026-06-11). My morning "KILLED" was a TIMESTAMP BUG
  (divided Binance trade ts by 1e6 though it was already ms -> Binance leg frozen to
  one price; accidentally tested HL-vs-its-own-mean). Corrected single day
  (2026-02-01): thr>8bps, ~3min hold = ~39 trades/day, gross +15bps, NET +4.2bps
  after 11bps, 92% win; momentum side a clean mirror. THIN (11-14 trades, idealized
  fills, funding ignored). Multi-day out-of-sample validation running.
  docs/research/lead-lag-study.md.
- Tooling DONE: Obsidian vault (MOC + styled graph) + NotebookLM pack + this
  always-on memory + tools/sync-vault.ps1 (repo->vault). Vault at
  C:\Users\User\Desktop\obsidian\forgeos.
- DATA = cryptohftdata (CHD, already in use, FREE no limits). Covers full spec:
  9+ venues incl Hyperliquid, FULL-DEPTH tick L2, trades, funding, LIQUIDATIONS;
  REST/bulk. Paid feeds (Tardis/Kiyotaka/CCT) NOT needed. Gap was OUR converter
  (only pulled Binance bookDelta+trade + thin HL hlquote). FIX: extend
  chd-to-parquet.py to pull HL FULL L2 (unblocks basis-reversion) + multi-venue L2
  + liquidations/funding. Do not re-shop providers.
- DATA QUALITY FIX (2026-06-18): 4 dates (Mar 1, Apr 1-3) excluded due to missing
  HL funding/OI data. Stitched CSV has 40K bars from 18 dates; clean CSV has 34K
  bars from 14 dates. Date-aware CV splits folds on date boundaries (not bar index).
  sweepscope v10: 16 PROMOTE, 4 PARK, 739 RETIRE (vs v9: 23 PROMOTE, 4 PARK, 732 RETIRE).
  OI collapse, CVD momentum, ask skew all promote on clean data.
- LAG-SUBSPACE engine clone = CANCELLED (no pulse to justify touching the engine).
- NEXT: pick a fresh lead from the UNTESTED high-value buckets - Type C forced-flow
  (LIQUIDATION cascades, via CHD liquidations) or Type B chart-trigger + orderflow-
  CONFIRM (the labels/ on-ramp). No engine touch needed for the studies.

## Standing decisions / rules
- Metrics in PERCENT, not bps (in reports). Leverage 20x, size 20%, EUR500 paper
  gate (profitable + not ruined; drawdown reported, not gated).
- Verdict gates on net/expectancy/DSR/PBO + KNOB-BITE (a "no edge" only counts if
  the dial changed trades). Low win rate is fine if expectancy is positive.
- Edge lives in: CONTEXT (conditioning) / HARD-TO-COMPUTE / FORCED-FLOW - not plain
  public indicators. Trader method 1 = chart pattern as TRIGGER, orderflow as
  real/fake CONFIRM. No-lookahead: trigger uses only data <= now.
- Compute: coarse-then-fine sweeps when a candidate shows a pulse; rent a big box
  BY THE HOUR for heavy hunts; current box for daily.
- NEVER trust a python pre-study number; only a fully-gated + paper result counts.

## Workflow norms (see .kiro/steering/environment.md for box/shell gotchas)
- Build/test on the Hetzner box in tmux; commit small + push + verify each step.
- File writes via execute_pwsh WriteAllText (no-BOM); heavy/box work detached in
  tmux (never block on ssh - it orphans).
- BASIS update: survives spread (HL spread ~0.13bps); REAL fills+9bps fee = NET
  +8.55bps/trade, ~9/day, positive all 6 days. Only unmodelled risks left = LATENCY/
  adverse-selection + impact = the ENGINE's job. NEXT = lag-subspace engine build
  (needs SIGN-OFF). Type C cascade study coded + bug-fixed, ready to run in parallel.

- SUBSPACE BUILD (branch lag-subspace, signed off): data contract documented
  (docs/research/subspace-design.md). Step 1 DONE+VERIFIED: converter hlbook stage
  (HL full-L2 -> correct delta stream) reconstructs IDENTICAL top-5 microprice
  (max|diff|=0.0 over 6539 snaps). NEXT: forge-data convert.rs hl_book stream ->
  *.forge; then BasisReversion strategy; then gates (null-edge/knob-bite/DSR/PBO/paper).

- SUBSPACE COMPLETE (steps 1-6, branch lag-subspace): hlbook stream (verified
  exact) + BasisReversion strategy + latency-ladder sweep + paper gate; build/clippy/
  test green; engine core UNTOUCHED. ENGINE-GRADE VERDICT: basis edge is REAL
  (shuffle control loses), degrades smoothly with latency (9bps@0ms -> 3bps@700ms),
  lives in SIDEWAYS regime, PAPER GATE PASSES (EUR500/20x +5.7-7.8% @300ms). BUT
  DSR~=0 -> 0 promote (too thin on 6 days). NOT trusted live. NEXT: many more days
  for DSR/PBO power + sideways-only regime gate. (Fixed a fill_timeout<latency
  order-flood artifact mid-run.)

- 13-DAY HUNT done: PBO 0.002 (OOS-stable), edge real @0ms (t~2.0-2.25) but DROPS
  below significance @300ms latency (t~0.9-1.4). Paper +5.8-6.6%@300ms not trusted
  (t~1). DSR gate mis-calibrated for sparse strategy (bucket-Sharpe~0.017) -> use
  per-trade t-stat. VERDICT: promising+OOS-stable but UNPROVEN at realistic latency.
  NEXT: more days (sqrt(N): ~26+ days to reach t~2 @300ms if edge holds) - CHD has
  more HL days to pull. Viability hinges on execution latency (<150ms = t~2).

- FORGELAG ENGINE (branch forgelag, dedicated basis engine, null-edge passes):
  22-CONSECUTIVE-DAY hunt on FRESH data = edge SIGNIFICANT at realistic latency.
  per-trade t-stat: 300ms->t=4.95 (+5.7bps,742 trades), 500ms->3.31, 700ms->1.35,
  1s->-2.44 (inverts). Shuffle control t=-6.70 (negative). LATENCY CLIFF: tradeable
  only <~500ms exec to HL. Best lead by far. NEXT gates: robustness across OTHER
  months (these 22d = one regime), multi-venue/aggregated ref, drawdown/ruin check,
  real exec-latency measurement. docs/research/forgelag-hunt.md.

- OOS CONFIRMED: basis edge HOLDS on Feb (14d, independent of May-Jun): 300ms
  t=4.77 vs May-Jun t=4.95. Two independent periods significant + shuffle negative
  + no ruin. Validated across regimes - strongest result yet. NEXT: expand
  (venues/variances/dead-indicator confirm) + measure REAL HL exec latency + live paper.

- MAKER entries TESTED+REJECTED: limit entries lose hard (t=-8..-11, win ~10%) =
  adverse selection (resting fills only on continuation, misses the bounce). Edge is
  TAKER-ONLY -> latency is binding. forgelag now models maker fills (queue) + HL
  trades feed; null-edge passes. NEXT real unknown = measure true taker signal->fill
  latency on HL (tiny funded order); HL structural ~230ms floor puts us on the cliff.

- REAL HL LATENCY known (researched): ~884ms Tokyo / ~1079ms VA / ~900-1100ms our
  Germany box (server-side dominated; colocated-only 200ms). Small/frequent edge DEAD
  at real latency. BUT big-dislocation variant SURVIVES: thr>=20bps/3min/taker @884-
  1000ms = t=2.4-3.0, +6-8bps, ~5-6 trades/day, RR~1.9, win~48%, paper +59-80%/36d,
  DD~10%. Deployable SLIVER (borderline). NEXT: more periods to firm up + live validate.

- VARIANCE TESTS: velocity gate=no effect (rejected); Z-SCORE k=3=disaster (over-
  trades noise, rejected); REVERT-TO-MEAN EXIT = WINNER. Best deployable now =
  big-dislocation (>=15bps) + revert-exit: @884ms real latency t~4, ~12 trades/day,
  RR 2.0-2.2, +43-57% paper/36d, DD ~6-7% (beats fixed-hold). forgelag has exit()
  hook + zscore + velgate flags. NEXT variances: funding/book condition, magnitude
  sizing, ref-leg swap. (entry now uses sampled-dev; minor implementation sensitivity.)

- BOOK-CONFIRM (#4) WORKS on selective variant: thr20+revert-exit+confirm(imb>=0.2)
  @884ms = t=4.81 (up from 3.92), win 58%, RR 2.3, DD ~5%, +43%/36d (~EUR715). Dead
  imbalance indicator validated AS A CONFIRM (method-1). Quality win not return boost.
  BEST CONFIG: big-dislocation(>=20bps)+revert-exit+book-confirm. NEXT: magnitude
  sizing (#6) to lift euro, ref-leg swap (#5).

- MULTI-ASSET: ETH FAR STRONGER than BTC for the basis edge. @884ms real latency,
  36d, revert-exit no-confirm: thr20 t=7.95 +283% (EUR500->1915) win62% DD7.4% ~15/day;
  thr15 +353% (~2265) DD12.8%. vs BTC thr20 +43%. ETH thinner on HL -> bigger gaps +
  more trades. BREADTH = the euro-scaling lever. SOL downloading (test next). Data now:
  BTC+ETH 36d full each; SOL pending. Best ETH config: thr20+revert-exit.

- SOL weak (t=2.16, DD28%, exclude). PORTFOLIO BTC+ETH (36d,884ms,revert-exit thr20):
  20% each = +448% (EUR500->2742) DD9.6%; 10% each = +135% (->1177) DD4.9%. ETH=engine,
  BTC=diversifier (DD only 7.4->9.6 combined). CAVEAT: sequential compounding, doesn't
  model concurrent open positions -> prudent=10% each. lag-hunt --dumptrips added.
  Verified ETH/SOL basis sane + prices align (not a scale bug).

- HYPE test (HL native token, 18d): WEAK. basis sane (-13bps) but only thr30+reversion
  positive (t=2.29,+25%,DD5.6%); small thr negative, momentum negative. Causes: thin Bybit
  ref (~10.9k vs BTC 480k trades) + HL likely LEADS its own token. RANKING: ETH>BTC>>SOL~HYPE.
  LAG RESEARCH saved (docs/research/lag-improvements.md): top improvement = AGGREGATED
  multi-venue ref (Binance+OKX+Bybit spot VWAP) - would de-noise HYPE + sharpen ETH/BTC;
  also funding-condition + cross-asset(BTC) lead. NEXT build: aggregated reference.

- AGGREGATED REF (improvement #1) = TESTED + REJECTED for BTC/ETH (2026-06-11). Built
  multi-venue ref in forgelag (mean of Binance+OKX+Bybit spot; --symbol CSV; clippy+
  null-edge green). 36d head-to-head thr20/884ms: ETH agg t=7.88/+346%/DD12.3 vs single
  t=7.95/+283%/DD7.4; BTC agg t=3.60 vs single 3.92. => NO edge gain (t flat-to-down),
  only more drawdown. Single Binance ref stays default; agg kept as option for thin-ref
  coins (HYPE). REPRODUCIBILITY: logged ETH +283% & BTC +43% reproduced EXACTLY on fresh
  data = engine consistent (not drifting). REGIME-CONCENTRATION (real risk): edge is in
  the FEB window - the 22d May-Jun window alone is weak (ETH thr20 +2.6% single/+11% agg,
  BTC negative). Returns are lumpy/regime-dependent, NOT always-on. NEXT lead = Type C
  forced-flow (liquidation cascades) per the untested high-value buckets.
- FUNDING-CONDITIONING (improvement #2) = TESTED + REJECTED for BTC/ETH (2026-06-11).
  Built full funding infra: converter funding stream (HL mark_price; its event_time is
  NANOSECONDS - handled), feed/engine funding field + LagCtx.funding, BasisConfig
  fund_gate/fund_min/fund_align (hunt --fundgate/--fundmin/--fundalign). clippy+null-edge
  green. BTC funding ASYMMETRIC: positive capped +1.25e-5, negative spikes -1.4e-4.
  36d/thr20/884ms: gating on extreme funding lifts per-trade quality (ETH win 62->69%)
  but cuts ~80% of trades -> ETH +283%->+30% t7.95->3.29; BTC +43->+3.5. fund-align
  least harmful (keeps 60%) but still costs return/t. => basis edge is ORTHOGONAL to
  hourly funding; crowdedness not the driver. Infra kept as option, not default.
  LAG-VENUE RESEARCH now: #1 aggregated ref REJECTED, #2 funding REJECTED. STILL OPEN:
  single-venue bake-off (Binance vs OKX vs Bybit anchor), aggregated-ref ON HYPE (thin-
  ref case), #3 cross-asset lead (BTC leads ETH). Best deployable stays: thr20+revert-
  exit, single Binance ref (ETH t~8/+283%, BTC t~3.9/+43%, edge concentrated in Feb).
- SINGLE-VENUE BAKE-OFF (#5) DONE (2026-06-11): OKX > Binance > Bybit as the HL basis
  anchor. 36d/thr20/884ms: ETH OKX t=9.73/+391%/DD5.7 STRICTLY beats Binance
  t=7.95/+283%/DD7.4 (higher t+RR+return, LOWER DD) despite ~1/10 volume; BTC OKX
  marginally best (t4.27 vs 3.92); Bybit worst (ETH DD14.6, EXCLUDE). Split: OKX's win
  is from the VOLATILE FEB regime (t10.53 vs 8.21); May-Jun both flat (no edge to
  compare). VERDICT: tentatively switch default ref Binance->OKX (free swap, strictly
  better where edge exists), needs a 2nd volatile period to confirm. Explains agg failure
  (noisy Bybit diluted). LAG-VENUE LIST now: #1 agg REJECTED, #2 funding REJECTED, #5
  bake-off DONE (OKX best). STILL OPEN: agg-ref ON HYPE (thin-ref case), #3 cross-asset
  lead (BTC leads ETH; needs small engine change). Best deployable: thr20+revert-exit,
  OKX ref, ETH primary.
- AGGREGATION CLOSED (2026-06-11): tested Binance+OKX (drop Bybit) vs OKX-alone. BTC
  Bin+OKX ~tie (t4.34 vs 4.27); ETH OKX-alone WINS (t9.73 vs Bin+OKX 7.60, lower DD) -
  adding Binance dilutes OKX. FINAL ANSWER on reference venue: use SINGLE OKX (strict
  upgrade over old Binance default, free). No aggregation. REFERENCE-VENUE RESEARCH
  COMPLETE. Lag-venue list: #1 agg REJECTED, #2 funding REJECTED, #5 ref-leg DONE (OKX).
  Remaining lag ideas: agg-ref ON HYPE (thin-ref), #3 cross-asset lead (needs engine
  change), confirm OKX on a 2nd volatile period. Best config: thr20+revert-exit+OKX ref,
  ETH primary (Feb-concentrated edge).
- CROSS-ASSET LEAD (#3) = TESTED + REJECTED (2026-06-11). Added non-traded LEAD channel
  (feed Role::Lead, engine lead_px, BasisConfig xlead/xlead_bps/xlead_lookback; hunt
  --leadsym/--xlead/--xleadbps/--xleadlb). clippy+null-edge green. Filter skips ETH
  reversion when BTC moved same dir as the gap. 36d/thr20/OKX-ref/884ms: every variant
  WORSE (t 9.73->~8, +391->291-335%, DD 5.7->7.7-10.3). Gaps revert regardless of BTC;
  edge is HL-vs-its-own-spot. Lead channel kept as infra, unused.
  *** LAG-VENUE RESEARCH COMPLETE: #1 agg, #2 funding, #3 cross-asset ALL REJECTED;
  #5 bake-off DONE. ONE win = OKX reference swap (free, strict upgrade). All conditioning
  failed - clean reversion resists extra knobs. ***
  NEXT (user req): re-validate the variance settings (revert-to-mean exit, book-confirm,
  fixed-hold vs revert) on OKX data (they were validated on Binance; OKX is now default).
- VARIANCE RE-TEST ON OKX done (2026-06-11): (1) REVERT-TO-MEAN EXIT confirmed winner
  (fixed-hold disaster t~0.1-0.3 DD12-23%); (2) BOOK-CONFIRM FLIPPED - helped on Binance,
  HURTS on OKX (ETH t9.73->8.17, BTC 4.27->3.95) - it was compensating for Binance noise;
  drop it with OKX; (3) thr15 = more euro/more DD. SETTLED BEST: thr20 + revert-exit +
  OKX ref, NO confirm (thr15 for more return). Lesson: settings don't auto-transfer
  across ref venues. ETH primary (t9.73 +391% DD5.7). Caveats unchanged (Feb-concentrated,
  idealized fills, 884ms assumed). NEXT options: confirm OKX/edge on a 2nd volatile period;
  re-check the rejected variances (vel/zscore/magsize) on OKX if wanted; or live-paper prep.
- OKX PORTFOLIO (BTC+ETH, 36d, 884ms, revert-exit) computed (paper replica verified
  exact vs hunt): thr20 20%-each EUR3468/+594%/DD7.6 (21 trades/day); thr20 10%-each
  (TRUSTED, prudent) EUR1323/+165%/DD3.8; thr15 20%-each EUR4461/+792%/DD12.3 (46/day);
  thr15 10%-each EUR1504/+201%/DD6.3. No ruin. Beats old Binance portfolio (2742/1177).
  CAVEAT: sequential compounding, no concurrent-position cap -> trust 10%-each. CHD FEED
  VENUES (probed): hyperliquid_futures[exec], okx_spot[best ref]+okx_futures, binance
  spot+futures, bybit+bybit_spot, bitmex. NOT avail: coinbase/kraken/deribit/kucoin/
  gateio/htx/mexc/bitget/dydx. 5 exchanges/8 feeds - covers the strategy fully.
- KRAKEN-USD REF TESTED -> WORSE (2026-06-11). User flagged coinbase/kraken/deribit; CHD
  has Kraken (not coinbase/deribit). Kraken-USD ref 36d thr20: ETH t-0.69/+24%/DD33, BTC
  t-1.20/-18%/DD29 (vs OKX ETH t9.73 BTC t4.27). Basis sane but Kraken ~18x THINNER -> stale
  ref -> fake dislocations, negative edge + huge DD. PATTERN: reference LIQUIDITY is what
  matters; OKX wins. REFERENCE-VENUE SEARCH DONE: OKX anchor, full stop. CHD venues (SDK):
  binance/bybit/okx/kraken/bitget (spot+fut), hyperliquid, bitmex, lighter, aster. NO
  coinbase/deribit. SETTLED STACK: HL exec + OKX-spot ref + thr20(or15) revert-exit, ETH
  primary. OKX portfolio: EUR3468 aggr / 1323 prudent (user runs aggressive).
- THRESHOLD CLIFF MAP done (thr5-25, OKX, revert-exit, 36d, 884ms). Frontier: lower thr =
  more profit + more DD; <12bps = latency cliff (negative, thr5 = -85% win14%). PER-ASSET
  OPTIMA: ETH max-profit thr16 (EUR3105/+521/DD10), min-DD thr23 (DD4.9), BEST thr19
  (t10.13/+453%/DD5.0). BTC max-profit thr15 (EUR845/+69), min-DD thr24 (DD3.0), BEST thr16
  (t5.19/+67/DD5.6). NEW DEFAULTS: ETH thr19, BTC thr16 (beat old thr20 on profit+t+DD).
  Robust band = ETH 16-19 / BTC 15-17 (plateau, not exact decimal = overfit risk).
  NEXT: re-run BTC+ETH portfolio at per-asset optima (ETH~17-19, BTC~16) - should beat
  EUR3468. Then 2nd-volatile-period confirm + real HL latency. Each asset has its own thr.
- SELECTED (MAX-PROFIT configs, 2026-06-11): ETH thr16 (+521%, EUR500->3105, DD10%) and
  BTC thr15 (+69%, EUR500->845, DD5.5%), OKX ref, revert-exit, 884ms. These are the CHOSEN
  profit configs. NEXT FOCUS = reduce the DRAWDOWN (esp ETH thr16's 10%) WITHOUT killing the
  edge. Levers to try: (1) hard STOP-LOSS (cap the tail - DD is loser-sequence driven; sl_bps
  exists in ManagedConfig, expose via hunt --sl); (2) asymmetric stop = bail fast if gap WIDENS
  (structural move, not noise) but let reverters run; (3) vol/regime gate (skip the worst
  volatile clusters); (4) shorter hold/tighter revert-exit. Keep ETH thr19 (DD5.0/+453) as the
  low-DD alternative benchmark to beat.
- DRAWDOWN WORK done (2026-06-11): stop-loss FAILED (mean-reversion needs to ride through);
  REGIME filter (skip fade into HL trend) built+tested = WORKS but REDUNDANT with threshold
  (cuts DD ~20% on thr16/17 only by cutting return; on thr19 can worsen DD). DD is INTRINSIC;
  THRESHOLD is the dial. FRONTIER: max-profit thr16 +521%/DD10 (EUR3105); best risk-adj thr19
  +453%/DD5.0 (EUR2765,t10.13); min-DD thr19+regime(lb6/b12) +410%/DD4.6 (EUR2550). Regime
  infra kept optional. STOP knob-tuning = overfitting from here. NEXT = 2nd volatile period
  (OOS stability of edge AND drawdown) + real HL latency measurement. User keeps thr16 for profit.
- *** OOS VALIDATION Nov-Dec 2025 (61d independent) = EDGE REPLICATES (2026-06-11) ***
  Pulled 61d Nov-Dec 2025 (HL book+trades+funding+OKX ref, BTC+ETH; disk 29G/150G). OKX ref,
  revert-exit, 884ms: ETH thr16 OOS t13.31/+542%/DD4.2/win62.5 (vs train t9.35/+521/DD10) =
  STRONGER OOS, LOWER DD. ETH thr19 OOS t11.64/+343/DD4.5. SHUFFLE ctrl OOS t-13.38/-85% =
  no fake edge. BTC thr15 OOS t3.35/+37, thr16 t3.32/+35 (holds, weaker but significant).
  => TWO independent periods significant + shuffle negative both + no ruin = edge REAL +
  REGIME-ROBUST, NOT Feb artifact. 10% training DD was bad-luck case (OOS only 4.2%).
  FEB-CONCENTRATION DOUBT RESOLVED. LAST big gate = real HL execution latency (884ms assumed,
  idealized fills). Edge validated; next = measure live fill latency / tiny funded HL order.
- STRATEGY NAMED: *** LAGSHOT *** (LAG = HL lags spot + SHOT = slingshot stretch-then-snap).
  Spec doc = docs/research/lagshot-spec.md. = cross-venue basis reversion, HL-perp taker vs
  OKX-spot anchor, ETH primary (thr16 profit / thr19 risk-adj) + BTC diversifier, revert-to-
  mean exit, 884ms. Validated: null-edge + shuffle(neg x2) + OOS-replicates(Nov-Dec25, ETH
  t13.31/+542%/DD4.2). ONE open gate = real HL execution latency (live test). All else done.
- LATENCY RESEARCH done (2026-06-11, docs/research/latency-research.md). KEY: HL validators
  in Tokyo (AWS), 884ms round-trip is CONSENSUS/server-side dominated (net only 2-5ms).
  #1 MOVE = put box in AWS TOKYO (ap-northeast-1) -> ~884ms = the latency we ALREADY VALIDATED
  (vs Germany ~1080). Tokyo box alone = strong proven regime, no node needed for ETH.
  DON'T switch to a faster venue: the LAG IS THE EDGE; faster venue = less lag = less edge.
  SUB-884ms recipe (UPSIDE, not required): own non-validating node in Tokyo (32c/128GB/NVMe) +
  local order book from node outputs + split_client_blocks (70-150ms faster reads) + ORDER
  priority fee (~45ms faster PER 1bp, max 8bp, burned HYPE). Stacked -> plausibly ~400-600ms
  (where t jumps: 500ms t3.31, 300ms t4.95). PRIORITY-FEE ECON: 45ms/1bp; our net edge ~5-8bps
  so only 1-2bp worth it - MODEL it in forgelag (cost+Xbp, latency-45ms*X) before paying live.
  BREADTH (later): test Lagshot-edge on Aster/Lighter perp vs spot (CHD has the data) = more
  venues, not a switch. NEXT = Tokyo box -> tiny live order -> MEASURE real fill-latency dist.
- *** REAL HL LATENCY MEASURED (2026-06-11) - GATE GREEN ***. AWS Tokyo box (35.78.232.67,
  t3-class, ap-northeast-1), live HL account, 20 tiny ETH taker IOC round-trips alternating
  buy/sell (ended flat). TCP to HL = 2-4ms (confirmed Tokyo-adjacent). SIGNAL->FILL latency:
  min690 p25 722 MEDIAN 766 p75 874 p90 1101 max1107 mean806 (ms). => REAL median 766ms is
  UNDER the 884ms we VALIDATED Lagshot at (ETH t9-13). The make-or-break gate came back
  FAVORABLE. No priority fees, no own node = baseline (upside levers remain). CAVEATS: n=20
  single window (need bigger sample across hours/vol); p90 tail ~1.1s (edge weaker there);
  measured LATENCY not FILL-PRICE/slippage (next check). Setup: ~/lagshot/measure.py +
  venv + secret.env on box; spot->perp transfer needed (unified account had to be OFF);
  agent/main = 0xE4Cde743 (test wallet, $13). NEXT: larger latency sample + slippage check
  (compare real fill px vs expected) + then a live paper run of the actual Lagshot logic.
- BLENDED REAL-LATENCY result (2026-06-11): wired 20 live latency samples (690-1107ms) into
  engine (set_latency_samples; hunt --latdist real), sample per-order. 36d OKX revert-exit:
  ETH thr16 +845%/EUR4723/DD5.9/t11.5; ETH thr19 +634%/EUR3672/DD4.3/t11.8; BTC thr16 +88%/
  EUR937/DD5.0/t6.4. Lands between 766ms(+1056) and 884ms(+521) point estimates = honest
  number. PRIORITY FEE rejected (45ms costs 1bp, recovers only ~0.5bp). CAVEATS: latency dist
  n=20 ONE CALM window (latency worsens in vol = when signals fire; model samples latency
  INDEP of market state -> if correlated, true# LOWER); fills still idealized (slippage
  unchecked). NEXT: latency sample during VOLATILE stretch + slippage check + live paper loop.
  Box: AWS Tokyo 35.78.232.67 (key ~/.ssh/lagshot_tokyo); HL test acct 0xE4Cde743 ($13).
- *** LAGSHOT LIVE BOT DEPLOYED (2026-06-12) - tiny/real ***. tools/live_lagshot.py running
  in tmux `lagshot` on AWS Tokyo box (35.78.232.67). Real strategy loop: HL ETH l2Book
  (microprice top5) vs OKX ETH-USDT spot trades (ws), rolling baseline (win500/0.5s), enter
  on |dev|>=16bps, revert-exit |dev|<=2bps, 10m hold, 30s cooldown. SAFETY: 1x leverage
  (NO liquidation), $11 notional, single position, daily-15%-loss halt, realistic 5bps IOC
  cross (NOT the SDK 5% default - per trader: be realistic). Fees covered by thr16 (>9bps RT
  taker). DRY-RUN validated first: feeds connect, gap/dev match backtest (hl~okx~1672, gap
  -4 to -6bps = matches ETH basis, dev oscillates +-2bps calm). Logs every ENTRY/EXIT with
  net-edge, latency, fill px, slippage to /home/ubuntu/lagshot/live.log. MONITOR: grep
  ENTRY/EXIT from live.log over days; compare real per-trade bps (expect ~5.5-6 net) + win%
  + latency to backtest. This is the final integration test (latency+fills+logic in reality)
  at lunch-money size. Start equity $12.86. Probe earlier cost ~$0.14 in fees (expected).
- LIVE BOT FIX (2026-06-12 ~05:49): OKX public WS was dropping idle connections (~every few
  min, then stuck reconnecting) -> bot was ALIVE-BUT-BLIND for ~2.5h (03:17-05:47), stale-guard
  correctly refused to trade (no loss, calm market, stayed FLAT). ROOT CAUSE: OKX ws needs
  keepalive ping + sparse trades channel goes idle. FIX: replaced OKX ws with REST ticker
  poll every 0.4s (robust, no persistent conn to drop; failures skip via staleness guard) +
  added [wait] liveness log so 'blind' != 'dead'. Restarted --live: heartbeats steady, OKX
  fresh each beat, no errors. HL feed was rock-solid throughout (only OKX ws was flaky).
  LESSON: unattended bots need robust feeds + liveness logging. Still: 1x, $11, FLAT, $12.86.
- MORNING CHECK (2026-06-12 ~05:53): bot healthy post-fix (heartbeats steady, OKX REST fresh,
  FLAT, $12.86). ZERO strategy trades overnight - market too CALM: peak |dev| only 9.3bps vs
  16bps trigger (0 samples even hit 10bps; gap range -14.7..0). NOT a bug - legit quiet regime
  (backtest min was also 0/day). The 17 round trips in user_fills = YESTERDAY's latency probe
  (random orders, -0.6bps each = spread loss, NOT strategy); total acct impact -$0.21 all
  probe/fees. NO LIVE EDGE DATA YET - need a VOLATILE session to get real trades. Confirmed
  live: Lagshot is NOT always-on; idles in calm, fires in bursts on volatility (median 14/day
  but calm stretches = 0). PLAN: let it run several days incl a volatile stretch, then compare
  real per-trade bps to backtest (~5.5-6 net). Infra now robust. Box pennies/day; leave running.
- LATENCY RECONFIRMED (2026-06-12 morning, fresh 40-order probe, calm): median 741ms, mean
  787, p75 789, p90 891, max 1756 (one outlier). vs yesterday n=20 (median 766/mean806/p90
  1101/max1107). Combined ~60 orders: typical ~740-770ms median, CONSISTENTLY under validated
  884ms = strong regime holds. FAT TAIL real: ~1/40 fill at 1.76s (dud, edge gone there) -
  already baked into blended backtest (+845%). STILL calm-market latency; volatile-session
  latency (when bot fires) = the remaining unknown, needs bot's own trades. Bot restarted
  healthy (N reset usage: probe used N=40/NOTIONAL=11; bot back live). Acct still ~$12.86 FLAT.
- LIVE BOT FIX #2 (2026-06-12 ~09:53): HL feed ALSO died - the SDK l2Book WS subscription
  silently stopped ~09:00 (hl_age grew to 2700s). Our [wait] liveness log CAUGHT it (vs
  looking dead). Bot correctly REFUSED to trade on stale HL (no bad trades, calm anyway).
  FIX: converted HL feed from SDK ws subscription -> REST poll info.l2_snapshot(COIN) every
  0.3s (skip_ws=True). NOW BOTH feeds (HL + OKX) are robust REST polls, ZERO persistent ws.
  PATTERN/LESSON: SDK websockets unreliable for unattended runs; REST polling is the robust
  choice. Restarted, both feeds fresh, hb flowing, process alive. Equity $12.64 (= morning
  probe cost; 0 strategy trades still - market calm, dev never near 16). STOP running probes
  (burns acct). Just let it run for a volatile session.
- *** LIVE BOT BUG FOUND + FIXED (2026-06-12 ~12:45) - trader skepticism caught it ***.
  Forced a live machinery test (lowered thr to 1.5 to fire in calm market). Found exits were
  FAILING 100%: error "Order has invalid price." ROOT CAUSE = SDK arg mistake: market_close
  3rd positional arg is `px` not slippage; my call ex.market_close(COIN, None, 0.0012) set a
  $0.0012 limit price -> invalid. (Entries worked: market_open's 5th arg IS slippage.) Old
  code silently went FLAT on failed close -> position desync (unmanaged open position).
  FIX #1: rewrote bot to EXCHANGE-TRUTH state machine - pos_thread polls real position every
  2s; loop acts on REAL pos not internal belief; failed close just retries next loop (no
  desync possible) + pending guard. FIX #2: ex.market_close(COIN, slippage=EXIT_CROSS_BPS/1e4)
  (keyword). VERIFIED at thr1.5: clean round trips, ENTRY+EXIT both ok=True/filled, ends FLAT.
  Latency live 748-1221ms (matches measured), slip 0.2-1.8bps. Restored production thr16/exit2,
  1x, flat, eq $12.56 (test trades cost ~$0.10 spread, expected - no edge at thr1.5).
  LESSON: this is the bug that would have broken EVERY live exit. Test the machinery, don't
  assume. Bot now robust: both feeds + position all REST-polled, self-correcting.
- *** FIRST LIVE TRIGGERS (2026-06-12 NY ~13:30-13:40) - 2 key findings ***. Production
  thr16 fired 2 real ENTRY SELL signals on genuine dislocations (dev=16.4, 17.1bps) - DETECTION
  WORKS LIVE. BUT both fill=None ok=False "Order could not immediately match": (1) LATENCY IN
  VOLATILITY = 1314ms & 1827ms - MUCH worse than the calm-measured 766ms median. CONFIRMS the
  latency-vol correlation worry: latency blows out exactly when we trade. (2) the 5bps IOC
  cross was TOO TIGHT to fill - we SELL (HL rich, reverting), and in the 1.3-1.8s latency HL
  already dropped past our limit -> IOC sell can't match -> NO FILL. Latency-selected by our
  own prediction. FIX: widened CROSS/EXIT_CROSS to 50bps TOLERANCE (guarantees taker fill like
  backtest market-taker; on deep ETH book the tiny order fills at touch, tolerance just stops
  rejection; REAL slippage is logged). Redeployed. NEXT: catch a filled NY trigger + read the
  REAL slippage/per-trade edge - THE moment-of-truth (does edge survive ~1.5s vol-latency +
  reverted-price fills?). Honest: 1.3-1.8s live latency is in the backtest's danger zone
  (700ms t1.35, 1s negative) - the realized edge may be thin/negative. Measuring it now.
- *** DECISIVE LIVE RESULT (2026-06-12 NY) - EDGE NOT CAPTURABLE AT OUR LATENCY ***.
  First 3 REAL FILLED trades (post fill-fix), thr16, 1x:
  T1 14:52 BUY trig dev=-23.1 lat=1609ms, dev-at-fill=-9.6, 1674.0->1680.1 = +6.1 WIN
  T2 15:35 BUY trig dev=-17.2 lat=2380ms, dev-at-fill=+6.5(OVERSHOT past 0), 1676.7->1675.8 = -0.9 LOSS
  T3 15:49 SELL trig dev=16.1 lat=946ms, dev-at-fill=+0.4(reverted), 1662.1->1662.6 = -0.5 LOSS
  Equity 11.787->11.789 = ~FLAT (one lucky win offset 2 losses+fees). SLIPPAGE 15-22bps/trade.
  ROOT (smoking gun = dev-at-fill): we trigger at 16-23bps but by fill (1-2.4s later) the gap
  has ALREADY reverted to ~0 (or overshot). The basis-reversion happens FASTER than we can
  fill at real latency. We pay 15-22bps chasing a gap that's gone -> net wash-to-loss.
  TRIGGER-MOMENT latency: 2 of 3 >1.6s (negative zone per backtest). CONFIRMS backtest:
  >1.3s = breakeven/negative. VERDICT: Lagshot edge is REAL (null-edge+shuffle+OOS all
  passed) but NOT CAPTURABLE at retail execution latency (~1-2.4s, worst at triggers) -
  the reversion outruns us. Break-even at best, structurally negative with more trades.
  This is the PROCESS WORKING: found the binding constraint live with $12 before risking
  EUR500 (unlike the prior project that lied). ONLY path to deploy = cut trigger latency
  <~800ms via own HL node + colocation + priority fees (big infra, still borderline).
  Total spent of $13: ~$1.2 (probes+tests+these trades). NEXT DECISION (user): invest in
  latency infra to chase it, or shelf Lagshot as 'real-but-latency-locked' and move on.
- *** FINAL LATENCY RESEARCH -> LAGSHOT CLOSED (2026-06-12) ***. Asked if we can FIX the
  execution latency to capture the edge. Researched HL tricks + general HFT. HINGE FACT:
  884ms Tokyo = ~5ms NETWORK + ~879ms SERVER-SIDE (HyperBFT consensus+matching = the CHAIN's
  floor, shared by all, NOT buyable). HL-only levers (official docs): own non-validating node +
  build book locally (kills ~100-200ms API read lag; needs 32c/128GB box), split_client_blocks
  (70-150ms), gossip read-priority (~25ms/slot, HYPE burned), order write-priority (~45ms/1bp
  max 8bp=360ms, HYPE burned). GENERAL HFT toolkit (colocation/kernel-bypass/FPGA/fiber) =
  USELESS: it shaves NETWORK+OS (micro/ns); our Tokyo network is already ~5ms; the wall is the
  879ms consensus we don't control. EVEN FULLY STACKED: best calm ~500-600ms, but (1) write-pri
  fee BURNED 45ms/1bp vs ~5-8bps edge = buying speed burns the edge (net-neg, confirmed); (2)
  TRIGGER latency was 1.3-2.4s live (blows out in the volatility when signals fire) - shaving
  300ms still leaves >1s and the reversion closes in <1s. VERDICT: reversion half-life ~= HL's
  consensus floor; retail taker is structurally below it; no spend justifiable on EUR500 crosses
  it. Latency = STRUCTURAL MOAT not a bug. LAGSHOT shelved: real edge (null-edge+shuffle x2+OOS
  x2 all passed) but NOT capturable by us. docs/research/latency-research.md. No further latency
  work. NEXT lead when ready = Type C forced-flow / liquidation cascades (CHD liq data; different
  edge that does NOT depend on out-racing a reversion). Bot can be stopped (tmux kill lagshot).
- *** LAGSHOT-MAKER PIVOT = DEAD (2026-06-12, spec .kiro/specs/lagshot-maker) ***. Built the
  full maker (resting-limit) variant to dodge the taker latency race: forgelag engine cancel/
  reprice path + honest maker fill model (queue+adverse-selection, no mid fills) + FairValueOracle
  (OKX-anchored) + InventoryController + QuoteManager + MakerQuoter + metrics + hunt --maker sweep
  + null-edge MAKER gate (coinflip maker LOSES synth -9.48 / real ETH -57.67 -> engine not lying).
  80 tests green, clippy clean, determinism byte-identical, no-lookahead proven. ALL on branch
  forgelag, sacred core untouched. COARSE SWEEP (10 real ETH days Nov-Dec25, OKX ref, grid
  quote_offset{0,1,2,4,8} x entry_thr{8,12,16,20}, REALISTIC fees maker+1.5/taker+4.5bps):
  * 800ms exec+cancel: EVERY cell NEGATIVE. * 0ms idealized upper bound + real fees: STILL every
  cell negative (~= 800ms). => pivot SUCCEEDED at dodging latency (0ms~=800ms) but loses to
  ADVERSE SELECTION + fees. Resting fills only on continuation (wrong side of reversion); clean
  reversions snap back WITHOUT trading through our quote = we never get the good fill. Tight
  offset bleeds (win 6-9%, t-22); wide offset ~3 trips/day + still < 6bps round-trip fee. Only
  green = default-REBATE control (maker -0.2bps) = REBATE MIRAGE (wall-bot lie pattern), negative
  under honest fees. NO net-positive knob-bite-valid cell at any latency. docs/research/lagshot-
  maker-hunt.md. VERDICT: both doors closed - TAKER Lagshot latency-locked, MAKER Lagshot adverse-
  selection-locked. Edge REAL but structurally uncapturable by us. Killed cheap in sim, 0 euros
  risked. NEXT lead must NOT depend on out-racing a reversion (taker) or being crossed-on-the-right-
  side (maker): -> Type C forced-flow / LIQUIDATION CASCADES (CHD liq data) is the queued lead.
- *** GAP-CLOSE ORDER-BOOK STUDY (2026-06-12, gapscope) - the "go back to the book" prize ***.
  Built forgelag/src/bin/gapscope.rs (analysis-only; reuses load_window + forge_book + oracle
  gap/dev; sacred core untouched; 88 tests green). Replays real ETH, detects dislocations
  (|dev|>=thr), measures HOW the gap closes over the close window. PATH-mode tweak first showed
  the close is TRADE-DRIVEN (rest-in-path fills ~19x fade) but the flow is MOMENTUM/overshoot not
  providable liquidity (run over harder: win 3%, t-65). gapscope then characterized the close,
  10 ETH days, thr 8/16/25 (knob-bite valid, monotonic): closed<5s 92-98%; MEDIAN TIME-TO-CLOSE
  1.1-2.0s (UNDER our 0.8-2.4s latency = why taker misses it); MECHANISM mostly RE-QUOTE 56-68%
  (price shifts with little/no trade) vs trade-driven 32-44%; WALLS: 95-98% of resolved walls are
  PULLED (cancelled) not ABSORBED (2-5%) = LIQUIDITY VACUUM not reload-to-absorb; close-dir volume
  2-3x against; OVERSHOOT (dev flips past 0) 41-68% (more for big gaps). MECHANISM = dislocation ->
  makers YANK quotes -> price re-quotes through the vacuum with a momentum shove -> overshoots ->
  settles. The trader's "wall reloads to absorb" hypothesis = REFUTED (it's pull/vacuum, opposite).
  VERDICT: book CONFIRMS the kill - can't provide into a re-quote, wall-pulls are concurrent (no
  predictive lead) + intent-ambiguous (defensive vs spoof, unprovable), momentum/overshoot is the
  flow that already ran us over. LAGSHOT FULLY EXHAUSTED: taker(latency)/maker-fade(adverse
  sel)/maker-path(run over)/raw book(re-quote vacuum) - all closed for measured reasons, 0 euros
  risked. docs/research/gap-close-study.md (+ per-dislocation CSVs on box /root/runs/gapscope/).
  NEXT LEAD = Type C forced-flow / LIQUIDATION CASCADES (CHD liq data) - a forced-flow edge that
  does not need to out-race a reversion or provide liquidity into a vacuum.
- *** NEW LEAD: OI-DROP FORCED-FLOW on HL (2026-06-12, liquidation-cascade pivot) ***. Original
  plan (CHD HL liquidations) = DEAD: CHD does NOT carry Hyperliquid liquidations (verified airtight
  - Oct-10 2025 mega-cascade shows 0 HL liq rows while Binance liq = thousands same day; ABSENCE not
  sparseness). CHD HL ids = hyperliquid_futures/_spot only, no liq feed. DO NOT re-shop HL liq on CHD.
  CHD liquidations DO exist for binance_futures etc (schema: received_time ns, event_time ms, side
  BUY/SELL=liq order side, price, quantity, ...). CHD FILE-PATH FORMAT (corrected, from SDK):
  <exchange>/<YYYY-MM-DD>/<HH>/<SYMBOL>_<type>.parquet.zst (HH is a DIR, file=SYMBOL_type; NOT
  <hh>_<type>). PIVOT CHOSEN (option 1): reconstruct the liquidation footprint from HL-NATIVE
  open_interest (CHD HAS hyperliquid_futures open_interest, dense ~83k rows/day; cols received_time,
  symbol, sum_open_interest, sum_open_interest_value, timestamp). THESIS: sharp OI DROP + price SPIKE
  + one-sided aggressive flow = forced deleveraging (cascade) -> big SLOWER-reverting overshoot we can
  REACT to (latency far less binding than the Lagshot micro-race - different edge shape, doesn't repeat
  the trap). CAVEAT: OI drop can be voluntary deleverage too -> require OI-drop + price-spike + one-
  sided flow TOGETHER as the high-confidence cascade signal. Rejected option 2 (CEX liq lead + trade HL
  leg) = risks repeating Lagshot latency trap. ETH first then BTC. PLAN: (A) converter open_interest
  stage + pull ETH/BTC (same 10 days) + feed.rs OI event wiring; (B) cascade study (oiscope) = detect
  cascades + characterize overshoot/reversion (time, size, tradeable at our latency?); (C) if pulse ->
  spec + build with honesty gates. gapscope is reusable for the post-cascade book behavior.
- *** OI-CASCADE STUDY (2026-06-12, oiscope) - NAIVE FADE = NO PULSE, latency NOT the
  killer this time ***. Built forgelag/src/bin/oiscope.rs (analysis-only; reuses
  load_window + forge_book + ctx.oi; sacred core untouched; build+clippy(-Dwarnings)+
  7 new tests green; full suite green). Detects HL cascades = OI-drop>=D% + |microprice
  move|>=P bps + one-sided HL flow over window W(5s), cooldown-dedup; characterizes
  spike/reversion + a SIMPLE reactive FADE entered at 0/800/2000ms (our latency band).
  10 days ETH+BTC, grid oi-drop{0.2,0.5,1.0}% x move{5,10,20}bps (KNOB-BITE valid:
  counts move monotonically). FINDINGS: cascades EXIST + frequent (ETH 6-33/day, BTC
  3-23/day); ~50-60% revert >=half the spike, ~40-50% TREND; reversion is SLOW (half
  ~15-23s, full ~22-35s). *** KEY: latency does NOT bind - fade@2s ~= fade@0ms (often
  better) - the "react do not race" thesis was RIGHT, genuinely unlike Lagshot. *** BUT
  the fade EDGE is negative-to-tiny: ETH NEGATIVE mean every cell (-1..-14bps, t<=0, fat
  LEFT tail = trending cascades -30..-70bps swamp small wins; ETH worse than BTC, opposite
  of Lagshot); BTC faint +2-3bps gross t~2 at mid-thr (move10) but TINY. FEE WALL kills
  it: taker RT ~9bps, best gross capture ~+3bps (BTC), ETH negative -> NET negative both.
  VERDICT: NO strategy spec - naive reactive fade is sub-fee + trend-tail-wrecked. Killed
  cheap, 0 euros. Latency-robustness is necessary-but-not-sufficient. Only cheap follow-up
  = trend-abort/stop to cut the left tail (Lagshot found stops hurt MR, skeptical) or a
  MAKER cascade fade (but gapscope showed post-event = re-quote vacuum/pulled walls ->
  adverse selection likely kills maker too). docs/research/oi-cascade-study.md; CSVs
  /root/runs/oiscope/{eth,btc}_cascades.csv. Type C forced-flow lead = effectively
  exhausted for the naive fade shape.
- TWEAK 1 exhaustion-conditioned entry (oiscope --exhaust/--exhaust-stall/--exhaust-decel, default OFF;
  build+clippy+97 tests green) TESTED ISOLATED 10d ETH+BTC = NOT tradeable alone. Fires on 100% of
  cascades / skips 0 over the 60s horizon -> it's an entry-TIMING shift (~4-5s later), NOT a trender-skip
  filter (n unchanged). Cuts ETH mid left-tail (<-30bps ~halved, mean +1-2bps) but worst trade ~-77bps
  survives (paused trenders). Best gross still BTC oi0.5/move10 +3.75bps t2.58 (n=30) << ~9bps taker fee;
  ETH neg-to-flat; net-of-9bps NEGATIVE every cell both assets. Knob-bite: STALL window is the live dial
  (wait 2.5->5.9s); DECEL nearly inert (price-stall leg binds). LESSON: "exhaustion" != selectivity (all
  cascades stall once); still cannot separate reverters from trenders. NEXT (isolated, one-by-one): tweak
  2 magnitude filter, then tweak 3 gapscope confirm. docs/research/oi-cascade-study.md Tweak 1 section.
- TWEAK 2 magnitude filter (oiscope --min-spike/--min-oidrop, default OFF; build+clippy+99 tests green,
  baseline byte-preserved) TESTED ISOLATED 10d ETH+BTC = NOT tradeable alone. NO-LOOKAHEAD gate =
  |realized window-start->fire move| (NOT the forward peak; unit-tested). Knob-bite valid (ETH 335->3->0;
  BTC 235->1; in-tool counts == dump). *** KEY ASYMMETRY: ETH bigger=WORSE (size=TREND; mean -1.6->-28,
  tail 4%->33% <-30bps) = dead any size; BTC bigger=BETTER quality, left tail stays ~0% (structural
  OPPOSITE of ETH). *** BTC min-spike15 (n=13-16, t~2.5-3.0) gross ~+5.7-6.2bps = BEST honest gross in
  the whole study, BUT net-of-9bps-taker ~-3bps (SUB-FEE); only thr>=20 clears fee (gross ~+11-12, net
  +2-3) at n=4-5/10d = too thin (tiny-n t artifact). VERDICT alone = NO. *** BINDING CONSTRAINT has
  shifted: it is now the ~9bps TAKER FEE, not the signal - BTC HAS a clean size-scaling no-tail reversion
  (~+6bps gross) we just can't take at 9bps. *** Implications: (1) BTC is the only asset with a pulse
  (ETH dead for the fade); (2) the lever that matters most may be FEE (maker entry) not more selectivity -
  but gapscope warned post-cascade HL = re-quote vacuum / pulled walls -> maker adverse selection risk.
  NEXT (isolated): tweak 3 gapscope-confirm (does liquidity come back=revert vs stay pulled=trend; also
  tells if a maker could rest post-cascade). docs/research/oi-cascade-study.md Tweak 2 section.
- TWEAK 3 order-book confirm + maker-fill feasibility (oiscope --ob-confirm/--ob-confirm-window/
  --ob-confirm-imb + --maker-fill/--maker-fee, all default OFF; build+clippy(-Dwarnings,all-targets)+
  101 tests green [99->101, oiscope 11->13], baseline byte-preserved) TESTED ISOLATED 10d ETH+BTC.
  PART A confirm gate = revert if top-5 depth IMBALANCE shifts back toward the HIT side within an
  800ms window (no-lookahead: book read at fire + window-end, entry DELAYED to window-end+delays so
  read<=entry; reuses gapscope imbalance). *** This is the FIRST REAL SELECTIVITY filter (skips
  ~50-64%, knob-bite MONOTONIC ETH 52->36% / BTC 63->37% across imb 0.00->0.20) - unlike tweak1
  exhaustion (fired 100%=timing only). *** It cuts ETH worst tail ~half (-78->-39bps) + lifts every
  cell +2-3bps (ETH dead->flat, best 0.5/5 +1.35@2s t~1); FIRMS BTC oi0.5/move10 to +5.80bps gross
  t3.03 @2s (BEST honest knob-bite-valid TAKER gross in the study, tail 0%). BUT net-of-9bps-taker
  STILL NEGATIVE at trustable n (ETH ~-8..-12, BTC ~-3.2); only n~1-4 cells clear the fee = too thin.
  TAKER-fade-with-confirm = still NO. PART B maker-fill (measurement only, on REVERTED cascades):
  a maker resting at the pre-cascade BASELINE FILLS in ~53-71% of reverted cascades (rest = re-quote
  vacuum) => post-cascade HL is NOT a total vacuum (softens gapscope worry); post-spike-level fill
  100% is MECHANICAL (ignore). Reversion gross +18-32bps, net maker(-1.5) +16-30bps - BUT conditioned
  on revert (LOOKAHEAD) + excludes the trender tail = FEASIBILITY UPPER BOUND, not tradeable. *** VERDICT
  isolated: neither part a green light; BINDING CONSTRAINT confirmed = the ~9bps TAKER FEE (BTC has a
  clean +5.8bps gross t3 reversion we still can't take as taker). The fee-beating path = MAKER entry
  that PRE-SELECTS reverters via the confirm gate -> the ONE combined test left worth running
  (ob-confirm gate + maker fill TOGETHER). *** docs/research/oi-cascade-study.md Tweak 3 section; logs
  /root/runs/oiscope_ob/{ETH,BTC}_{base,obconfirm,maker}.log + _kb_imb*.log.
- TWEAK 4 COMBINED ob-confirm + HONEST maker-in-path fade (oiscope --maker-fade/--maker-offset/
  --maker-armgate + reuse --ob-confirm; default OFF, baseline byte-preserved; build+clippy+oiscope
  tests 13->18 green) = NO-GO. The decisive test: maker entry fixes the fee (6bps RT vs 9bps taker)
  and FILLS ~95% (forced flow really trades through an in-path limit -> post-cascade is NOT a vacuum
  for in-path liquidity, contra the pure-vacuum worry). BUT removing the revert-conditioning collapses
  the Tweak-3 Part-B mirage: honest BTC gross WITH TRENDERS COUNTED is ~0 to +1.9bps at trustable n
  (not +18-32bps); the high fill rate is mostly trenders that fill then run over. NET-of-6 NEGATIVE at
  every trustable-n cell both assets (BTC ~-4..-6, ETH ~-7..-12); only n<=5 cells reach breakeven (too
  thin). ob-confirm hold-gate halves ETH tail (-79->-50) + lifts ~+2-3bps but ETH still dead; on BTC
  the tail was already ~0 so it barely moves. VERDICT: binding constraint flips back FEE->SIGNAL once
  measured honestly - same wall, other side. *** OI-CASCADE FADE FULLY EXHAUSTED *** (like Lagshot).
  Killed cheap, 0 euros. docs/research/oi-cascade-study.md Tweak 4 section; logs /root/runs/oiscope_t4/.
  KEY REUSABLE: ob-confirm (depth-imbalance returning to hit side) = a WORKING selectivity/CONFIRM
  primitive (knob-bite valid, cuts tails); "react don't race" VALIDATED latency-robust for printed
  forced moves; in-path maker fills ~95% post-cascade. Recurring kill pattern across ALL leads =
  latency OR adverse-selection OR the ~6-9bps FEE FLOOR vs small-bps mean-reversion. Implication for
  the NEXT edge: must be BIGGER per-trade (>>9bps), latency-robust (react to printed flow), and likely
  NOT symmetric mean-reversion (the momentum/continuation/overshoot tail kept running us over -> maybe
  trade WITH the forced continuation, not against it - UNTESTED).
- *** COMMUNICATION RULE (trader request 2026-06-12, ALWAYS follow) ***: report results
  STRAIGHTFORWARD + SHORT + simple terms. LEAD with a one-line clear VERDICT (good or bad,
  say which). When there is both good and bad news, state the BAD FIRST. No hype, no
  emotional rollercoaster, no burying the lede. Never omit anything important, but keep it
  tight. He gets lost when a result is ambiguous - always make positive-vs-negative explicit.
- TWEAK 5 MOMENTUM = trade WITH the cascade continuation (oiscope --momentum/--mom-target/--mom-stop/
  --mom-hold/--mom-trend, default OFF, baseline byte-preserved; build+clippy(-Dwarnings,all-targets)+
  oiscope tests 18->23 green) TESTED 10d ETH+BTC = NO, does not clear the 9bps taker fee at trustable n.
  TAKER entry IN the cascade dir (down->short, up->long), first-touch target/stop/hold, reverters that
  stop out COUNTED. *** SIGN FLIPS on ETH: momentum earns +1..+5bps GROSS where the fade lost - the
  overshoot/continuation IS the dominant flow on ETH (trader's read confirmed); trend-confirm gate
  (ob-confirm INVERTED: liquidity STAYS pulled = trend unopposed) is a real knob-bite-valid filter that
  lifts ETH gross/win/t (oi0.2/mv10 ungated +2.80 t2.40 -> trend0 +5.41 t2.98 n=85). BUT best trustable
  NET-of-9 ~-3.6bps (still sub-fee); only n<=12 cells go net-positive (tiny-n). BTC momentum flat-to-
  NEGATIVE everywhere (BTC reverts = wrong side for momentum), gate does not save it. 0ms entry best
  (continuation freshest at fire, decays by 2s = mirror of fade where 2s was best). *** SAME WALL as the
  fade, other side: ~9bps taker fee vs a few-bps per-trade edge. OI-cascade edge too small for a taker in
  BOTH directions at our fees. *** Killed cheap, 0 euros. docs/research/oi-cascade-study.md Tweak 5;
  logs /root/runs/oiscope_mom/{ETH,BTC}_{t15s8,t20s10,t25s12,t30s15,t40s20,trend0,trendL}.log.
- TWEAK 5 MOMENTUM (trade WITH the cascade continuation; oiscope --momentum/--mom-target/--mom-stop/
  --mom-hold/--mom-trend, default OFF; build+clippy+oiscope 18->23 tests green; baseline+T1-4 byte-
  preserved) = NO at trustable n. VERDICT bad-first: net of ~9bps taker every trustable-n cell NEGATIVE
  both assets -> OI-cascade edge DEAD in BOTH directions (fade AND momentum). GOOD: ETH sign FLIPS
  POSITIVE (+1..+5bps gross where fade lost) -> overshoot/continuation IS the dominant flow on ETH
  (confirms trader's read); trend-confirm (ob-confirm INVERTED: liquidity STAYS pulled) = real knob-bite
  filter, lifts ETH to best +5.41bps gross t2.98 n=85 - but NET-of-9 = -3.59 (sub-fee). BTC momentum
  flat-to-NEG everywhere (BTC reverts = wrong side; mirror of its faint fade-positive). 0ms entry best
  for momentum (continuation freshest at fire), mirror of fade (2s best); mild decay not a cliff.
  *** THE WALL, now nailed across ALL leads: ~9bps TAKER FEE vs a ~5bps microstructure edge - signals
  are REAL+honest but SMALLER than the toll. *** OI-cascade fully exhausted (taker, both directions).
  docs/research/oi-cascade-study.md Tweak 5; logs /root/runs/oiscope_mom/. IMPLICATION: microstructure-
  bps edges on HL TAKER are structurally dead. Only escapes = (a) much lower fees (real maker rebate /
  different venue/tier) or (b) a fundamentally BIGGER-per-trade edge (tens of bps, not 5), i.e. NOT
  tick-scale microstructure. Next lead must clear that bar by construction.

- *** SWEEP / STOP-RUN SWING STUDY (sweepscope, branch forgelag) = NO PULSE (bigger-size lead) ***.
  First BIGGER-per-trade lead (tens-to-hundreds of bps, leaving tick-scale microstructure dead at 9bps).
  Built forgelag/src/bin/sweepscope.rs (analysis-only; reuses load_window+forge_book+microprice+imbalance;
  sacred core untouched; clippy -Dwarnings clean; 16 new tests + full forgelag suite green). Mechanised the
  trader's setup no-lookahead: SIDEWAYS range (width<=range-max bps over lookback L) -> SWEEP poke beyond an
  edge by margin S -> 90m forward classify CONTINUATION(extends>=20bps) vs REVERSAL(re-enters range); two
  honest first-touch trades (run-overs incl, real fees): (A) REVERSAL maker@swept-edge (~6bps RT) + taker
  (~9bps), (B) CONTINUATION taker (~9bps); orderflow CONFIRM = depth-imbalance absorption(rev) vs stays-
  pulled(cont) as a no-lookahead gate. 10 days ETH+BTC, grid L{15,30,60}m x range-max{30,50,80} x
  margin{5,10,20} (knob-bite). VERDICT bad-first: NO tradeable pulse. (1) SPLIT is real+sizeable - reversal
  ~60-75% / continuation ~25-40%; rev mag ~45-80bps, cont mag ~52-126bps (ETH>BTC) = moves CLEAR the fee bar
  (unlike all prior leads). (2) CONFIRM does NOT separate them ahead of time: P(rev|absorption)~=P(rev|no-
  absorption) every trustable cell (e.g. ETH 15m/80/5 68% vs 69%); gate filters ~half (bites mechanically)
  but moves outcome prob ~0 = invalid separator (same dead imbalance, now as a chart-confirm). (3) EVERY n>=30
  cell both assets, all THREE trades NET-NEGATIVE: rev-MAKER gross-negative BEFORE fees (-8..-26bps, fills 100%
  by construction but INTO the continuing poke -> stopped, win 5-25% = resting at the edge catches the knife),
  rev-TAKER gross~0 net -7..-15, cont-TAKER gross~0 (60-75% reverse+stop, win 16-29%) net -8..-20. RR healthy
  (~2.0-2.8, targets>stops) but win% too low (overshoot trips the stop before the big move). BINDING CONSTRAINT
  SHIFTED: not the fee floor this time (size is big enough) but DIRECTIONAL UNCERTAINTY (can't pick rev vs cont)
  + STOP-BLEED + maker-fee-escape backfires (edge-rest fills into run-over). Killed cheap, 0 euros. Only follow-
  ups that target the bad entry = maker deeper (mid/far edge) or taker after confirmed re-entry, but all still
  need a directional read we don't have. docs/research/sweep-study.md; logs /root/runs/sweepscope/{eth,btc}.log
  + {eth,btc}_sweeps.csv. NEXT bigger-size lead must give a DIRECTIONAL read the orderflow can actually confirm.

- SWEEP / STOP-RUN SWING STUDY (2026-06-12, sweepscope - first BIGGER-SIZE lead) = NO tradeable pulse.
  Built crates/forgelag/src/bin/sweepscope.rs (analysis-only; mechanical no-lookahead: sideways range
  over lookback L -> sweep poke past edge by margin S -> 90m forward classify CONTINUATION vs REVERSAL;
  reversal trade maker@edge ~6bps + taker ~9bps, continuation taker ~9bps, first-touch target/stop with
  run-overs IN; orderflow confirm = depth-imbalance absorb/pull). clippy+16 unit tests+full suite green.
  10 days ETH+BTC, grid L{15,30,60}m x range-max{30,50,80} x margin{5,10,20}. VERDICT bad-first: every
  n>=30 cell both assets, all 3 trades NET-NEGATIVE. *** SIZE IS NOT THE PROBLEM this time *** (first
  lead where it isn't): moves are BIG - reversal ~60-75% of sweeps @45-80bps, continuation ~25-40%
  @52-126bps - well over the fee bar. THREE KILLERS: (1) DIRECTIONAL UNCERTAINTY - can't tell rev vs
  cont ahead of time; (2) STOP-BLEED - honest first-touch stops hit by the sweep's own overshoot before
  the big move (win 12-29% despite RR 2.0-2.8); (3) maker@swept-edge BACKFIRES (fills into the run-over,
  gross-neg before fees). *** KEY DURABLE FINDING: depth-of-book IMBALANCE confirm does NOT separate
  rev vs cont (P(rev|absorb) ~= P(rev|no-absorb), random) - it has now FAILED as the orderflow CONFIRM
  in EVERY study (Lagshot book-confirm, gap-close, cascade ob-confirm, sweep). Resting-depth imbalance
  is NOT predictive for us. *** Method-1 confirm needs a BETTER signal than book shape - likely actual
  AGGRESSIVE TRADE FLOW / absorption-at-price / CVD, not resting depth. docs/research/sweep-study.md;
  box /root/runs/sweepscope/. Killed cheap, 0 euros. Setup is real + big; we lack a directional read.
- SWEEP FLOW-SEPARATOR 3-state (sweepscope extended, branch forgelag, 2026-06-13) = PARTIAL WIN: the flow model SEPARATES where depth-imbalance was random (FLOW DECELERATION/exhaustion lifts P(reversal) ~+15-30pp over base on BOTH ETH+BTC, no-lookahead; impact-per-vol "absorption" works on BTC +12-41pp but INVERTS on ETH; easy-push weak) BUT still NO net-positive trade at trustable n>=30 (every gated rev/cont trade net-neg after fees, with+without --min-oidrop; green cells thin n=9-19). Master var = |microprice disp|/forced-vol over [fire,+60s]; rev maker@edge+taker, cont taker UNCHANGED; clippy+27 tests green, sacred core untouched. Real directional read finally exists (the unlock) but stop-bleed+overshoot+fee still kill the honest first-touch trade -> 0 promote. NEXT if revisited: trade STRUCTURE that survives overshoot (enter on confirmed re-entry / wider-scaled stop) on exhaustion-selected reversals. docs/research/sweep-study.md "Flow-separator" section; logs /root/runs/sweepscope_flow/.
- OPTION A/B DYNAMIC EXIT (sweepscope --dyn-stop/--rr/--stop-buffer + --flow-exit/--flow-exit-win/
  --flow-exit-k; default OFF, baseline byte-preserved; build+clippy(-Dwarnings)+31 tests green incl 4
  new) = NO net trade but the STOP IDEA IS VALIDATED. Bad-first: every trustable-n cell still NET-
  NEGATIVE after the 9bps taker fee (best ETH 15m/80 rr2 struct n=104 win40% GROSS +4.0 t1.49 -> NET
  -5.0). Good: the dynamic STRUCTURAL WICK-STOP (beyond the sweep extreme + buffer, no-lookahead) FIXED
  the stop-bleed - flipped ETH 15m/80 reversal taker from gross -1.4 to +4.0bps, win 27->40% = FIRST
  gross-positive cell at trustable n on this setup; confirms the trader's "no fixed RR" call (the fixed
  stop WAS the leak, tripped by the sweep's own overshoot). Option B flow-reaccel-exit ~ same, slightly
  worse. HIGHER reward multiple WORSE (rr3/4/6 all degrade - reversal doesn't run to big targets, win-
  rate collapses). Structural stops small ~10-14bps (entry 60s post-poke = wick near; win is from dynamic
  R:R not a wide stop). *** BINDING WALL back to the ~9bps TAKER FEE (gross +4 < 9) - same wall as every
  lead. *** NEXT puzzle piece = CUT THE FEE: MAKER entry (~6bps RT) gated on EXHAUSTION (picks the 80-90%
  reverters -> should dodge the run-over/adverse-selection that killed the earlier maker test) + structural
  exit. docs/research/sweep-study.md "Option A / Option B" section; logs /root/runs/optab*.
- MAKER FEE-ESCAPE (sweepscope --maker-rev + structural stop + EXH hold-gate; default OFF, clippy+32
  tests green) = REJECTED, WORSE than taker. Bad-first: GROSS goes NEGATIVE before fees on every cell
  both assets (-8 to -10bps, win 10-21%, t -5..-10, NET-of-6 ~ -14..-16). Maker fills ~100% (the
  aggressive poke always trades THROUGH the resting edge) = textbook ADVERSE SELECTION (fills precisely
  on the wrong side / continuation instant). EXHAUSTION hold-gate does NOT help - fill is AT the poke
  (before the 60s confirm), so the gate only flattens at window-end = locks the adverse move (gated
  cells MORE negative, RR ->0.2-0.9). Cheaper 6bps fee irrelevant when gross is deeply negative.
  *** SWEEP LEAD EXHAUSTED - both doors closed: TAKER (struct stop) gross +4 but 9bps fee -> net -5;
  MAKER 6bps fee but adverse-selection gross -8 -> net -14. The reversal is only a-few-bps gross =
  below the fee floor (SAME WALL as every lead). No deployable strategy; 0 euros risked. *** DURABLE
  WINS to reuse: (1) EXHAUSTION/flow-deceleration = real no-lookahead directional read (+15-30pp
  P(reversal), both assets, first separator to beat random); (2) dynamic STRUCTURAL WICK-STOP fixes
  stop-bleed (turns the taker reversal gross-positive). Both reusable for a FUTURE edge that is BIGGER
  per trade (>>9bps) or runs on LOWER fees. docs/research/sweep-study.md "Option A maker" section;
  logs /root/runs/maker/.
- *** TEST A RECLAIM ENTRY (sweepscope --reclaim: wait for price back INSIDE the range, then reversal
  taker, structural stop beyond the wick; default OFF, clippy+34 tests green) = BEST RESULT YET, FIRST
  net-positive cells. *** Good: the reclaim entry closes the capture gap the 60s-late poke entry was
  bleeding. "all" reclaim gross ~0 -> +4-10bps, win 43-51% (ETH 15m/80 rr1.5 +6.7 t1.50; BTC 30m/80
  rr1.5 +10.4 t1.81 NET +1.4 n=45). EXHAUSTION-gated subset goes NET-POSITIVE after the FULL 9bps taker:
  ETH 15m/80 +EXH rr1.5/2/3 = +5.2/+7.3/+8.6bps NET (n=11, win 55%, gross +14-18, RR 2.0-2.3, positive
  on EVERY reward dial = robust); BTC 15m/80 rr1.5 +EXH +5.9 (n=7). CONFIRMS the 85%-revert read was a
  real edge all along - the late entry threw it away; entering on the reclaim recovers it. Bad/honest:
  net-positive cells THIN (n=7-11, t~1.1-1.3 = NOT yet significant, need t>2); bigger-n "all" cells
  ~breakeven (best +1.4); 30m/BTC +EXH shaky small-n. PROMISING, NOT PROVEN (could be noise). rr 1.5-3
  all work on ETH +EXH; rr2 a good middle; stop dist 26-38bps (dynamic, beyond the printed wick). NEXT
  GATE = MORE DAYS on the winning config (reclaim+EXH, ETH 15m/80, rr2) for trustable n + t>2 (have 61d
  Nov-Dec25 + Feb from prior OOS pulls), then OOS. docs/research/sweep-study.md "Test A" section; logs
  /root/runs/reclaim/.
- *** TEST A RECLAIM @ SCALE (61d train Nov-Dec25 + 36d OOS Feb-Jun26, reclaim+EXH rr2, ETH+BTC) =
  DID NOT SURVIVE - the 10-day net-positive was SMALL-SAMPLE OPTIMISM. *** Bad: the EXHAUSTION
  separator COLLAPSES at scale - P(rev|exhausted) vs base = ETH train 15m/80 73% vs 70% (+3pp),
  30m/80 +1pp, BTC NEGATIVE (71 vs 75) - the +15-30pp on 10 days was an ARTIFACT (now ~+1-3pp = gone).
  +EXH NET mostly neg/breakeven (ETH train 15m/80 -3.0 n=60, 30m/80 +0.6 n=45; BTC train 30m/80 -11.4)
  and does NOT replicate train<->OOS (BTC 30m/80 +EXH train -11.4 vs OOS +13.3 t2.48 n=17 = SIGN FLIP =
  noise; ETH 15m/80 +EXH DIED in OOS net -8.0). Big-n "all" reclaim ~breakeven gross (ETH train +2.5/+3.0,
  OOS -0.1/+0.9), never near 9bps fee. Good(small/honest): the reclaim ENTRY is a real mechanical capture
  improvement (moved "all" gross negative->~0-+3, win 38-45%) - keep as a TECHNIQUE, not an edge by itself.
  *** SWEEP LEAD CONCLUDED: real setup + reclaim entry improves capture + structural wick-stop fixes bleed,
  but NO robust net edge survives proper sample+OOS; the exhaustion "tell" was OVERFIT to 10 days. Bigger
  sample caught the overfit BEFORE risking euros (process working). *** DURABLE/REUSABLE: (1) reclaim entry,
  (2) structural wick-stop, (3) LESSON: a 10-day separator MUST be re-checked at 60d+/OOS before trust.
  0 euros. docs/research/sweep-study.md "Test A @ SCALE" section; logs /root/runs/rcval/.
- VOLUME-PROFILE LVN MEAN-REVERSION (NEW tool vpscope, branch forgelag, analysis-only, sacred core
  untouched; rolling NO-LOOKAHEAD volume profile -> POC/value-area/LVN; clippy+6 tests green) STEP 1
  (location ALONE, taker) = NO EDGE at scale. Built per trader's LVN-reversion idea. 61d train Nov-Dec25
  + 36d OOS Feb-Jun26, ETH+BTC, grid lookback{1,2,3,4h}x lvn-frac{0.15,0.25}. Bad: revert-TAKER "all"
  gross ~0 EVERY cell (-4..+1.6bps), NET -8..-13 after 9bps; win 47-62% but RR 0.65-1.0 (reach-POC
  target often unmet, losers run). The 2-day smoke (+5-6 net) was SMALL-SAMPLE NOISE - confirmed
  instantly at scale (sweep lesson again). Mild CONSISTENT structure: shorter lookback (1h) reverts more
  (55-58%, holds train+OOS); BTC "far-from-value" LVNs gross-positive BOTH periods (+2..+6) but BELOW
  the fee; ETH thin/far sign-flips train<->OOS = noise. SAME WALL = ~9bps taker vs ~0-6bps gross.
  Location alone NOT tradeable as taker. NEXT (planned step2 = add exhaustion timing) is a LONG SHOT
  (location gross~0 + exhaustion already collapsed at scale); honest = taker microstructure @9bps
  structurally dead, escapes = lower-fee/maker (but LVN=thin=adverse sel) or much bigger moves.
  docs/research/volume-profile-study.md; logs /root/runs/vpval/.
- *** VP LVN RESEARCH-ON-FILES + FEE CORRECTION (2026-06-13, vpscope --fee configurable [default 6 =
  taker-in+maker-out], --dump added; clippy+6 tests green). CORRECTS the prior "fee wall" framing. ***
  Dumped 661 ETH LVN detections (61d, 1h profile) and analysed the file directly: MOVES ARE BIG (mean
  favourable toward value 70bps, mean adverse away 76bps) so a 6-9bps fee is TRIVIAL at this scale -
  the trader is RIGHT that 5/15/1h structure covers fees. The real binding constraint is DIRECTION: a
  raw LVN touch is a ~COIN FLIP (fav 70 ~= adv 76, ratio 0.92; bigger excursion toward value 327 vs
  away 334 = 49/51); every target/stop combo (T30-60 x S15-30) approx gross ~ -1..+0.2bps, clean-losses
  ~2x clean-wins (thin nodes get traversed as often as they reject - that's WHY they're thin). No fee
  or stop tuning fixes a coin flip. *** This VALIDATES trader method-1: the level is only the TRIGGER
  (coin flip alone); the EDGE is the ORDERFLOW CONFIRM (absorption/rejection on heavy volume = revert
  vs push-through on thin = continue) he adds BY HAND and we never mechanised at the LVN. SELF-CORRECTION:
  I over-attributed failures to fees; at his scale the move covers the fee, the binding constraint is
  the directional confirm. *** NEXT = build absorption/impact orderflow confirm AT the LVN, re-test
  location+confirm on 61d train + 36d OOS at fee 6. Caveat: depth-imbalance confirms were weak before
  but at arbitrary edges; LVN is a real structural level = fair test of his method. docs/research/
  volume-profile-study.md.
- VP LVN ORDERFLOW CONFIRM (method-1, vpscope --confirm-window 20s: absorption=heavy push-vol+low
  price-impact -> hypothesised REVERT, vs push-through=high impact -> continue; entry at window-end,
  no-lookahead; clippy+7 tests green) = DOES NOT SEPARATE. 61d train + 36d OOS, ETH+BTC, 1h/2h. Bad:
  P(reversion|absorbed) at/BELOW base in nearly every cell (lift -2..-14pp; hypothesis flat-to-INVERTED);
  CONFIRMED trade net-negative everywhere (-3.8..-9 net of 6bps), "absorbed" gate usually WORSE than
  "push-through" (BTC push-thru gross +2-3 = opposite of thesis = noise). SECOND natural orderflow
  confirm to fail to separate (first = depth-imbalance, failed every prior study). A simple single-window
  orderflow metric at a level adds NO directional edge here; LVN stays a coin flip. NEXT requires the
  trader to NAME the specific observable he reads (CVD divergence / absorption-at-price / resting-wall
  stacking-spoof / multi-bar rejection) so we test THAT precisely (blind variant-hunting = overfit).
  vpscope confirm infra reusable. docs/research/volume-profile-study.md; logs /root/runs/vpcf/.
- *** LADDER (grid) ENTRY on the sweep reversal (trader's method; sweepscope simulate_ladder: N maker
  rungs INTO the dislocation, avg entry = filled rungs, hard invalidation = full-size stop, run-overs IN;
  clippy + 38 tests green; fee 6) = PER-TRADE LIE, euro-realistic NEGATIVE. *** Per-trade (each trade=1
  unit) looked like the BEST result ever: net +8..+12bps, win 69-73%, t 5-9, REPLICATED OOS (BTC r120
  train +10.9 -> OOS +12.2). It was a SIZE ILLUSION: ladder fills PARTIAL size (~2 rungs) on winners,
  FULL size (5 rungs) on the invalidation run-overs. SIZE-WEIGHTED (euro-realistic, weight by deployed
  capital) = EVERY one of 36 configs (rungs{3,5} x range{40,80,120} x invalid{10,20,40}) BOTH assets
  NEGATIVE -5.0..-11.2bps, NONE positive; worst-case full-size hit -115..-563 bps-units. ROOT CAUSE:
  deep overshoots fill the most rungs (most size) but are disproportionately real BREAKOUTS that don't
  revert -> ladder loads max size into the losers; no stop/range/rung fixes it (whole space swept).
  CONSTRUCTIVE: the ladder DID fix the FEE (6 vs 9) and ENTRY-TIMING (per-trade flipped breakeven->strongly
  positive) = the trader was RIGHT those were real problems; the remaining killer is the SIZE asymmetry
  (averaging up into trends), intrinsic to a fixed-per-rung grid, needs a deep-end reversal-vs-continuation
  read we never found. *** PERMANENT LESSON: judge any grid/ladder/DCA SIZE-WEIGHTED, never per-trade -
  per-trade hides that losers are bigger than winners (the prior-project lie class). size-weighted +
  worst-case now in sweepscope. *** docs/research/sweep-study.md "Ladder" section; logs /root/runs/lad*.
- *** LADDER CORRECTION + FIRST OOS-VALIDATED EDGE (2026-06-13). Trader clarified the ladder = ONE
  trade (fixed position, rungs only set the AVG ENTRY), not one-trade-per-rung -> my size-weighted
  (per-rung) NEGATIVE view was the WRONG lens for his sizing (that lens only applies if deeper fills
  add risk = grid/martingale). Under FIXED-SIZE-per-trade (risk capped to the invalidation), PER-TRADE
  equal-weight is correct. *** RESULT (97 days full, ETH r120/i30 + BTC r80/i30, lookback15m margin5,
  rungs5, fee6): ETH n=905 win72% net +11.2bps t=10.5; BTC n=587 win70% net +10.6bps t=9.5; ~15
  combined trades/day; worst single -106bps. Validated IN + OUT of sample (train 61d -> OOS 36d both
  hold) = the ONLY OOS-replicating edge in the whole project. EUR (NON-COMPOUNDED fixed notional, no
  sequential-compound lie): EUR500/~3mo -> 1x +163% DD3.4%, 2x +326% DD5%, 4x +653% DD9.4%. The
  sequential-compounded EUR260k/+51858% = FANTASY (rejected: ~15 overlapping trades/day can't compound
  serially nor all carry full size). UNLOCKED by the trader's execution fixes: ladder entry + correct
  maker fee (6 vs 9) + let it run. NOT yet deployable - GATES: (1) concurrency-capped sizing sim
  (can't 4x every concurrent trade -> realistic ~1x-2x line), (2) partial fills, (3) maker-fill realism,
  (4) slippage + -106bps tail, (5) live paper. NEXT = build a concurrency-capped position-sizing sim +
  live paper, NOT more param hunting. docs/research/sweep-study.md "Ladder CORRECTION" section; CSVs
  /root/runs/lad_eth.csv,lad_btc.csv; per-config logs /root/runs/lad*.
- *** CONCURRENCY-CAPPED SIZING SIM (2026-06-13) = PASSED, concurrency is a NON-issue. *** Built
  event-driven sim from the ladder dumps (entry fire_ts + exit ladder_exit_ts added to sweepscope dump;
  clippy+36 tests green). KEY: MAX concurrent open positions = ONLY 2 across the whole 1492-trade,
  222-day window (30m cooldown + fast resolution => signals rarely overlap within a coin; it is basically
  ETH-trade + BTC-trade at most). So a cap of 2 skips ZERO trades and compounding is LEGITIMATE (not the
  earlier 15-trades-stacked fantasy). Realistic EUR500 (event-driven, proper time model, ~222d window):
  cap3 1x-notional (safe) -> EUR2501 (+400%) maxDD 5.6% worst1 -EUR25; cap2 2x -> EUR11970 (+2294%)
  maxDD 11% worst1 -EUR233; cap1 4x (aggr) -> EUR59584 maxDD 25%. (worst1 EUR large only because equity
  compounded up; %DD is the risk gauge.) ~6.7 trades/day over 222d (~15/active-day). The earlier
  sequential-compound EUR260k was inflated by NOT modeling time; proper sim = +400% conservative /
  +2294% @2x with single-digit-to-11% DD. *** Sizing/concurrency check PASSED rather than exposing a lie.
  *** STILL UNVERIFIED before live: (1) partial fills (each trade = full size assumed), (2) taker-exit
  slippage, (3) maker-fill realism as equity/size grows, (4) regime coverage (trades cluster Nov-Dec +
  Feb + May-Jun), (5) LIVE PAPER run. This is now the FIRST genuine deployable CANDIDATE. NEXT = live
  paper run on the AWS Tokyo box (HL), tiny size, the ladder sweep logic. scripts: _concsim.ps1; CSVs
  /root/runs/lad_eth.csv,lad_btc.csv.
- *** LADDER REALISTIC-SIZING CORRECTION (2026-06-13) - RETRACTS the "deployable candidate" claim. ***
  Trader asked the right question: did price fill ALL orders, and what is the % size per trade. FILL
  DISTRIBUTION (5-order ladder, 1492 trades): 1 order=40%, 2=22%, 3=12%, 4=8%, ALL 5=18%; avg 2.4/5.
  EUR with HIS sizing 20% margin x 20x, max-2-concurrent, two sizing models: (a) FIXED (every trade
  counted as full 20% size regardless of fills) = +47909% (THE FANTASY - treats a 1-order partial fill
  as full size, NOT achievable with a passive limit ladder); (b) SCALE = 5 real orders of 4% each,
  deployed grows as they fill (THE REALISTIC LADDER) = EUR500 -> EUR36 = -93%, maxDD 94.7% = ACCOUNT
  BLOWN. *** The earlier +400%/+11bps used the FIXED (equal-weight) accounting = WRONG; realistic ladder
  LOSES. *** ROOT CAUSE: you win SMALL on shallow pokes (40% fill only 1 order = tiny position, revert)
  and lose BIG on deep fills (18% fill all 5 = disproportionately real BREAKOUTS that hit the stop at
  FULL size). The ladder puts the MOST capital on exactly the losing trades; per-euro-deployed expectancy
  is NEGATIVE (= the size-weighted -6..-11bps found earlier, now in stark EUR). The reversion read is REAL
  (~70% of pokes revert) but lives in the SHALLOW pokes that are too small to size; size only lands on the
  deep breakouts. *** EDGE IS REAL-BUT-UN-SIZEABLE. Sweep+ladder CLOSED. *** No sizing that adds size on
  deeper fills can be positive (per-euro neg). Single-entry-at-poke = the shallow version = the breakeven
  reclaim already tested. PERMANENT LESSON reinforced: always report FILL-RATE + per-euro SIZE-WEIGHTED
  EUR, never per-trade equal-weight, for any ladder/scale-in. scripts _fillsim.ps1.