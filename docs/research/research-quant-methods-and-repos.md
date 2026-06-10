# Research Note -- How Quants Build Strategies + Repos to Mine (2026-06-09)

A different kind of research: not "what is the edge" but "HOW do practitioners
turn research into a working strategy, and WHERE is the open code we can learn
from". Goal = borrow methodology and implementation patterns, mapped to OUR work.

## SAFETY (non-negotiable)
External code is UNTRUSTED REFERENCE. Read it for ideas/patterns; never run it
blindly against our keys/data. Re-implement in our own deterministic primitives.

================================================================================
PART A -- The quant research-to-strategy pipeline (what to ADOPT)
================================================================================
The dominant modern framework is Lopez de Prado's "Advances in Financial Machine
Learning" (AFML). The pipeline and its named techniques, each mapped to us:

1. STRUCTURED BARS, not time bars. Sample data on volume/dollar/imbalance bars so
   each observation carries comparable information. -> Maps to our CVD volume-
   clock upgrade (research-flow-primitives T3) and VPIN.

2. TRIPLE-BARRIER LABELLING. Label a trade by which barrier it hits first:
   take-profit, stop-loss, or time. -> This is literally how we should LABEL
   outcomes in backtest analysis (and it formalises the TP/SL/hold grid).

3. META-LABELLING (the big one for us). A SECONDARY model decides whether to ACT
   on the primary signal -- a gatekeeper that passes only high-confidence signals
   and sizes them. -> This is the principled version of EVERY gate/filter idea in
   our notes: the VPIN gate (Q5 T6), the trend/regime filter (strategy-edges #2),
   the spoof-as-FAKE filter (Q4), the predicted-move>cost filter (Q2). Frame them
   all as ONE meta-label layer on top of the OFI/sweep/absorption primaries.

4. PURGED / COMBINATORIAL CV (CPCV). Cross-validation that purges and embargoes
   overlapping samples; CPCV is shown to beat walk-forward at reducing the
   Probability of Backtest Overfitting (Arian, Norouzi & Seco, SSRN 4778909).
   -> An upgrade to our current DSR/OOS gate.

5. DEFLATED SHARPE / PBO. Because we SWEEP many configs (cartesian grid), the
   multiple-testing problem inflates false positives badly. Deflated Sharpe Ratio
   and Probability of Backtest Overfitting correct for "how many things did you
   try". -> We already use DSR; ADD PBO and track the number of trials per sweep.

6. OPTIMAL TRADING RULES WITHOUT BACKTESTING (Lopez de Prado, SSRN 2502613).
   Derive exit thresholds from a fitted Ornstein-Uhlenbeck process instead of
   grid-searching TP/SL. -> Directly relevant to our hold-time/exit work (Q2-C,
   T5): a way to set exits that does NOT overfit the trade-management grid.

The recurring practitioner warning: finance has very low signal-to-noise;
multiple testing + leakage + non-stationarity are the killers. Our DSR/OOS habit
is correct; meta-labelling + CPCV + PBO are the upgrades.

================================================================================
PART B -- Repos & frameworks to mine (read for patterns)
================================================================================

## Backtest/exec frameworks (architecture references)
- NautilusTrader -- github.com/nautechsystems/nautilus_trader. Event-driven,
  IDENTICAL core across backtest and live. This is EXACTLY our "same BotEngine
  path as live" principle -- the best architectural reference for our harness.
- vectorbt -- github.com/polakowo/vectorbt. Vectorised, run thousands of param
  combos fast. Useful for sweep speed -- but it AMPLIFIES the multiple-testing
  risk, so pair with PBO.
- Hummingbot -- github.com/hummingbot/hummingbot. Open-source crypto market-
  making framework; connector + strategy patterns.
- Freqtrade -- crypto strategy/backtest framework (large community, many example
  strategies to study).

## Methodology code (the AFML toolkit)
- mlfinlab (Hudson & Thames) -- github.com/hudson-and-thames/mlfinlab. Triple-
  barrier, meta-labelling, purged/combinatorial CV, DSR, PBO, bet sizing. The
  reference implementation of Part A.
- mrbcuda/pbo -- github.com/mrbcuda/pbo. Probability of Backtest Overfitting.

## Limit-order-book / microstructure (closest to our work)
- mansoor-mamnoon/limit-order-book -- LOB engine + real crypto/equity replay +
  spread/imbalance/impact analytics + VWAP/TWAP/POV/MM backtest. Mirrors our
  replay + OFI + impact agenda.
- abhi-wadhwa/limit-order-book -- price-time matching + Avellaneda-Stoikov market
  making + microstructure analytics (the canonical MM model, if we ever do maker
  on HL).
- silue-dev/limit-order-book-market-making -- MM on a matching engine (clean,
  readable).
- shaileshkakkar/OrderImbalance -- order-imbalance HFT strategy (OFI patterns).
- kostyafarber/crypto-lob-data-pipeline -- streaming crypto LOB -> OFI indicator
  (a capture+feature pipeline like ours).
- tspooner/rl_markets -- RL market making (research-grade; ideas not production).
- davecliff/BristolStockExchange -- minimal LOB exchange sim (for agent/ABM
  experiments).

## Idea/strategy catalogs & lists
- wilsonfreitas/awesome-quant -- the master curated list of quant libraries.
- Kakushadze & Serur, "151 Trading Strategies" (arXiv:1912.04492) -- 150+
  strategies with formulas across asset classes (incl. crypto); an idea index.
- Quantpedia / paperswithbacktest -- strategy encyclopedias with backtests and
  the "pitfalls/overfitting" practitioner wisdom.

================================================================================
PART C -- Concrete adoptions for OUR lab (prioritised)
================================================================================
1. META-LABEL LAYER: unify our gates (VPIN, trend, spoof, predicted-move>cost)
   into one secondary "should I take this primary signal?" model. Highest-value
   structural idea -- it is how the pros turn a noisy primary into a tradeable one.
2. PBO + trial-count on every sweep: our cartesian sweeper is a multiple-testing
   machine; add Probability of Backtest Overfitting beside DSR. Cheap, high-value.
3. CPCV instead of single IS/OOS split for validation robustness.
4. OU-based exits (Optimal Trading Rules) as an alternative to grid TP/SL -- less
   overfit; pairs with Q2-C / T5.
5. Triple-barrier labelling in the trade-analysis layer (cleaner outcome labels).
6. Architecture: keep validating our "same code path backtest<->live" against
   NautilusTrader's design; mine the LOB repos for OFI/impact/MM implementations.

## Honest note
None of this CREATES edge -- it is how you AVOID FOOLING YOURSELF about edge and
how you turn a weak primary signal into a tradeable one (meta-labelling). Given
our standing problem (mostly net-losing bots, no validated edge, thin data), the
overfitting-control toolkit (PBO/CPCV/DSR + meta-labelling) is arguably as
important as any single signal. Treat all linked code as untrusted reference.