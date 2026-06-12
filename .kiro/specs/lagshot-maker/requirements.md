# Requirements Document

## Introduction

Lagshot proved a real cross-venue basis/lag edge: Hyperliquid's perp microprice lags OKX spot, the gap stretches past its baseline, then snaps back. As a TAKER the edge was validated through every gate (null-edge, shuffle x2, out-of-sample x2) and backtested huge, but a live test with real money proved it is NOT capturable: order-to-fill latency to Hyperliquid is ~766ms calm and 1.3-2.4s in the volatile moments signals fire, dominated by HyperBFT consensus we cannot buy past. The reversion closes faster than a taker can fill, so we pay 15-22bps chasing a gap that is already gone.

This feature is the PIVOT: capture the SAME edge as a MAKER (resting limit orders) instead of a taker. A maker does not race to take; it rests and gets crossed to, so entry latency stops mattering and fees are lower. The maker's enemy is ADVERSE SELECTION: resting orders fill preferentially when we are wrong. A naive maker version was already tested and lost hard (t = -8 to -11, win ~10%) because resting orders only filled on continuation and the reversions often snapped back without trading through our price. So this feature must solve the real market-making problem: WHERE to place quotes and WHEN to pull or reprice them.

The single most important deliverable is an HONEST maker fill model. The prior project (wall-bot-tournament) lied: idealized fills made a losing strategy look like a 100% winner. ForgeOS exists to never repeat that. Therefore the fill model must never manufacture optimistic fills, must model queue position and adverse selection, must never fill at mid, and must bias fills toward wrong-way (toxic) flow. No live order may be placed until the honest simulation passes the null-edge gate and all validation gates.

All work lives in the `forgelag` crate (`crates/forgelag/`), a sandbox engine branch that reuses proven primitives. The SACRED engine core (`forge-core`, `forge-data`, `forge-book`, `engine.rs`, `account.rs`, `fills.rs`) MUST NOT be modified without explicit sign-off.

## Glossary

- **Maker_Strategy**: The new resting-limit-order strategy under test in `forgelag`; quotes around a fair value to capture basis reversion without crossing the spread.
- **Fair_Value_Oracle**: The component that computes where Hyperliquid is expected to trade next, from the OKX spot reference price plus the running basis.
- **Quote_Manager**: The component that decides when resting quotes stay live, when they are pulled, and when they are repriced.
- **Inventory_Controller**: The component that tracks accumulated signed position and enforces the position cap and inventory skew.
- **Fill_Model**: The honest maker fill model inside the `forgelag` engine: queue-position plus adverse-selection, no mid-price fills, fills biased toward wrong-way flow.
- **Validation_Harness**: The test and sweep tooling (`tests/`, `bin/hunt.rs`) that runs the gates and reports metrics.
- **Metrics_Reporter**: The component that produces the per-run report.
- **HL**: Hyperliquid perpetual, the execution venue.
- **OKX**: OKX spot, the reference / fair-value anchor venue (validated as the best anchor).
- **Basis**: The signed difference between the HL microprice and the OKX reference price, expressed in basis points (bps).
- **Dev**: The deviation of the current basis from its rolling baseline, in bps; the dislocation signal.
- **Microprice**: The size-weighted top-N HL book price used as HL's instantaneous value.
- **Adverse_Selection**: The tendency of resting orders to fill when the market is about to move against the resting side.
- **Toxic_Flow**: Trade flow that crosses a resting quote because price is continuing through it (the wrong-way case for a maker).
- **Cancel_Latency**: The time between the strategy deciding to pull or reprice a quote and that cancel reaching the HL matcher.
- **Null_Edge_Gate**: The mandatory test that a seeded coinflip strategy must net negative (~ fees plus costs) over large N; if a coinflip profits, the engine is lying.
- **Knob_Bite_Rule**: A "no edge" verdict is valid only if the swept parameter actually changed trade behavior.
- **Round_Trip**: One completed entry-then-exit cycle.
- **Net_Expectancy**: Average per-round-trip profit-and-loss net of all fees, expressed as a percent of entry notional.
- **Account_Model**: EUR 500 starting equity, leverage and sizing per project norms (20x leverage, 20% sizing unless a requirement overrides).

## Requirements

### Requirement 1: Maker quoting behavior

**User Story:** As a microstructure trader, I want the strategy to capture the basis reversion by RESTING limit orders rather than crossing the spread, so that my entry no longer races the reversion and I stop paying taker slippage on a gap that has already closed.

