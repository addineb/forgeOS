---
tags: [research, latency, lagshot, infra]
type: research
---
# LAGSHOT latency research (2026-06-11): how to get faster to Hyperliquid

Lagshot is TAKER + latency-sensitive. We validated at 884ms (AWS Tokyo round-trip).
This doc = how to hit/beat that, the priority-fee economics, and the venue-switch question.

## The latency facts (sourced)
- HL validators sit in TOKYO (AWS, AZ1/AZ2/AZ4). Network Tokyo->HL = 2-5ms ONLY.
- Public-API round-trip: ~884ms from AWS Tokyo, ~1079ms from Ashburn VA, ~900-1100ms our
  Germany box. The ~880ms is CONSENSUS/server-side + API-server hops, NOT network.
- HyperBFT block finality ~0.2s median, <1s p99. The 0.2s "colocated" number = physical
  colocation with validators (not retail-accessible). 884ms = realistic best for normal API.
- Content rephrased for licensing compliance.

## #1 MOVE (cheap, decisive): put the box in AWS TOKYO (ap-northeast-1)
Our Germany box ~1080ms (edge degrades). A Tokyo box ~884ms = EXACTLY the latency we
already VALIDATED (ETH thr16 OOS t13.31). So Tokyo box alone lands us at the strong,
proven regime. This is THE prerequisite before any live test. ~5ms network to HL.

## DON'T switch to a "faster" venue - the lag IS the edge (key insight)
Lagshot profits BECAUSE HL lags spot price discovery (minutes-scale reversion). A faster /
more-efficient venue (Lighter zk-L2, edgeX) lags LESS -> smaller, faster gaps -> LESS edge,
and what gap exists reverts faster than we can catch. Switching to a "better" venue is
COUNTERPRODUCTIVE. HL is well-suited precisely because it lags AND is liquid + executable.
(Breadth idea, LATER: the SAME perp-lags-spot edge may exist on OTHER laggy perp venues -
CHD has lighter + aster_futures data. A newer/less-efficient venue might lag MORE = fatter
edge. Worth testing as added venues, each with its own data + full validation. NOT a switch.)

## To go BELOW 884ms (UPSIDE, not required for ETH) - the HL official recipe
HL "optimizing latency" doc + priority-fees doc:
1. Run own NON-VALIDATING NODE in Tokyo vs Hyper Foundation peer (fewer hops than API).
2. Build the order book LOCALLY from node outputs (faster + more granular than API);
   official example = github hyperliquid-dex/order_book_server. Run --disable-output-file-buffering.
3. Node specs: >=32 logical cores, 128 GB RAM, 500 MB/s disk (more cores = faster block exec).
4. GOSSIP (read) priority: split_client_blocks:true streams txs pre-inclusion = 70-150ms
   faster READS (signal arrives sooner). Auction slots ~25ms each. Paid in HYPE (burned).
5. ORDER (write) priority: up to 8bps max; EMPIRICAL ~45ms faster end-to-end PER 1bp paid.
   IOC perp orders only. Paid from staking balance, burned.
Stacked (Tokyo node + local book + split_client_blocks + a little order priority), retail can
plausibly push ~884ms -> ~400-600ms. Our latency ladder: 500ms t=3.31, 300ms t=4.95 -> getting
under 500ms makes EVERY config much stronger and re-opens the lower thresholds.

## PRIORITY-FEE ECONOMICS for Lagshot (testable in forgelag)
- Order priority: 45ms faster per 1bp paid. Our gaps 16-19bps, NET edge ~5-8bps/trade.
  => 1-2bp priority (45-90ms faster) MAY be worth it; 8bp would EAT the edge. Sweet spot small.
- We can MODEL this directly: add a priority knob to forgelag (cost +X bps to fees, latency
  -45ms*X) and sweep - find the optimal priority spend BEFORE paying a cent live.

## RECOMMENDED PATH (in order)
1. Rent a Tokyo box (ap-northeast-1), by the hour for the test. -> 884ms = validated regime.
2. Tiny funded HL order from Tokyo: MEASURE real signal->fill latency distribution (not assume).
3. Model priority-fee tradeoff in forgelag; decide if 1-2bp is worth it.
4. ONLY if we want sub-600ms upside: stand up a 32c/128GB Tokyo node + local order book.
5. Breadth (later): test Lagshot-edge on Aster/Lighter perp vs spot (more venues, more capacity).

## Honest note
At 884ms (Tokyo box, no node, no priority) ETH Lagshot is ALREADY strong (validated t9-13).
The node/priority stack is UPSIDE (more return, BTC stronger, lower thresholds viable), NOT a
prerequisite. The real unknown remains the LIVE latency distribution + real fills - measure first.
## FINAL LATENCY RESEARCH (2026-06-12) - can we fix the execution latency? NO.
Asked after the decisive live verdict: is there ANY way to cut Lagshot's trigger->fill
latency enough to capture the edge. Researched HL-specific tricks first, then general HFT.

### The hinge fact
884ms Tokyo round-trip = ~5ms NETWORK + ~879ms SERVER-SIDE (HyperBFT consensus + matching).
The 879ms is the CHAIN's own block processing - shared by everyone, not buyable with hardware.

### Hyperliquid-only levers (official docs: optimizing-latency + priority-fees)
1. Own non-validating node + build book locally from node outputs = biggest win; kills the
   ~100-200ms public-API/WS read lag (see dislocation sooner). Needs 32c/128GB/500MB/s box.
2. --disable-output-file-buffering = block outputs the instant they execute.
3. split_client_blocks:true (gossip/read priority) = 70-150ms faster reads.
4. Gossip (read) priority auction = ~25ms per slot, HYPE burned.
5. Order (write) priority = ~45ms per 1bp, max 8bp (~360ms), HYPE burned.

### General HFT toolkit (colocation, kernel-bypass DPDK/RDMA, FPGA NIC, fiber) = USELESS here
That whole industry shaves NETWORK + OS latency (micro/nanoseconds). From AWS Tokyo our network
to HL is ALREADY ~5ms - nothing left to cut on that side. Kernel bypass = 20-30% off internal
tick-to-trade (microseconds), irrelevant vs an ~879ms consensus we do not control.

### Why it still fails even fully stacked
- BEST realistic calm median maybe ~500-600ms (own node + split_client_blocks + gossip + order pri).
- WRITE-PRIORITY ECON: 45ms/1bp, fee BURNED. Net edge ~5-8bps/trade. Buy 360ms = pay 8bp = burn the
  whole edge. Even 1-2bp eats half. Net-negative (confirmed earlier).
- TRIGGER-MOMENT KILLER: live triggers fired at 1.3-2.4s (not calm 766ms) - latency blows out in
  volatility = exactly when signals fire. Shaving ~300ms off 1.5-2.4s leaves ~1.1-2.1s; reversion
  closes in <1s -> still chasing a vanished gap.

### Verdict
The reversion half-life ~= HL's consensus floor. Everyone executing ON HL hits the same ~879ms wall
(incl us with a full node + priority fees). Captured by resting makers (tested = adverse selection,
loses) and validator-adjacent priority players. Retail taker is structurally below the floor; no
spend justifiable on EUR500 crosses it. LATENCY IS A STRUCTURAL MOAT, not a bug. Lagshot CLOSED:
real edge, not capturable by us. No further latency work warranted.