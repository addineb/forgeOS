# Implementation Plan

Ordering follows the honest ladder: the engine cancel path and the null-edge maker gate come FIRST, so if the fill model cannot stay honest once we add cancels/quoting, we find out cheap before building strategy logic. No live code is in scope here (Req 12 is gated behind sim success).

- [ ] 1. Add the cancel/reprice path to the forgelag sandbox engine
  - Introduce a `LagAction` enum (`Market`, `Place{id}`, `Cancel{id}`) or add `Cancel{id}` alongside `LagOrder`; add `id: u64` to `Resting`.
  - Route actions through `pending`: `Place`/`Market` use `order_latency_ns` (or sampled), `Cancel` uses a new `cancel_latency_ns`.
  - `drain_pending`: `Place` rests only if non-marketable (REJECT if marketable on arrival - a maker never crosses); `Cancel` removes the resting order by id (no-op if filled/gone); apply cancel to the remainder on a partially-filled order.
  - Keep the existing taker `Managed` path and all current tests green (Req 11.4).
  - _Requirements: 1.1, 3.3, 3.4, 6.7, 6.8, 11.1, 11.2_

- [ ] 2. Verify the honest fill model with unit tests (the anti-lie tests)
  - [ ] 2.1 Through-trade fill, queue-ahead protection, partial fill
    - Resting bid + Ask-aggressor trade through it after queue cleared -> fills at resting price (never mid, never improved); trade qty <= queue_ahead -> NO fill + queue decremented; residual < qty_remaining -> partial fill, remainder rests with queue_ahead 0.
    - _Requirements: 6.1, 6.2, 6.3, 6.4_
  - [ ] 2.2 Clean-reversion-no-fill (the killer case) and cancel-latency window
    - Gap stretches then reverts with NO HL trade printing through the resting price -> order stays unfilled.
    - Pulled order still fills from an in-window through-trade; a post-window trade does not.
    - _Requirements: 6.5, 6.6, 6.7, 6.8_

- [ ] 3. Build the maker null-edge gate (must pass before any strategy work is trusted)
  - Add a random-side maker control (rest random side at the offset, same cadence) and a `tests/null_edge_maker.rs`.
  - Assert net P&L strictly < 0 per stream (synthetic + one real day) over >= configured min round trips, after maker+taker fees and honest fills; fail + block promotion if it nets >= 0; seeded -> identical across runs.
  - _Requirements: 7.1, 7.2, 7.3, 7.4, 7.5_

- [ ] 4. Implement the FairValueOracle
  - Reuse `BasisSignal`'s gap/baseline sampling; expose `fair_value()` (= ref_px adjusted by rolling baseline; None until first ref), `dev_bps()`, and `is_stale(now)` (default 1000ms).
  - Update only from data with ts <= now (no lookahead).
  - _Requirements: 2.1, 2.2, 2.4, 2.5, 2.6, 9.3_

- [ ] 5. Implement the InventoryController
  - Track signed position from `ctx.position_qty`; suppress a quote if it would push |position| strictly above `pos_cap`; suppress all quotes if `pos_cap` < one quote size; skew the reducing side closer to mid (offset monotonic in |position|); symmetric at flat.
  - _Requirements: 4.1, 4.2, 4.3, 4.4, 4.6, 4.7_

- [ ] 6. Implement the QuoteManager
  - Compute target = fair value offset by `quote_offset_bps` on the reversion side + inventory skew; want a quote only when |dev| >= entry_threshold.
  - Keep within `reprice_tol` (no duplicate quote); reprice (cancel+place) when fv drifts beyond tol; pull when basis widens beyond `danger_bps`; cancel the resting quote when |dev| falls within exit band or flips side.
  - Emergency flatten (Market) if a pull issued in danger fails to land before fill (ack-timeout) and position is non-zero.
  - _Requirements: 1.2, 1.3, 1.4, 2.3, 3.1, 3.2, 3.3, 3.5, 3.7_

- [ ] 7. Assemble the MakerQuoter strategy (exchange-truth state machine)
  - Implement `LagStrategy`; act on `ctx.position_qty` (not assumed fills). FLAT -> run QuoteManager entry logic; IN POSITION -> cancel stray entry quote, taker-exit on revert-to-mean (Req 5.2) or emergency flatten on danger; stale/no-fv -> pull quotes, keep managing any open position.
  - Wire maker fee on entry fills, taker fee on market exits.
  - _Requirements: 1.5, 3.7, 5.1, 5.2_

- [ ] 8. Config, validation, and fee economics
  - Add maker knobs (quote_offset_bps, entry_threshold, reprice_tol_bps, danger_bps, cancel_latency_ns, ack_timeout_ns, pos_cap, inv_skew_bps, staleness_ns) to the config; reject out-of-range values at construction naming the bad param; fail-fast on missing maker fee at first maker fill; treat rebate as credit / fee as debit.
  - _Requirements: 1.5, 3.6, 5.3, 5.4, 5.5, 5.6_

- [ ] 9. Maker metrics in LagReport + Metrics_Reporter
  - Add `quotes_submitted`, `quotes_filled`, `max_abs_inventory`; track max abs inventory each step and the realized-equity drawdown.
  - Reporter: net expectancy % (mean of trip_returns), fill rate % (filled/submitted, 0 if none), max inventory as % of EUR500 equity, max drawdown %, all rounded to 2dp, against the EUR500 account; net expectancy 0% when zero round trips.
  - _Requirements: 4.5, 10.1, 10.2, 10.3, 10.4, 10.5, 10.6, 10.7, 10.8_

- [ ] 10. Determinism + no-lookahead verification
  - Replay one real day twice -> byte-identical LagReport + identical determinism hash; assert non-monotonic ts halts; assert latency delivery at submit+latency and sampled latency >= 0.
  - _Requirements: 9.1, 9.2, 9.4, 9.5, 9.6, 9.7, 6.9_

- [ ] 11. Hunt CLI wiring + knob-bite reporting
  - Add `--maker`, `--quote-offset`, `--reprice-tol`, `--danger`, `--cancel-lat`, `--pos-cap`, `--inv-skew`, `--staleness` to `bin/hunt.rs`.
  - Record the signal/quote sequence per sweep point; mark a "no edge" verdict valid only if quotes/fills changed between adjacent points; report trade count + fill rate per point; report "cannot establish" for < 2 sweep points.
  - _Requirements: 8.1, 8.2, 8.3, 8.4, 8.5, 8.6_

- [ ] 12. First research pass (coarse sweep) on one real day
  - clippy + null-edge maker gate + fill-model tests green, then sweep quote_offset x entry_threshold x reprice_tol x danger on one real ETH day; look for any net-positive-after-fees region with a believable fill rate; record results in docs/research.
  - If a pulse appears: fine sweep + multi-day + OOS + shuffle control + DSR/PBO (later tasks/spec); if not, record the honest negative.
  - _Requirements: 7.1, 8.1, 10.1, 10.2_

## Task Dependency Graph
- Task 1 -> 2, 3, 6 (cancel path underpins fills, gate, and quoting)
- Task 2, 3 gate everything (honest fill proven before strategy is trusted)
- Tasks 4, 5 -> 6 -> 7 (oracle + inventory feed the quote manager feed the strategy)
- Task 8 supports 6, 7; Task 9 supports 12; Task 10 independent after 1; Task 11 -> 12
- Task 12 is the payoff and depends on 1-11 (at least 1-9, 11) being green