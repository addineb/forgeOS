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
## RESEARCH-ON-FILES correction (fee is NOT the wall here; direction is)

Made the fee configurable (--fee, default 6 = taker-in + maker-out for a target exit)
and dumped every detection (--dump). Analysed 661 ETH LVN detections (61 train days,
1h profile) directly from the file.

KEY FINDING (corrects the earlier "fee wall" framing):
- The MOVES ARE BIG: mean favourable excursion toward value = 70bps, mean adverse
  (away) = 76bps. A 6-9bps fee is trivial against a ~70bps move - the trader is right
  that at 5/15/1h structure scale the fee is easily covered.
- The real problem is DIRECTION: a raw LVN touch is a ~COIN FLIP. Favourable 70 vs
  adverse 76 (ratio 0.92); the bigger excursion was toward value 327 times vs away 334
  = 49/51. Across every target/stop combo (T 30-60 x S 15-30) approx gross ~ -1..+0.2bps
  because clean-losses outnumber clean-wins ~2:1. Thin nodes get TRAVERSED about as
  often as they reject (that is WHY they are thin) - no fee or stop tuning fixes a coin
  flip.

IMPLICATION (validates the trader's method-1): the LVN/level is only the TRIGGER (a
coin flip alone). The EDGE is the ORDERFLOW CONFIRM he applies by hand - is aggressive
flow ABSORBED/rejected at the node (price stalls on heavy volume -> revert to value) or
does it PUSH THROUGH (high impact on thin liquidity -> continue). That confirm has NOT
been mechanised at the LVN yet. Correction logged: prior "doesn't cover fees" verdicts
over-attributed to fees; at this structure scale the move covers the fee and the binding
constraint is the directional confirm.

NEXT: build the absorption/impact orderflow confirm AT the LVN and re-test (location +
confirm), 61d train + 36d OOS, fee 6. Honest caveat: depth-imbalance confirms have been
weak in past studies, but those were at arbitrary edges; this tests it at a real
structural level = the trader's actual method.