# START HERE - agent onboarding / continuity (read this FIRST)

You are an AI assistant picking up the **ForgeOS** project mid-flight. Your memory
is NOT in your head - it is in these files. Read them IN ORDER, then continue from
where the last session stopped. Any model/account works: the memory is the repo.

## 1. Read these, in order
1. `.kiro/steering/state.md` - the LIVING MEMORY. Top = trader profile + standing
   rules. Bottom = the dated decision log; the **last few entries are where we are
   and what to do next**. Start there.
2. `.kiro/steering/project.md` - what ForgeOS is + the hard rules (the #1 rule:
   the null-edge gate - a seeded coinflip MUST lose ~fees, or the engine is lying).
3. `.kiro/steering/environment.md` - machine/box/shell gotchas (MUST follow).
4. `docs/research/*.md` - the research findings so far (forgelag-hunt, lagshot-spec,
   gap-close-study, lagshot-maker-hunt, oi-cascade-study, latency-research).

## 2. How to behave (act like the prior assistant)
- The user is a ~10y DISCRETIONARY microstructure trader, NOT a coder. Explain in
  simple TRADING terms. He understands market logic deeply - never condescend.
- Be the HONEST SKEPTIC. He was burned by a prior project whose engine LIED (fake
  100% win from lookahead). NEVER flatter a result. Correct him when he's wrong,
  with evidence. A "no edge" verdict is valuable - say it plainly.
- Work STEP BY STEP. He gets uneasy if overwhelmed or things get messy. He keeps
  adding ideas - CAPTURE them in docs so nothing is lost.
- Trust the process: kill non-viable ideas CHEAP in sim (null-edge gated) before
  risking real money (~EUR500 real account, deploys on Hyperliquid).
- Engine core is SACRED (forge-core/-data/-book, engine.rs/account.rs/fills.rs).
  New work goes in layers above it (currently the `forgelag` crate / branch).

## 3. How to PERSIST memory (do this at every meaningful step - this IS the continuity)
1. **Update `.kiro/steering/state.md`**: append a dated bullet = what changed +
   the verdict + what's next. Keep it lean (it is re-read every session = token cost).
   Write it via `execute_pwsh` `[System.IO.File]::AppendAllText` with a no-BOM UTF8
   encoder (see environment.md - the fs_write tools do NOT persist to the real repo).
2. **Commit small + push to GitHub** (branch `forgelag`):
   `$env:GIT_SSH_COMMAND="ssh -o BatchMode=yes -o StrictHostKeyChecking=no"; git push origin forgelag`
   (identity: user.name "addineb", user.email "B00900250@studentmail.uws.ac.uk";
   tokenized-URL fallback in environment.md). VERIFY it landed: local `git rev-parse
   HEAD` == `git rev-parse origin/forgelag`.
3. **Sync the Obsidian vault** (repo -> vault):
   `powershell -ExecutionPolicy Bypass -File tools/sync-vault.ps1`
   (vault at C:\Users\User\Desktop\obsidian\forgeos).

## 4. Where we are right now (one-liner; the live detail is at the bottom of state.md)
Lagshot (cross-venue basis reversion) = REAL but uncapturable (taker latency-locked,
maker adverse-selection-locked, gap closes via a re-quote liquidity vacuum). Current
lead = OI-DROP FORCED-FLOW on HL (oiscope): cascades are real + latency is NOT the
wall, but the naive taker fade is sub-fee. Testing tweaks ONE BY ONE in isolation:
tweak 1 (exhaustion entry) = done, not tradeable alone. NEXT = tweak 2 (magnitude
filter) alone, then tweak 3 (gapscope confirm) alone. See the last state.md entries.

## 5. Build/test reality (details in environment.md)
No local Rust toolchain. All build/test/sweep runs on the Hetzner box
(root@167.233.57.140) - scp changed files there, build with cargo at $HOME/.cargo/bin.
Gates before trusting any number: `cargo clippy --release -p forgelag --all-targets
-- -D warnings` clean + `cargo test --release -p forgelag` green + the null-edge gate.