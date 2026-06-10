---
idea: sweep-prior-high
date: 2026-06-10
status: draft
direction: short
window_file: btc-2026-02-01
window_range: 12:30:00 - 12:45:00
trigger_ts: 12:38:07
tags: [label, example]
---
# Sweep of prior high (liquidity grab) - EXAMPLE

(This is a filled example so you see the shape. Replace with your real setups.)

## Screenshot
(a chart image would be dragged here)

## Setup (one sentence)
Price pokes just above the prior session high to grab stops, then falls back
under it - I short the failed breakout.

## Trigger - what fires the entry
Prior high = X (known). Price trades above X, then prints back below X. Fire SHORT
at the first print back below X. (All known at trigger_ts - no future needed.)

## Invalidation (wrong if...)
Wrong if price reclaims X and holds above it for more than a few seconds.

## Orderflow behind it (real vs fake check)
REAL fake-out (good short) if: the bids that pushed above X were PULLED (spoof) /
thin, little aggressive buy volume executed up there, ask absorption on the push.
NOT a short if: heavy aggressive buying actually EXECUTED through X (real breakout).

## Notes
Works best in range/sideways regime; typical hold minutes; this is the
liquidity-sweep (SW1) idea - pairs with the wall-flow pulled-vs-eaten read.