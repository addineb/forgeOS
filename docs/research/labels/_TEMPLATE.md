---
idea:
date:
status: draft
direction:
window_file:
window_range:
trigger_ts:
tags: [label]
---
# <idea name>

## Screenshot
(drag your chart image here)

## Setup (one sentence, plain)
When I see ___ in the book/tape and price does ___, I ___.

## Trigger - what fires the entry
Must be computable at trigger_ts from PAST price/flow only (no lookahead).
e.g. "price trades above the prior swing high, then prints back below it"

## Invalidation (wrong if...)
e.g. "wrong if it reclaims the high and holds"

## Orderflow behind it (real vs fake check)
What should the book/tape show for this to be REAL?
absorption? wall pulled vs eaten? CVD divergence? thinning? big aggressor?

## Notes
anything else - which regime, time of day, typical hold, etc.