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