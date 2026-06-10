# Spec: Idea Annotator + Book/Tape Visualizer

Status: SPEC. The tool that turns the trader's discretionary setups into
data-anchored, no-lookahead labels Kiro can build rules from. Pairs with
docs/legacy/idea-transfer-workflow.md and docs/research/edge-directions.md
(chart-trigger x orderflow-confirm).

## Purpose
Let the trader (a) mark a setup by drawing + a sentence, and (b) eventually SEE
what the engine sees (book + tape), so the rule we code matches what he means -
with every annotation tied to a real timestamp so there is no lookahead.

## Two phases (build only what an idea needs)
### Phase A - INTENT capture (cheap, near-zero build)
Surface: Obsidian (+ Excalidraw plugin) OR any drawing tool. The trader:
- drops/loads a chart image of the setup, draws boxes/arrows,
- writes the setup in one sentence + the EXACT timestamp/window + invalidation.
- saves it as a note in `docs/research/labels/<idea>-<ts>.md`.
Kiro reads the note + pulls the real `.forge` window behind that timestamp.
NOTE: Obsidian/candles carry INTENT only - they cannot show orderflow and an
arrow on a candle can imply lookahead. The real data is the source of truth.
This phase needs no custom code; just a label-note convention (template below).

### Phase B - TRUTH view (custom build, when an idea needs orderflow)
A small LOCAL web app on OUR data:
- price panel (TradingView Lightweight Charts - free, open source),
- order-book panel (depth around the touch over time),
- trade-tape panel (prints, aggressor side, size),
- scrub/replay around a timestamp; draw overlay; description box;
- SAVE -> writes a label file (below). Later: overlay the BOT's entries/exits on
  the same replay so he can eyeball bot-trades vs his-trades.
Engine-safe: a separate tool under tools/, reads `.forge`, never touches the sim.

## Label file format (what Kiro consumes) - JSON or md front-matter
- idea: short name
- window: file + [start_ts, end_ts] (ns)
- trigger_ts: the moment he would act (decision point; must be detectable <= now)
- direction: long | short
- setup: one sentence ("price swept prior high then back below")
- invalidation: one sentence ("wrong if it reclaims the high")
- orderflow_notes: what he sees behind it (absorption / pull / cvd divergence...)
- drawings: optional (excalidraw json / image path)

## No-lookahead rule (hard)
trigger_ts and every input must be computable from data with ts <= trigger_ts.
The OUTCOME (did it work) is future = what we score, never an input. Test: "could
the engine detect this trigger live, at trigger_ts, with no future bars?"

## How it feeds the pipeline (chart-trigger x orderflow-confirm)
1. Labels define the TRIGGER (the chart pattern) + give real instances.
2. We code the trigger (real-time, past-only) -> it fires on those instances.
3. We measure orderflow at each trigger; test pattern-alone vs pattern+filter.
4. Gates: shuffled control, null-edge, knob-bite, DSR/PBO, paper.

## Build order (later, after lead-lag)
1. Label-note template + `docs/research/labels/` (Phase A) - trivial.
2. Phase B visualizer v1: price + tape; then book panel; then bot-trade overlay.