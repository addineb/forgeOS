# Bot Edge Workflow (per-bot lifecycle)

GOAL: a bot with a robust win-rate x risk:reward that nets POSITIVE after
realistic costs (~11 bps round-trip). "Edge" -- not literally predicting a price push.

## The loop (run for every bot)
1. THESIS -- one line: the idea, and why it should have edge.
2. HARD RESEARCH -- papers / docs / blogs / quant+algo forums / sites. Produce a
   SPEC SHEET (docs/specs/<bot>.md): params the literature says matter + sane
   ranges, entry/exit logic, expected regime, known failure modes, CITATIONS.
3. HARNESS THE BOT -- set the research-backed specs into the bot.
4. SWEEP the variances (methodology below).
5. BACKTEST ENGINE -- our faithful harness, realistic fills, per-regime, IS/OOS, gates.
6. VERDICT -- promote / gulag (tune) / retire, against PRE-DECLARED kill criteria.

## Sweep methodology (research-grounded)
- Coarse -> fine. Coarse grid finds the region; fine grid refines around it.
  Never brute-force a giant grid (picking the single best combo = discovering
  NOISE, i.e. selection bias).
- Every knob must BITE: verify each swept param actually changes behaviour across
  its range before trusting the result. (S1 OFI threshold did not bite -- below
  BTC scale.) Prefer scale-free / percentile / z-score thresholds.
- PLATEAU, not peak: trust settings whose NEIGHBOURS also work. A broad positive
  region is robust; a lone spike is curve-fit -- reject it.
- SIGNAL-FIRST: confirm the move exists with MFE/MAE BEFORE tuning exits. No
  stop/target saves a signal with no forward edge.
- OUT-OF-SAMPLE: walk-forward IS/OOS; edge must hold OOS. CPCV is the stronger
  multi-path standard; walk-forward is the cheap default.
- MULTIPLE-TESTING correction: Deflated Sharpe (penalise # combos tried) +
  Probability of Backtest Overfitting (PBO / CSCV).
- REALISTIC fills + fees always.
- Do NOT sweep to MANUFACTURE edge -- only to TUNE one already showing a pulse.
  If the coarse pass is flat/negative everywhere (selective), the signal is dead.

## Pre-declared kill criteria (avoid endless tweaking)
RETIRE when, on a properly-selective coarse sweep across clean windows with
realistic fills, NO config is net-positive in BOTH in-sample AND out-of-sample.
Otherwise GULAG (tune) or PROMOTE (passes gates).
Rules: no kill without a swept trial; no promote on default-config or IS-only.

## Sources (paraphrased for licensing)
- Walk-forward vs curve-fitting: quantstrategy.io, nexusfi.com, hmaquant.
- Deflated Sharpe / PBO: Bailey & Lopez de Prado; quantbeckman.com.
- CPCV superiority over walk-forward/K-fold: Arian, Norouzi & Seco (SSRN 4778909).
