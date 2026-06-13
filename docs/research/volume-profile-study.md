# Volume-profile LVN mean-reversion study (vpscope)

Trader idea: price reaches a LOW-VOLUME NODE (thin price, rejected by the auction)
outside the value area, then MEAN-REVERTS toward value (the POC). Tool = vpscope
(crates/forgelag/src/bin/vpscope.rs), analysis-only, sacred core untouched, rolling
NO-LOOKAHEAD volume profile from HL aggressive trades over a lookback L -> POC /
value-area / LVN; first-touch reversion trade toward the POC (run-overs IN, 9bps
taker). clippy -Dwarnings clean; 6 unit tests green.

## STEP 1 - location ALONE (no exhaustion conditioning)
61d train (Nov-Dec 2025) + 36d OOS (Feb/May/Jun 2026), ETH+BTC. Grid lookback
{1,2,3,4h} x lvn-frac {0.15,0.25}, nbins 50, va 70%, forward 2h, stop 40, min-dist 15.

### VERDICT (bad): NO edge at scale. The raw LVN reversion taker is ~breakeven gross.
- revert-TAKER "all" GROSS ~0 in EVERY cell (-4 .. +1.6bps) -> NET -8 .. -13 after the
  9bps taker fee. Win 47-62% but RR 0.65-1.0 (the reach-to-POC target is often unmet
  while the losers run) -> ~zero expectancy before fees.
- The 2-day smoke that showed +5-6bps NET was SMALL-SAMPLE NOISE - confirmed instantly
  once we went to 61+36 days (exactly the lesson the sweep taught us).

### Mild structure that IS consistent (but below the fee)
- Shorter lookback (1h) reverts MORE (~55-58%, holds in BOTH train and OOS); longer
  (4h) continues more. So the "thin node snaps back" tendency is real but small.
- BTC "far-from-value" LVNs show gross-POSITIVE in BOTH periods (+2..+6bps) - a real
  faint signal - but still under the 9bps fee. ETH thin/far cells SIGN-FLIP train<->OOS
  (e.g. 1h thin train -2.5 vs OOS +4.6) = noise.

### Bottom line
Location alone is NOT tradeable as a taker - same wall as every prior lead: a real but
small (~0-6bps) microstructure tendency that the ~9bps taker fee kills. The reversion
is real (price does snap back ~55% at 1h) but the captured edge is below the toll.

### Next (honest)
Planned step 2 = add the exhaustion/flow timing at the LVN. It is a LONG SHOT: the
location gross is ~0 (no pulse to amplify) AND the exhaustion separator already
collapsed at scale in the sweep study. If tried, gate hard and judge on net + OOS.
Deeper truth reasserted: taker microstructure at 9bps is structurally dead for us; the
only escapes are lower fees / maker (but an LVN is thin = adverse selection) or much
bigger per-trade moves. Logs /root/runs/vpval/.