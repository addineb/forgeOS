# Research Note -- Harnessing the Flow Primitives (2026-06-09)

From the order-flow research session (see `docs/research-charter.md`, Q5/Q2).
Purpose: translate the literature into CONCRETE, flag-gated upgrades to our
existing primitives so the build session can pick them up as discrete tasks.

## Ground rules for anyone implementing these
- These touch signal logic. Per the project hard rules every change is
  ADDITIVE and FLAG-GATED: off-by-default, byte-identical to today when off.
- Validate in `pnpm backtest` over the captured 3-stream window
  (2026-06-06 05:00-12:00 UTC) BEFORE any live touch. Use the DSR/OOS gates.
- Cost wall: ~11 bps round-trip (~0.11% on BTC). Every success bar is stated
  net of that.
- Citations are paraphrased for licensing. They are the "why", not the "what to
  copy".

## The one theme that ties it all together
Most primitives are MECHANICALLY correct but fed RAW, non-stationary inputs, so
their entry thresholds drift with regime and our parameter sweeps partly chase
noise. The highest-leverage upgrade across OFI / MLOFI / CVD / VPIN is the same:
put the signal on the right SCALE before thresholding (depth-normalise the
book-flow signals; volume-clock the trade signals). That alone makes thresholds
stationary, sweeps honest, and edge comparable against the 11 bps wall.

Priority order (do them in this order):
1. OFI depth-normalisation        (biggest, unblocks honest sweeps)
2. Absorption signing + reload confirm
3. CVD volume-clock + pivot-anchored divergence
4. MLOFI integrated (depth-normalised) aggregation
5. Persistence-aware hold-time (shared exit upgrade)
6. VPIN on true signs (only if we keep VPIN as a gate)

---

## 1. OFI -- depth-normalise the signal
File: `artifacts/api-server/src/replay/primitives/ofi.ts`

**Mechanism.** Net change in resting size at the touch: bid added / ask removed
= up-pressure; mirror = down. Our code computes the canonical Cont-Kukanov-
Stoikov quantity `bid_added - bid_removed - ask_added + ask_removed`, rank-based,
windowed, with an executed-vs-cancelled split. The core is faithful.

**Literature.** Cont, Kukanov & Stoikov, "The Price Impact of Order Book Events"
(arXiv:1011.6402): price change ~= beta * OFI with beta INVERSELY PROPORTIONAL
to market depth. The depth scaling is the part we currently ignore.

**Our gap.** We threshold RAW windowed OFI. The same raw OFI implies a big move
in a thin book and a small move in a thick book, so a fixed threshold drifts
with regime and the threshold sweep is partly measuring book thickness.

**Flag-gated change.** Add `ofiNormalization` in {`raw`, `depth`, `predicted-bps`}:
- `raw`     -> today's behaviour (default; byte-identical).
- `depth`   -> divide windowed OFI by a rolling avg top-N resting size over the
               same window -> stationary, regime-comparable signal.
- `predicted-bps` -> estimate beta online and emit the predicted move in bps,
               directly comparable to the 11 bps hurdle.

**Backtest success bar.** With `depth`/`predicted-bps`, the best swept cell must
(a) beat the best `raw` cell on net P&L per trade after 11 bps, AND (b) hold its
sign on OOS (IS 05:00-09:00 / OOS 09:00-12:00). If not, keep raw.

---

## 2. MLOFI -- integrated (depth-normalised) aggregation
File: `artifacts/api-server/src/replay/primitives/mlofi.ts`

**Mechanism.** Per-level OFI vector collapsed to a scalar
(`weighted-sum`/`max-abs`/`top-K`), built on the shared OFI engine.

**Literature.** Cont, Cucuringu & Zhang, "Cross-Impact of OFI" (arXiv:2112.13213):
an INTEGRATED OFI -- each level normalised by its own average depth, then
combined with empirically-fit (PCA/regression) weights -- explains price impact
BETTER than best-level OFI. Xu, Gould & Howison (arXiv:1907.06230) is the MLOFI
vector definition.

**Our gap.** Aggregations skip per-level depth normalisation, so deeper levels
with naturally larger queues can dominate for the wrong reason; weights are
assumed shapes, not fit.

**Flag-gated change.** Add `ofiLevelWeights: "integrated"` that depth-normalises
each level (OFI_i / avg_size_i) BEFORE combining; optionally allow fitted
weights as a later step.

**Backtest success bar.** Integrated MLOFI must beat the depth-normalised SINGLE-
level OFI (item 1) on OOS. If it doesn't, the extra levels are just cost -> fold
back to single-level.

---

## 3. CVD -- volume-clock + pivot-anchored divergence
File: `artifacts/api-server/src/replay/primitives/cvd.ts`

**Mechanism.** Running signed-volume sum on 60s TIME bars; divergence =
price-vs-CVD disagreement z-scored over per-bar deltas; absorption =
|deltaCVD| / price-range. Aggressor sign comes from `isBuyerMaker` -- i.e. our
signs are EXACT (no Lee-Ready/BVC inference error). Keep that; it's an edge.

