# Handoff Document — forge-anomaly Causal Engine Project

This document captures everything needed to resume work in a fresh chat. It includes the full decision history, current code state, what just happened before handoff, and the explicit next steps.

---

## Project Context

**Repo:** `forge-anomaly` crate inside `C:\Users\User\.kiro\forgeOS`
**Hetzner box:** `root@167.233.57.140` — all data lives here, NEVER run locally, always use Hetzner
**Hetzner data path:** `/root/depthscope_out/` (e.g., `BTCUSDT_2026-06-02_vb10.csv`, `stitched_vb10.csv`)
**Hetzner repo path:** `/root/forgeOS/`
**Hetzner cargo:** `/root/.cargo/bin/cargo`
**Hetzner binary location:** `/root/forgeOS/target/release/validate`

## Tool Decisions (Final)

| Tool | Status |
|---|---|
| CVD | Keep (Primary) |
| Absorption | Keep (Primary) — strict `>` in `features.rs::compute_absorption` |
| Liquidity Vacuum | Keep (Primary) |
| Large Print | Keep (Primary) |
| Depth Imbalance | Filter only (precondition gate) |
| VolDelta Divergence | Confirmation only |
| OFI | Drop for now |
| CVD Momentum | Drop as feature; computed check at template step 3 |
| Aggressor Ratio | Drop |
| Trade Intensity | Drop |
| Mahalanobis + z-scores + FDR | Drop in causal mode (legacy still has them) |
| PatternRepeat | Drop (in causal mode) |

## Architecture

```
EngineConfig.engine_mode = EngineMode { Legacy, Causal }   # default: Legacy
```

- `--engine legacy` (default): unchanged Mahalanobis pipeline
- `--engine causal`: runs new CausalEngine + absorption_reversal template
- Both modes share the same `validate` binary output schema for A/B comparison

## Files Created/Modified

```
crates/forge-anomaly/src/
  causal.rs                          # EngineMode, Step, TemplateOutcome, CausalSignal,
                                     #   AbsorptionReversalParams, CausalTemplatesConfig,
                                     #   CausalRollingBuf
  causal/mod.rs                      # re-exports
  causal/template.rs                 # CausalTemplate trait + DiagnosticSnapshot
  causal/confidence.rs               # causal_completeness() (replaces calc_confidence)
  causal/rate_limit.rs               # per-template rolling rate cap (replaces null-edge)
  causal/engine.rs                   # CausalEngine: FeatureExtractor + rolling buffer +
                                     #   templates + rate limiter
  causal/templates/absorption_reversal.rs  # FIRST template: 3-step causal chain
  features.rs                        # strict-`>` absorption change
  types.rs                           # EngineConfig.engine_mode + causal fields
  lib.rs                             # pub use causal::{...}
  bin/validate.rs                    # --engine + --diagnostic flags
```

## First Template: Absorption → Exhaustion → Reversal

Causal chain:
1. **Pressure:** heavy one-sided CVD (`|cvd_delta| > 0.6 × rolling_cvd_std`)
2. **Absorption:** defending-side price strictly improved (`bid > prev.bid` for Long, `ask < prev.ask` for Short) AND defending-side absorption > 0.3
3. **Deceleration:** CVD same sign as step 1 AND magnitude < 0.5 × step1 magnitude

Precondition gate: defending-side depth_imbalance > 0.25.

Template ONLY emits when all 3 steps fire. Step 1 fires only when no episode is currently armed (no re-arming).

## Recent Decision: Diagnostic Mode (in progress)

The user asked me to implement `--engine causal --diagnostic` to localize where the chain breaks on real data, BEFORE changing thresholds or logic. Currently in the middle of this work.

### What I just did (NOT YET COMMITTED):
1. Added `DiagnosticCounters` struct to `absorption_reversal.rs` with fields:
   `bars_evaluated`, `step1_attempts`, `step1_fired`, `step2_attempts`, `step2_fired`, `step3_attempts`, `step3_fired`, `step1_expired`, `sign_flip_rejected`, `absorption_strict_failed`
2. Added `diag: DiagnosticCounters` field to `AbsorptionReversalTemplate`
3. Added `pub fn reset_diag(&mut self)` and `pub fn diagnostic(&self) -> &DiagnosticCounters` methods
4. Incremented counters at each transition point in `evaluate()`:
   - `bars_evaluated` at top
   - `step1_attempts` always; `step1_fired` after step 1 fires
   - `step2_attempts` after recency check passes; `step2_fired` after absorption passes; `absorption_strict_failed` when absorption didn't hold
   - `step1_expired` when recency window exceeded
   - `step3_attempts` when step 2 was active; `step3_fired` after decelerated; `sign_flip_rejected` when sign flipped
