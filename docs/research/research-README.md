# Research Lab -- Index

The home page for the order-flow EDGE research. Start here.

## How this lab works (two sessions, one loop)
- RESEARCH session (Kiro): reads the literature, fills these notes with
  mechanism + citations + a PURSUE/PARK/DROP verdict, and writes the EXACT
  swept experiment / build task. Output = decisions and specs, NOT production
  code.
- BUILD session (Kiro): picks up a spec from these notes, IMPLEMENTS it behind a
  default-off flag, and validates it in `pnpm backtest` (DSR/OOS gates) BEFORE
  any live touch.
- The notes are the contract between the two. Findings flow research -> build;
  results flow build -> back here as updated verdicts.

## Standing rules (apply to every item)
- Signal changes are ADDITIVE and FLAG-GATED: byte-identical to today when off.
- Validate in `pnpm backtest` over captured Parquet before live. Use DSR + OOS.
- Cost wall: ~11 bps round-trip (~0.11% on BTC). Every success bar is net of it.
- Citations are paraphrased for licensing (the "why", not copy-paste).

## Documents
- `research-charter.md` -- the agenda (Q1-Q6) and how to run a session.
- `research-remaining-questions.md` -- Q1 (survey) / Q3 (stop-runs) / Q4
  (spoofing-as-filter) / Q6 (cross-venue lag) findings + verdicts.
- `research-data-sources.md` -- capture vendors (Tardis/Kaiko/Coinglass...)
  and the native-feed L0 path.
- `research-strategy-edges.md` -- "what works" survey: trend/momentum,
  funding carry, stat-arb, grid, ML -- cost-survival verdicts for our setup.
- `research-quant-methods-and-repos.md` -- HOW quants build strategies
  (meta-labelling, triple-barrier, CPCV, PBO, OU exits) + open-source repos
  to mine (Nautilus, mlfinlab, LOB/MM repos). Untrusted-code caveat inside.
- `research-flow-primitives.md` -- Q5 findings: harnessing OFI / MLOFI / CVD /
  VPIN / absorption / trade-sign persistence / queue / Hawkes. Includes the
  T1-T6 build tasks.
- `research-impact-measurement.md` -- Q2 findings: predict move SIZE (OFI
  linear + sqrt-law) and PERSISTENCE (order-flow autocorr); an impact-
  calibration layer (beta / Y / decay kernel) the bots consume. Tasks Q2-A..C.
- `research-volume-profile-and-liquidations.md` -- two future bots: Volume/
  Market-Profile (PURSUE) and Liquidation-heatmap (PARK, data-blocked). Includes
  the L0-L2 build tasks.

## Verdict ledger (current state)
Signals / primitives (from Q5):
| Item                     | Verdict                         | Build state            |
|--------------------------|---------------------------------|------------------------|
| OFI                      | PURSUE (backbone)               | T1: depth-normalise    |
| Absorption               | PURSUE (reversal lead)          | T2: sign + reload      |
| CVD divergence           | PURSUE (low confidence)         | T3: volume-clock+pivot |
| MLOFI                    | PURSUE (must beat OFI)          | T4: integrated agg     |
| Trade-sign persistence   | PURSUE (hold-time meta)         | T5: adaptive hold      |
| VPIN                     | PARK standalone / gate only     | T6: true-sign gate     |
| Hawkes                   | PARK (regime descriptor only)   | --                     |
| Queue dynamics           | DROP (HFT / cost-killed)        | feature input only     |

Future bots:
| Bot                      | Verdict                         | Build state            |
|--------------------------|---------------------------------|------------------------|
| Volume / Market Profile  | PURSUE (no new data)            | build next             |
| Liquidation: cascade-fade| PURSUE-AFTER-CAPTURE            | needs L0 capture       |
| Liquidation: magnet      | PARK (OI heatmap build)         | later (L2)             |

Strategy families (what-works survey):
| Family                       | Verdict                  | Blocker / fit       |
|------------------------------|--------------------------|---------------------|
| Time-series momentum (trend) | PURSUE                   | needs long history  |
| Trend/regime filter on micro | PURSUE (synergy)         | none -- cheap        |
| Funding carry / basis        | PARK (data-gated)        | funding+spot capture|
| Stat-arb / cointegration     | PARK (needs universe)    | multi-asset capture |
| Grid trading                 | CAUTION (no edge)        | --                   |
| ML direction classifiers     | PARK (overfit/low SNR)   | --                   |

## Open research questions (charter)
- Q1 broad order-flow survey ......... DONE (research-remaining-questions.md)
- Q2 impact / move-size + persistence . DONE (research-impact-measurement.md)
- Q3 liquidity traps / stop-runs ...... DONE (research-remaining-questions.md)
- Q4 spoofing detection ............... DONE -- PURSUE as FAKE-wall filter
- Q5 flow-measurement survey .......... DONE (research-flow-primitives.md)
- Q6 cross-venue lag .................. DONE -- big-move variant only
- Extra: Volume Profile + Liquidations  DONE (separate note)

## Data status (what grounds/limits the research)
- CAPTURED: Binance depth20@100ms, trade, bookDelta (top-15), bookSnapshot;
  Hyperliquid hlquote (BTC).
- Valid 3-stream backtest window so far: ~7h (2026-06-06 05:00-12:00 UTC) -- thin
  for DSR/OOS confidence. MORE CAPTURE is the gating dependency for most verdicts.
- NOT captured (blocks bots): liquidations (forceOrder), open interest, funding;
  Bybit/OKX/Coinbase (for Q6 multi-venue).

## Research tooling (MCP)
- arXiv + Semantic Scholar MCP servers are wired in `.kiro/settings/mcp.json`
  (gitignored) -- literature search + citation graph in-session.
- Postgres MCP (user-level) -- read-only access to the trade DB to ground
  research in real `closed_trades`.
- A free Semantic Scholar API key in `SEMANTIC_SCHOLAR_API_KEY` raises rate limits.

## Adding a new finding (template)
For each new question/bot, create `research-<topic>.md` with:
mechanism -> literature (cited) -> what we already have -> our gap -> verdict
(PURSUE/PARK/DROP) -> flag-gated change + swept experiment + success bar.
Then add a row to the verdict ledger above and link the note under Documents.