**Gap A -- wrong clock.** Fixed-time bars carry wildly different information
under variable activity, so the divergence z is non-stationary (the same defect
the volume clock was invented to fix in the VPIN lineage).
**Change A.** Add `cvdBarMode` in {`time`, `volume`, `tradeCount`}; bucket on
cumulative volume/trades so each bar carries comparable information.

**Gap B -- arbitrary anchor.** We compare first-bar-open vs last-bar-close. The
divergence practitioners actually trade is PIVOT-anchored: price higher-high
while CVD lower-high (and mirror).
**Change B.** Detect swing pivots and measure divergence between consecutive
pivots, not window edges.

**Confidence note.** CVD divergence tradeability is practitioner lore, thin in
peer review. These changes clean the signal; it still must prove out in
backtest.

**Backtest success bar.** Volume-clock divergence must produce a stationary z
(check the z distribution across the window) AND beat the time-bar version on
net-per-trade OOS in the reversal pair (item below).

---

## 4. Absorption -- sign it and confirm with reloads
File: `artifacts/api-server/src/replay/primitives/absorption.ts`

**Mechanism.** `volume / max(priceRange, granularity)`, z-scored vs a causal
Welford baseline. This is essentially inverse local impact (high volume, low
move = resilience), tied to the resilience/propagator picture
(square-root impact, arXiv:2506.07711).

**Gap A -- unsigned.** Sellers hitting the bid while the bid holds is BULLISH
absorption; the mirror is bearish. Our output is unsigned, so a bot can't trade
direction off it.
**Change A.** Use the dominant aggressor side (we already compute executed/
cancelled per side in the OFI engine) to SIGN the absorption output.

**Gap B -- low range != absorption.** Quiet chop also yields high volume/range.
Genuine iceberg absorption is aggressive volume EXECUTED into a level that
RELOADS while price fails to break.
**Change B.** Confirm with "level executed-into AND reloaded (bookDelta) AND
price held", separating real icebergs from dead tape.

**Backtest success bar.** Signed+confirmed absorption, entered AGAINST the
exhausted push, must clear 11 bps net on OOS with median reversal MFE >= TP+fees.

---

## 5. Persistence-aware hold-time (shared exit upgrade)
Files: exit logic + a small new autocorrelation estimator.

**Mechanism / literature.** OFI explains the CONTEMPORANEOUS move; its forward
edge decays fast (OFI churns faster than price -- arXiv:1410.1900). How long a
push CONTINUES is governed by order-flow long memory (Lillo-Mike-Farmer,
SSRN 708303) and impact then partially REVERTS (square-root impact + reversion).
So OFI gives direction NOW; persistence sets the hold.

**Our gap.** Fixed `exitTimeMs` ignores the regime's measured persistence, so we
hold winners too short in trending flow and too long into reversion.

**Change.** Estimate rolling trade-sign autocorrelation (or a short Hurst proxy)
and map it to hold length: high persistence -> longer hold; low -> shorter / exit
before reversion. This is the only NET-NEW code (a small estimator); everything
else above is a param on an existing primitive.

**Backtest success bar.** Adaptive hold must beat the BEST fixed `exitTimeMs`
from the OFI sweep on OOS. If it can't beat a constant, the long-memory signal
isn't actionable for us at this horizon -> park it.

---

## 6. VPIN -- build on TRUE signs (only if kept as a gate)
File: `artifacts/api-server/src/replay/primitives/vpin.ts`

**Literature.** The critique that sank VPIN (Andersen & Bondarenko, Review of
Finance 2014) targets BULK VOLUME CLASSIFICATION misattributing buy/sell under
volatility. We have REAL signed trades, so we sidestep the core objection.

**Stance.** VPIN stays a DIRECTIONLESS regime/volatility gate, never an entry.
If we keep it, ensure it is computed from true aggressor signs (not BVC) on a
volume clock.

**Backtest success bar.** As a gate on the OFI winner: improves net-per-trade OR
Sharpe on OOS vs ungated. If neither, drop the gate.

---

## What is explicitly NOT worth building (from Q5)
- Queue-dynamics as a TAKER direction signal: strong but one-tick / sub-second
  (Cont-de Larrard; Gould-Bonart arXiv:1512.03492) -> HFT turf, cost-killed at
  11 bps. Maker variant also weak: maker fills are adversely selected
  (arXiv:2502.18625). Keep `queue-proxy` as a feature input only.
- Hawkes as an ENTRY: heavy to calibrate, edge duplicates OFI-persistence.
  Keep only the branching ratio as a possible regime descriptor.

## Suggested task breakdown for the build session
- T1: OFI `ofiNormalization` {raw,depth,predicted-bps} + sweep vs raw.   [item 1]
- T2: Absorption signing + reload/executed confirmation.                 [item 4]
- T3: CVD `cvdBarMode` {time,volume,tradeCount} + pivot divergence.      [item 3]
- T4: MLOFI `integrated` depth-normalised aggregation.                   [item 2]
- T5: Persistence-adaptive hold-time estimator + exit hook.              [item 5]
- T6 (optional): VPIN true-sign/volume-clock gate.                       [item 6]
Each ships behind a default-off flag, proves byte-identical when off, and
carries its own backtest cell that clears 11 bps net on OOS.