5. Added `DiagnosticSnapshot` struct to `causal/template.rs` with same fields plus `template_id: String`
6. Added `fn diagnostic(&self, template_id: &str) -> Option<DiagnosticSnapshot>` to `CausalTemplate` trait (default returns `None`)
7. Implemented `diagnostic()` on `AbsorptionReversalTemplate` (returns snapshot if id matches)
8. Added `pub fn diagnostics(&self) -> Vec<DiagnosticSnapshot>` on `CausalEngine`
9. Added `--diagnostic` CLI flag to `validate.rs`
10. Wired `run_causal_mode` to take `diagnostic: bool` parameter and print diagnostic table at the end
11. Added 2 new unit tests: `diagnostic_counters_track_chain_progress` and `diagnostic_counts_sign_flip`

### Compile error at handoff:
The new tests in `mod tests` of `absorption_reversal.rs` call `t.diagnostic("absorption_reversal")`. This is currently failing because:

```
error[E0061]: this method takes 0 arguments but 1 argument was supplied
   --> crates/forge-anomaly/src/causal/templates/absorption_reversal.rs:560:22
```

**Root cause:** The `AbsorptionReversalTemplate` struct has TWO methods named `diagnostic`:
- Inherent struct method (line 100): `pub fn diagnostic(&self) -> &DiagnosticCounters`
- Trait method (line 127, in `impl CausalTemplate for AbsorptionReversalTemplate`): `fn diagnostic(&self, template_id: &str) -> Option<DiagnosticSnapshot>`

When the test calls `t.diagnostic("absorption_reversal")`, Rust resolves to the inherent method (no args) which conflicts. The fix is to either:
- Rename the inherent method to something like `diag_counters()` to avoid the conflict
- OR use fully-qualified syntax `CausalTemplate::diagnostic(&t, "absorption_reversal")`
- OR remove the inherent `diagnostic()` method since the trait method supersedes it

**Recommended fix:** Remove the inherent `diagnostic(&self) -> &DiagnosticCounters` method (lines ~100-103) since it's not used outside tests and the trait method provides the same data via `DiagnosticSnapshot`.

### Code state to remember:
- Tests are at line 560, 575, 589, 621 in `causal/templates/absorption_reversal.rs`
- All 4 sites call `t.diagnostic("absorption_reversal")` expecting the trait method (returns `Option<DiagnosticSnapshot>`)
- The diagnostic output format string in `run_causal_mode` looks like:
  ```
  metric                          bars  step1   →step2   →step3  expires  absfail  signflip
  absorption_reversal              N     f/t      f/t      f/t      N        N        N
  ```

## Git State

- Last pushed commit: `76caeee` "validate binary: --engine legacy|causal flag + causal-mode report"
- Branch: `main`
- All uncommitted work is in the working tree (NOT staged, NOT pushed)
- Files changed but uncommitted:
  - `crates/forge-anomaly/src/causal/template.rs` (added DiagnosticSnapshot)
  - `crates/forge-anomaly/src/causal/engine.rs` (added diagnostics() method)
  - `crates/forge-anomaly/src/causal/templates/absorption_reversal.rs` (added DiagnosticCounters + counter increments + 2 new tests)
  - `crates/forge-anomaly/src/bin/validate.rs` (added --diagnostic flag + diagnostic output section)

## Test State

- 53/53 lib tests pass BEFORE the diagnostic changes
- Tests are currently broken due to the dual `diagnostic` method conflict described above
- After fix (remove inherent method): expect 55/55 passing (53 + 2 new diagnostic tests)

## A/B Test Results (the result that triggered diagnostic mode)

| Date | Legacy signals | Legacy conf avg | Causal signals |
|---|---|---|---|
| 2026-06-02 | 12 (5 pattern, 7 maha) | 0.739 | **0** |
| 2026-06-03 | 11 (2 pattern, 9 maha) | 0.836 | **0** |
| 2026-06-04 | 9 (4 pattern, 5 maha) | 0.839 | **0** |
| 2026-06-05 | 12 (7 pattern, 5 maha) | 0.711 | **0** |
| **stitched (18 days, 40,017 bars)** | **209 (134 pattern, 75 maha)** | 0.761 | **0** |

Causal template produced **zero signals** across all 5 datasets. User wants to localize the failure first via diagnostic mode, NOT loosen thresholds blindly.

## User's Plan for After Diagnostic (locked in)

