#!/usr/bin/env bash
# sweep-wait.sh - block until a sweep session finishes, then print the exit code
# and the tail of its log. Lightweight poll; safe to run over ssh.
# Usage: tools/sweep-wait.sh <session-name> [poll_secs] [tail_lines]
set -euo pipefail
SESSION="${1:?usage: sweep-wait.sh <session-name> [poll_secs] [tail_lines]}"
POLL="${2:-5}"
TAILN="${3:-30}"
RUNS="${FORGE_RUNS_DIR:-/root/runs}"

if ! tmux has-session -t "$SESSION" 2>/dev/null; then
  echo "'$SESSION' is not running (already finished or never started)"
else
  echo "waiting for '$SESSION' (poll ${POLL}s)..."
  while tmux has-session -t "$SESSION" 2>/dev/null; do sleep "$POLL"; done
fi

echo "== '$SESSION' finished =="
DONE="$RUNS/${SESSION}.done"
[ -f "$DONE" ] && echo "exit code: $(cat "$DONE")"
LATEST="$(ls -t "$RUNS/${SESSION}"-*.log 2>/dev/null | head -1 || true)"
[ -n "${LATEST:-}" ] && { echo "== $LATEST (last $TAILN) =="; tail -n "$TAILN" "$LATEST"; }