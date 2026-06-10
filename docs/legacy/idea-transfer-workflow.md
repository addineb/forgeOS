# Idea-Transfer Workflow (discretionary edge -> coded strategy)

Always read this when the trader (10y discretionary, microstructure) wants to
hand off a NEW edge idea. It defines HOW we move an idea out of his head into a
deterministic, null-edge-gated ForgeOS strategy. This is about COMMUNICATION /
elicitation, not a strategy itself.

## The problem we are solving
The edge lives in his visual intuition: he SEES a setup (book/tape behaviour) and
acts. The engine needs an unambiguous rule evaluable on every event with NO
hindsight. The gap between "what I see" and "an explicit rule" is where lookahead
and curve-fit rot creep in (cf. the archived wall-bot). The job of this workflow
is to extract the rule precisely enough that it survives the NULL-EDGE TEST.

## How he transfers an idea (methods, stack them)
1. ANNOTATED CHART IMAGE -- he drags a screenshot into chat with arrows / boxes /
   circles ("price sweeps here, book thins here, I fade into here"). Kiro reads
   the image. Fastest way to convey INTENT. Use for intent only (see caveat).
2. REPLAY-AND-NARRATE (strongest for microstructure) -- he points at EXACT
   moments in the real `.forge` stream ("2025-12-01 14:32:07.412 BTCUSDT, look at
   the book -- that is the setup; entry here; wrong when W"). We pull those exact
   windows. Turns the edge into LABELED real examples (setup / not-setup), and
   captures book+tape detail a candle chart throws away.
3. FEATURE DICTIONARY (shared vocabulary) -- write, in plain words first, the
   primitives he actually watches: imbalance, sweep, absorption, thinning, stop
   run. Each becomes ONE measurable quantity with an agreed numeric definition
   (e.g. "book thins" = top-3 depth drops >X% in <Y ms). Ideas then compose from
   these blocks instead of being re-explained each session.
4. DECISION TABLE -- conditions -> action: entry / invalidation / exit / sizing.
   This surfaces the hidden assumptions ("only in the first hour", etc.).
5. NODE / FLOW DIAGRAM (optional) -- wire condition -> AND -> trigger boxes when a
   setup has many conditions. Usually overkill until the rule is settled.

## Microstructure caveat (do not skip)
The edge lives in the ORDER BOOK + TRADE TAPE, which a candlestick chart cannot
show. Arrows drawn on candles risk encoding a signal that is not in the data the
bot trades on, or one that needs FUTURE candles to define (lookahead). So:
- Static drawings (method 1) = convey INTENT only.
- Real book/tape replay (method 2) = the SOURCE OF TRUTH for the rule.
- Likely need a small book/tape VISUALIZER that renders the book+tape around a
  timestamp so he can "see what the engine sees" and point at it. Does not exist
  yet; build early when an idea needs it.

## The loop (run per idea)
1. SHOW -- annotated image for intent, then find 5-10 real instances in `.forge`.
2. SPEC -- co-write `docs/research/<idea>.md`: one-line hypothesis; feature
   vocabulary it needs; decision table (entry/invalidate/exit/size); and the
   FALSIFICATION condition ("this edge is real only if X").
3. IMPLEMENT -- features + strategy in `forge-strategy` on our own primitives;
   deterministic; no mid-fills; fail-fast on bad data.
4. GATE -- null-edge test + knob-bite rule (swept param MUST change trade
   behaviour) before any P&L number is trusted.
5. EYEBALL -- he compares trades the bot took vs the ones he would have taken, on
   the same replay; correct the rule; iterate.

## To start a new idea, ask him for ONE of:
- A drawn-up screenshot of the setup (drag it in), even rough; or
- One sentence: "when I see X in the book and price does Y, I do Z, wrong when W."

## Cross-refs
- docs/legacy/bot-workflow.md -- per-bot research/sweep/verdict lifecycle.
- docs/engine-design.md -- no-lookahead, two-clock, fill model.
- docs/research/ -- where each idea's spec doc lives.