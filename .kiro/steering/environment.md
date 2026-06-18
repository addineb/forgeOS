# Environment gotchas (this PC / this workspace)

Always-on. These caused real friction in the prior project; follow them.

## File writes (IMPORTANT)
- The file-writing tools (fs_write / str_replace / fs_append) write to a
  VIRTUALIZED temp path, NOT the real workspace. ALWAYS write/edit files via
  `execute_pwsh` using `[System.IO.File]::WriteAllText` with a no-BOM UTF8
  encoder: `New-Object System.Text.UTF8Encoding($false)`.
- `read_file` / `read_files` / `read_code` / `grep_search` / `get_diagnostics`
  work fine on real absolute paths.
- For `.Replace` edits on existing files: detect newline first
  (`$crlf = ([regex]::Matches($t,"`r`n")).Count -gt 0`) and normalize your
  oldStr/newStr to match — PowerShell here-strings arrive as LF, so convert to
  CRLF when the file is CRLF or the match silently fails.
- `.Replace` is safe: if oldStr is absent the file is unchanged; verify with
  `.Contains()`.

## Shell
- Shell is PowerShell. Use `;` not `&&`. Multi-line `if/else` must keep
  `} else {` on one line.
- Terminal output is heavily truncated (tail only). Write results to a UNIQUE
  temp file and `read_file` it. Reused filenames hit a stale-read trap - use a
  guid suffix or delete first.

## Git Bash for ssh (THE shell fix - use this for all box work)
Git Bash at `C:\Program Files\Git\bin\bash.exe` solves PowerShell->ssh quoting/
truncation. Pattern: write a local `.sh` (UTF8 no-BOM, LF) with a heredoc:
    ssh -o BatchMode=yes -o StrictHostKeyChecking=no root@167.233.57.140 bash -s <<'EOF'
    ...real bash; pipes | quotes "x" python3 -c "..." all literal...
    EOF
Run: `& "C:\Program Files\Git\bin\bash.exe" "<path>.sh"`. Use `<<'EOF'` (quoted)
so local $ / backticks are not expanded. Output returns clean + untruncated.

## Hetzner box (all compute lives here; Railway retired)
- `ssh root@167.233.57.140`, 100 GB disk, 4 cores. New project root: /root/forgeOS.
- Clean feed: /root/chd/data/ticks (do NOT delete). Converter: tools/chd-to-parquet.py.
- cryptohftdata API key is on the box at /root/.chd_env but that file is BROKEN
  (sourcing dumps env) - pass the key INLINE in commands.

## tmux (run long sweeps detached - install + use)
- Start: `tmux new -s sweep1`; launch the sweep teed to a logfile
  (`cargo run --release ... 2>&1 | tee /root/runs/sweep1.log`); detach `Ctrl-b d`.
- Poll without timeouts: `tmux capture-pane -pt sweep1 | tail` or `tail -f` the
  logfile over ssh. Survives SSH drops - fixes the long-run timeout/truncation pain.

## Git / push
- Push via tokenized URL (scrub the token from any captured output):
  `$env:GIT_TERMINAL_PROMPT=0; git -c credential.helper= push "https://<PAT>@github.com/addineb/forgeOS.git" HEAD:refs/heads/main`
  then `-replace 'ghp_[A-Za-z0-9]+','***'`.
- Verify a push landed: compare `git rev-parse HEAD` to
  `git ls-remote <url> refs/heads/main` (local tracking ref can be stale).
- Identity if missing: user.name "addineb",
  user.email "B00900250@studentmail.uws.ac.uk".

## Rust build/test
- `cargo build --release` / `cargo test` / `cargo clippy`. Release profile uses
  panic=abort, thin LTO. Prefer writing test output to a file for clean reads.
## Box source sync (git pull — set up 2026-06-18)
- `/root/forgeOS` is a real git clone of `addineb/forgeOS` on branch `main`.
  Credentials live in `/root/.git-credentials` (credential.helper=store).
- **Normal update after you push from laptop:**
  `ssh root@167.233.57.140 'cd /root/forgeOS && git pull --ff-only origin main'`
- Machine-local paths are NOT in git (preserved across clone): `/root/forgeOS/data`
  (*.forge windows), `/root/chd/data` (parquet), `/root/depthscope_out` (CSVs),
  `/root/runs` (sweep logs).
- scp is only for uncommitted local experiments; committed work = push then pull.
