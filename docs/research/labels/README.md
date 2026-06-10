---
tags: [labels, workflow]
type: note
---
# Setup labels - your brain -> a tested rule (start here)

Drop one note per setup you want me to turn into a coded, tested trigger. Author
them in Obsidian (drag a screenshot straight in); I read them from the vault and
build from them. See also [[idea-transfer-workflow]] and [[annotator-visualizer]].

## How to add a setup (30 seconds)
1. Duplicate [[_TEMPLATE]] -> rename it (e.g. "sweep-prior-high").
2. Drag your chart screenshot into the note.
3. Fill the fields. Most important: the EXACT timestamp/window so I can pull the
   real order book + tape behind it.
4. Tell me the note name; I take it from there.

## THE ONE RULE - no lookahead (this is what killed the old project)
The TRIGGER must be detectable at the moment you would act, using ONLY price up to
that instant. Not "it reversed afterwards" - that needs the future.
- OK:  "prior high = X; price traded above X and is now back below X"  (all known now)
- NOT: "price spiked then dropped 1%"  (needs the future drop to know)
The trade OUTCOME (did it win) is in the future - that is what we SCORE, never an
input to the decision.

## What happens next
I code the trigger (real-time, past-only), measure the orderflow at each instance,
and test: does pattern-alone vs pattern+orderflow-confirm lift the edge? Then the
usual gates (shuffled control, knob-bite, DSR/PBO, paper).