#### Acceptance Criteria

1. THE Maker_Strategy SHALL place all entry orders as resting limit (maker) orders on HL, and the count of market (taker) orders submitted for entry SHALL be zero.
2. WHEN the absolute deviation (Dev) between the HL microprice and the OKX-spot fair value exceeds the configured entry threshold (accepted in bps, configurable within the range 1 to 100 bps), THE Maker_Strategy SHALL place exactly one resting limit quote on the reversion side of fair value, placing a resting bid when HL is below fair value and a resting ask when HL is above fair value, offset from fair value by the configured quote offset.
3. WHILE a resting entry quote is unfilled and the Dev still exceeds the entry threshold on the same reversion side, THE Maker_Strategy SHALL keep the existing quote live at its current price and SHALL NOT submit a duplicate quote at that price.
4. IF a resting entry quote is unfilled and the Dev returns to within the configured exit threshold or flips to the opposite reversion side, THEN THE Maker_Strategy SHALL cancel the resting quote and return to the no-position state with no entry recorded.
5. WHEN a resting entry quote fills, fully or partially, THE Maker_Strategy SHALL transition to the position-management state for the filled quantity and apply the configured exit rule, closing the position when the absolute Dev returns to within the configured exit threshold.
6. THE Maker_Strategy SHALL expose its entry threshold (in bps), quote offset (in bps), and reversion side as configurable parameters through the existing BasisConfig and hunt CLI.

### Requirement 2: Fair-value oracle from OKX plus basis

**User Story:** As a trader, I want to quote around where HL is ABOUT to be, derived from OKX spot plus the basis, so that I rest on the side the price is reverting toward instead of guessing from HL alone.

#### Acceptance Criteria

1. THE Fair_Value_Oracle SHALL define the fair value for HL as the latest valid OKX reference price plus the running basis baseline, where the basis baseline is the mean of the HL-minus-OKX price difference computed over a configurable rolling window (default 500 samples).
2. WHEN a new OKX reference trade is observed, THE Fair_Value_Oracle SHALL recompute the fair value using only OKX reference trades whose timestamp is at or before the current virtual clock.
3. THE Fair_Value_Oracle SHALL place the resting quote at a configurable offset from fair value, bounded between 0 and 100 bps, on the reversion side, defined as the side opposite the sign of the current deviation (HL price minus fair value).
4. IF no OKX reference price has been observed yet, THEN THE Maker_Strategy SHALL place no quotes.
5. IF the latest OKX reference price is older than a configurable staleness limit (default 1000 ms measured on the virtual clock), THEN THE Maker_Strategy SHALL place no quotes and SHALL cancel any resting quotes, retaining the last computed fair value without updating it.
6. WHERE the reference anchor venue is configurable, THE Fair_Value_Oracle SHALL default to OKX spot.

### Requirement 3: Quote management - pull and reprice (the WHEN)

**User Story:** As a trader, I want quotes to stay live while fair value is stable but pull fast when the basis WIDENS into a real move, so that I am not run over by a structural move that my resting order would fill into on the wrong side.

#### Acceptance Criteria

1. WHILE the fair value remains within the configured reprice tolerance (configurable in basis points, valid range 0.1 to 100.0 bps) of the resting quote price, THE Quote_Manager SHALL keep the resting quote live at its current price and issue neither a reprice nor a pull.
2. WHEN the fair value moves beyond the configured reprice tolerance, THE Quote_Manager SHALL, within one decision cycle, reprice the resting quote so that its new price is within the configured reprice tolerance of the updated fair value.
3. IF the Basis widens beyond the configured danger threshold (configurable in basis points, valid range 1.0 to 500.0 bps) in the direction that would adversely fill the resting quote, THEN THE Quote_Manager SHALL issue a pull of the resting quote.
4. WHEN the Quote_Manager issues a pull or reprice, THE forgelag engine SHALL delay the cancel reaching the matcher by the configured Cancel_Latency (configurable in milliseconds, valid range 0 to 5000 ms) measured from the decision timestamp.
5. WHILE a pull or reprice is in flight within the Cancel_Latency window, THE Fill_Model SHALL fill the still-resting quote at its resting price from any HL trade that trades through that price.
6. THE Quote_Manager SHALL expose the reprice tolerance, the widen danger threshold, and the Cancel_Latency as configurable parameters, and IF a configured value falls outside its stated valid range, THEN THE Quote_Manager SHALL reject the configuration and indicate the invalid parameter.
7. IF a pull or reprice fails to execute (no cancel acknowledgement received within the Cancel_Latency window plus a configured acknowledgement timeout, valid range 0 to 5000 ms) while the Basis remains beyond the danger threshold, THEN THE Quote_Manager SHALL trigger an emergency flatten of any resulting position and indicate the execution failure rather than leave the quote exposed.

