---
tags: [meta, guide]
type: note
---
# How this vault works (MOC second-brain)

Method: Maps of Content (MOC). A few hub notes link related notes; the Graph View
turns those links into the connected "dots" picture. Low overhead, grows with us.

## The hubs (start here)
- [[ForgeOS-HOME]] - top map + the visual [[ForgeOS-map.canvas]].
- [[MOC-Engine]] - the sacred core.
- [[MOC-Strategies]] - the bots + status.
- [[MOC-Edge-Hunt]] - where edge is + findings + the brain->strategy tools.
- [[MOC-Decisions]] - decision log + standing rules.

## How to add a note (keep it frictionless)
1. Make a note anywhere (research goes in docs/research/, labelled setups in
   docs/research/labels/).
2. Add it to the right MOC with a one-line "why it matters".
3. Add `tags:` in YAML frontmatter so it clusters in the graph.
That is it. Do not over-organise; let structure emerge from links.

## Views
- Graph view: see the whole web; MOCs are the big hubs.
- Canvas ([[ForgeOS-map.canvas]]): the deliberate engine-in-the-middle diagram.
- NotebookLM: upload [[PROJECT-CONTEXT]] (+ docs) for a queryable brain.

## Kiro + this vault
Kiro edits these files directly (you see changes live) and auto-loads
.kiro/steering/state.md every session. No API needed - the folder IS the interface.