1. Run `--engine causal --diagnostic` on stitched 18-day dataset
2. Show where the chain breaks
3. Then choose from these fixes (ranked by user's preference):
   - **Option 1 (Recommended):** Relax Step 3 — also allow `cvd_delta` near zero (pressure simply stops), not just same-sign fade
   - **Option 2:** Slightly loosen thresholds (`cvd_pressure_threshold` 0.6→0.45, `absorption_hold_threshold` 0.3→0.22, `deceleration_ratio` 0.5→0.65)
   - **Option 3:** Revert absorption back to `>=` (or small tolerance)
   - **Option 4:** Accept that current 3-step story is too narrow; reconsider template

## Rules Still In Force

- NEVER run anything locally — always Hetzner
- One template/philosophy at a time
- Practical, not over-engineered
- Legacy mode must remain default + bit-identical to before
- Wait for user confirmation before loosening thresholds or changing causal logic

## Prompt for the New Chat

```
You are a senior Rust developer working on the `forge-anomaly` crate inside the ForgeOS project.

## Handoff

I (the previous chat) was implementing `--engine causal --diagnostic` mode to localize where the absorption_reversal causal template breaks on real data. The full context, decision history, and current code state are in:

  C:\Users\User\.kiro\forgeOS\HANDOFF.md

Read that file FIRST. It has everything.

## Critical context

- **Hetzner box is the only place to run anything.** Local runs are forbidden.
  Host: `root@167.233.57.140`. Always pull, build, run there.
- Hetzner cargo: `/root/.cargo/bin/cargo`
- Hetzner data: `/root/depthscope_out/BTCUSDT_*.csv` and `/root/depthscope_out/stitched_vb10.csv`
- Hetzner repo: `/root/forgeOS/`
- Hetzner binary: `/root/forgeOS/target/release/validate`
- To sync Hetzner with latest code:
  ```
  ssh root@167.233.57.140 "cd /root/forgeOS && git fetch origin && git reset --hard origin/main && /root/.cargo/bin/cargo build --release -p forge-anomaly"
  ```
  (Note: Hetzner often has uncommitted local files blocking fast-forward; `git reset --hard origin/main` handles it.)

## What's already done (committed and on origin/main)

1. `absorption_reversal` causal template (3-step: Pressure → Absorption → Deceleration)
   - Strict `>` in `features.rs::compute_absorption` (absorption won't fire on equal-price bars)
   - Step 3 requires same-sign CVD (sign flip is rejected as different story)
   - Step 1 only fires when no episode is armed (no re-arming)
2. CausalEngine + rate limiter + causal_completeness confidence formula
3. `--engine legacy|causal` flag in `validate` binary (Legacy is default)
4. Both modes share the same output schema for A/B comparison
5. A/B tested on 18-day stitched + 4 single dates: legacy produces ~10-12 signals/day, causal produces ZERO.

## What's NOT done (next steps in order)

1. **Fix the compile error from the diagnostic changes.** See "Compile error at handoff" in HANDOFF.md. The fix is to remove the inherent `pub fn diagnostic(&self) -> &DiagnosticCounters` method on `AbsorptionReversalTemplate` (around line 100). The trait method supersedes it. After fix: cargo test -p forge-anomaly should report 55/55 passing.

2. **Commit the diagnostic-mode changes and push to GitHub.**

3. **Run `--engine causal --diagnostic` on the stitched 18-day dataset on Hetzner.** This will show step-by-step completion counts so we can see where the chain is breaking.

4. **Report the diagnostic results to the user.** Based on where it breaks, choose from these fixes (user's ranking):
   - Option 1 (recommended): Relax Step 3 to allow cvd_delta near zero (not just same-sign fade)
   - Option 2: Loosen thresholds (cvd_pressure 0.6→0.45, abs 0.3→0.22, decel 0.5→0.65)
   - Option 3: Revert absorption to >= strict (with small tolerance)
   - Option 4: Reconsider the template itself

5. **Do NOT loosen thresholds or change logic until you have the diagnostic output and the user has chosen a fix.**

## Rules

- One philosophy/template at a time
- Practical, not over-engineered
- Legacy mode must remain default + bit-identical
- Never run locally — always Hetzner

## Test command

```
ssh root@167.233.57.140 "cd /root/forgeOS && /root/.cargo/bin/cargo build --release -p forge-anomaly && /root/forgeOS/target/release/validate --input /root/depthscope_out/stitched_vb10.csv --engine causal --diagnostic --output /tmp/causal_diag_stitched.txt 2>&1"
```

Begin by reading HANDOFF.md in full, then fixing the compile error.
```