### Requirement 4: Inventory control and position cap

**User Story:** As a trader, I want a hard cap on accumulated inventory and a skew that leans against my current position, so that a run of one-sided fills cannot blow past my risk budget on EUR 500.

#### Acceptance Criteria

1. WHEN a maker order fills, THE Inventory_Controller SHALL update the signed accumulated position by the filled quantity, where a buy fill increases the position and a sell fill decreases it, measured in raw base quantity.
2. IF placing a new quote would cause the resulting absolute position to be strictly greater than the configured position cap, THEN THE Inventory_Controller SHALL suppress that quote and SHALL leave the current position unchanged.
3. WHILE holding non-zero inventory, THE Inventory_Controller SHALL place the position-reducing quote closer to mid than the position-increasing quote, increasing the skew offset monotonically with the absolute position.
4. WHILE inventory is zero, THE Inventory_Controller SHALL quote symmetrically with no inventory skew.
5. THE Inventory_Controller SHALL expose the position cap (valid range: at least 0 raw quantity) and the inventory skew (valid range: at least 0) as configurable parameters.
6. THE Metrics_Reporter SHALL report the maximum absolute inventory reached during a run, in raw base quantity.
7. IF the configured position cap is smaller than one quote size, THEN THE Inventory_Controller SHALL suppress all quotes for the entire run.

### Requirement 5: Maker fee and rebate economics

**User Story:** As a trader, I want the maker fee or rebate modeled explicitly so the profitability bar reflects the lower maker cost, so that I can quantify how much easier the maker bar is than the taker bar I failed at.

#### Acceptance Criteria

1. WHEN a maker order fills, THE Fill_Model SHALL apply the maker fee or rebate defined in the active fee schedule to that fill and SHALL NOT apply the taker fee to that fill.
2. WHEN a position is exited with a market order, THE Fill_Model SHALL apply the taker fee defined in the active fee schedule to that exit fill.
3. THE Metrics_Reporter SHALL report Net_Expectancy as the average per-trade net profit and loss after deducting all applied taker and maker fees and crediting all applied maker rebates.
4. THE Maker_Strategy SHALL expose the maker fee or rebate as a configurable parameter through the fee schedule.
5. THE Fill_Model SHALL treat a maker rebate as a credit that increases the net proceeds of the fill and a maker fee as a debit that decreases the net proceeds of the fill.
6. IF the maker fee or rebate is not defined in the active fee schedule when a maker order fills, THEN THE Fill_Model SHALL halt the simulation and emit an error indicating the missing maker fee parameter, without producing fill economics for that fill.

### Requirement 6: Honest maker fill model (the prime gate)

**User Story:** As a trader burned by a backtest that lied, I want a maker fill model that never manufactures optimistic fills, so that any edge it reports survives the adverse selection that killed the naive maker version live.

#### Acceptance Criteria

1. THE Fill_Model SHALL fill a resting maker order only when an HL trade prints through the resting price, where "prints through" means the trade price is equal to or beyond the resting limit price in the direction that executes the order, and only after the queue ahead of the resting order has been cleared.
2. THE Fill_Model SHALL never fill a maker order at the mid price and SHALL never fill a maker order at a price better than its resting limit price.
3. WHEN an HL trade prints through the resting price, THE Fill_Model SHALL consume the trade volume against the queue ahead of the resting order first (FIFO), and only residual volume beyond the queue-ahead SHALL fill the resting order.
4. WHEN trade volume through the resting price exceeds the queue ahead but is less than the queue-ahead plus the resting order size, THE Fill_Model SHALL partially fill the resting order by the residual volume only, leaving the remainder resting with zero queue ahead.
5. THE Fill_Model SHALL fill resting quotes in proportion to cumulative HL trade volume that continues through the quote (Toxic_Flow), reflecting Adverse_Selection.
6. IF a reversion returns the price without any HL trade printing through the resting price, THEN THE Fill_Model SHALL leave that resting order unfilled.
7. WHILE a pull or reprice is within the configured Cancel_Latency window (configurable, valid range 0 to 5000 ms), THE Fill_Model SHALL keep the resting order fillable at its resting price.
8. WHEN the Cancel_Latency window has elapsed for a pulled order, THE Fill_Model SHALL remove the order and SHALL NOT fill it thereafter.
9. THE Fill_Model SHALL produce a byte-for-byte identical sequence of fills (quantity, price, ordering, and timestamps) for identical input event streams and identical configuration.
### Requirement 7: Null-edge gate

