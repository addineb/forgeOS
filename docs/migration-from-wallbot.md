# Migration from wall-bot-tournament

## Kept (non-executing; cannot alter engine results)
- cryptohftdata feed + account (external data source).
- `tools/chd-to-parquet.py` — verified-accurate converter (may become reference
  once the Rust preprocessor reads CHD parquet directly).
- `docs/research/*` — all preserved research/findings.
- `docs/legacy/*` — business-rule + workflow reference (READ ONLY; never port code).
- Validation methodology (PBO, CPCV, DSR, OOS, knob-bite, null-edge) — re-coded
  fresh in Rust.
- Hetzner box + tmux workflow; the determinism contract as a principle.

## Reference-only (read for intent, never imported/ported)
- The old TS `engine.ts`, fill-engine, primitives' math, bot parameter values.
  Treated as suspect: re-derive, then test against the null-edge harness.

## Dropped / frozen
- All executable TS (engine, harness, parquet-source, book-reconstructor,
  book-throttle, fill-engine, signal sources).
- Live TS stack + Railway deploy (retired).
- R2 capture pipeline + bucket (buggy, retired).
- Old sweep specs + result artifacts (produced by the broken engine).
- Tardis data/converter (deleted); dashboard (rebuild later if ever).

## Status of old repo
`wall-bot-tournament` is FROZEN as a read-only reference + history. It is not
deleted: we still read its business logic while re-deriving strategies. No
ForgeOS code depends on it.