**User Story:** As a trader, I want a seeded coinflip maker to still lose after fees and realistic adverse-selection fills, so that I know the maker engine is not inventing edge before I trust any positive result.

#### Acceptance Criteria

1. WHEN a seeded coinflip Maker_Strategy is run over both the synthetic and the real event streams, THE Validation_Harness SHALL compute each stream's net result as total P&L after fees and adverse-selection fills, and SHALL report each stream's net result as strictly less than zero.
2. THE coinflip Maker_Strategy SHALL execute at least a configurable minimum of 1,000 completed round trips per event stream (a round trip being a maker entry fill paired with its closing exit fill) before the Null_Edge_Gate evaluates a pass/fail verdict.
3. IF the coinflip Maker_Strategy net result on any event stream is greater than or equal to zero, THEN THE Validation_Harness SHALL fail the Null_Edge_Gate, SHALL block promotion of any maker result, and SHALL emit an error indicating which event stream produced the non-negative net result.
4. WHEN the forgelag test suite executes, THE Null_Edge_Gate SHALL run as an automated test, and IF its pass condition (every event stream nets strictly less than zero over the minimum round trips) is not met, THEN THE Null_Edge_Gate SHALL fail the test suite run.
5. WHEN the Null_Edge_Gate is run more than once with the same seed and the same event streams, THE Validation_Harness SHALL produce identical net results across runs.

### Requirement 8: Knob-bite verdict rule

**User Story:** As a trader, I want a "no edge" verdict to count only when the swept setting actually changed trades, so that I never mistake an inert dial for a tested one.

#### Acceptance Criteria

1. WHEN a parameter is swept across maker runs, THE Validation_Harness SHALL determine whether trade behavior changed between each adjacent pair of sweep points, where a change means the ordered sequence of generated signals and submitted quotes differs in count, side, price, or timestamp.
2. THE Validation_Harness SHALL perform the knob-bite comparison for each adjacent pair of sweep points rather than across the sweep as a whole.
3. IF a swept parameter did not change trade behavior between adjacent sweep points, THEN THE Validation_Harness SHALL mark the corresponding "no edge" verdict as invalid.
4. WHEN a swept parameter changes trade behavior between adjacent sweep points, THE Validation_Harness SHALL treat the resulting "no edge" verdict as valid.
5. IF a sweep contains fewer than two sweep points, THEN THE Validation_Harness SHALL report that knob bite cannot be established.
6. THE Metrics_Reporter SHALL report the trade count and fill rate (in percent) at each sweep point so knob bite is verifiable.

### Requirement 9: Determinism and no-lookahead

**User Story:** As a trader, I want the maker simulation to be deterministic and free of lookahead by construction, so that a reported number is reproducible and not a peek at the future.

#### Acceptance Criteria

1. THE forgelag engine SHALL process events in non-decreasing local-timestamp (virtual-clock) order, advancing the virtual clock to each processed event's local timestamp.
2. IF an event presents a local timestamp earlier than the most recently processed event's local timestamp, THEN THE forgelag engine SHALL halt the simulation, emit an error indicating the non-monotonic timestamp, and produce no report.
3. THE Maker_Strategy SHALL base every quoting, pull, and reprice decision solely on data whose local timestamp is less than or equal to the current virtual-clock value.
4. IF a Maker_Strategy decision attempts to read an event whose local timestamp is greater than the current virtual-clock value, THEN THE forgelag engine SHALL halt the simulation, emit an error indicating the lookahead violation, and produce no report.
5. WHEN the same event stream and identical configuration are replayed any number of times on the same engine version, THE forgelag engine SHALL produce a byte-for-byte identical report and an identical determinism hash across all runs.
6. WHEN order or cancel latency is applied, THE forgelag engine SHALL deliver the order or cancel to the matcher at a virtual-clock time equal to the submission local timestamp plus the sampled latency, and SHALL NOT make that order or cancel eligible for matching before that delivery time.
7. THE forgelag engine SHALL constrain every sampled order or cancel latency to be greater than or equal to zero.

### Requirement 10: Metrics reporting

**User Story:** As a trader, I want every maker run to report net-of-fees expectancy, fill rate, inventory risk, and drawdown in percent, so that I can judge viability the same way across runs.

#### Acceptance Criteria

1. WHEN a maker run completes, THE Metrics_Reporter SHALL report Net_Expectancy per round trip net of all trading fees, expressed in percent rounded to two decimal places, where a round trip is one completed entry-and-matching-exit position pair.
2. WHEN a maker run completes, THE Metrics_Reporter SHALL report the fill rate as the count of filled quotes divided by the count of submitted quotes, expressed in percent rounded to two decimal places and bounded between 0 and 100 percent.
3. IF no quotes were submitted during a run, THEN THE Metrics_Reporter SHALL report a fill rate of 0 percent.
4. WHEN a maker run completes, THE Metrics_Reporter SHALL report inventory risk as the maximum absolute inventory reached during the run, expressed in percent of the EUR 500 starting equity rounded to two decimal places.
5. WHEN a maker run completes, THE Metrics_Reporter SHALL report maximum drawdown as the largest peak-to-trough decline in account equity over the run, expressed in percent of the preceding peak equity rounded to two decimal places.
6. THE Metrics_Reporter SHALL compute all reported metrics against the Account_Model initialized with EUR 500 starting equity.
7. THE Metrics_Reporter SHALL express all performance metrics in percent rather than basis points in the run report.
8. IF a run completes with zero completed round trips, THEN THE Metrics_Reporter SHALL report a Net_Expectancy of 0 percent.

### Requirement 11: Engine-core protection and crate boundary

**User Story:** As a trader, I want the maker work confined to the forgelag sandbox and the verified-honest core left untouched, so that the sacred engine stays trustworthy.

#### Acceptance Criteria

1. THE Maker_Strategy and Fill_Model code SHALL reside entirely within the forgelag crate, with zero such code residing in any other crate.
2. IF a change would modify forge-core, forge-data, forge-book, engine.rs, account.rs, or fills.rs, THEN a recorded written sign-off SHALL be obtained before the change is committed, and absent that sign-off those files SHALL remain byte-for-byte unchanged.
3. WHERE proven fill and account primitives already exist in forge-sim, THE Fill_Model SHALL invoke those primitives rather than implement a duplicate.
4. WHEN any change touches fill or profit-and-loss logic, THE Validation_Harness SHALL include a test that passes before the change and fails on a regression (positive coinflip edge, lookahead, or accounting error), blocking merge.
5. WHEN the forgelag build runs, THE Null_Edge_Gate (a seeded coinflip netting negative) SHALL be bound to the build as a required check.
6. WHEN a commit is prepared, THE Validation_Harness SHALL verify via diff that the SACRED core files are unchanged absent a recorded sign-off.

### Requirement 12: Live-order safety gate

**User Story:** As a trader with EUR 500 at stake, I want no live order placed until the honest simulation passes every gate, and then only tiny size to measure real cancel latency, so that I never risk real money on an unproven maker.

#### Acceptance Criteria

1. IF the honest maker simulation has not passed the Null_Edge_Gate and all validation gates (knob-bite, DSR, PBO, shuffle-control, out-of-sample replication), THEN THE system SHALL NOT place any live maker order.
2. WHEN all gates pass, THE first live deployment SHALL use a configured tiny notional cap (default EUR 11) at 1x leverage (no liquidation), whose purpose is to measure real Cancel_Latency on HL.
3. IF cumulative realized loss over the UTC day reaches the configured daily loss limit (default 15 percent of the day's starting equity), THEN THE live deployment SHALL halt by ceasing new orders for the remainder of that UTC day.
4. WHILE the live deployment is in the halted state, THE system SHALL reject new orders and SHALL require an explicit manual reset to resume.
5. WHEN at least 20 round-trip Cancel_Latency samples have been measured live, THE measured distribution SHALL be fed back into the Fill_Model for an honest blended re-run that SHALL pass all gates before any size increase.
6. IF the honest blended re-run fails any gate, THEN the live size SHALL remain at the tiny notional cap.