#!/usr/bin/env bash
# sweep-status.sh - poll a detached ForgeOS run without attaching: shows whether
# the tmux session is alive, the live pane tail, and the tail of its newest log.
# Usage: tools/sweep-status.sh <session-name> [n-lines]
set -euo pipefail

SESSION="${1:?usage: sweep-status.sh <session-name> [n-lines]}"
N="${2:-40}"
RUNS="${FORGE_RUNS_DIR:-/root/runs}"

if tmux has-session -t "$SESSION" 2>/dev/null; then
  echo "== session '$SESSION': RUNNING =="
  tmux capture-pane -pt "$SESSION" | tail -n "$N"
else
  echo "== session '$SESSION': not running (finished or never started) =="
fi

LATEST="$(ls -t "$RUNS/${SESSION}"-*.log 2>/dev/null | head -1 || true)"
if [ -n "${LATEST:-}" ]; then
  echo "== logfile $LATEST (last $N) =="
  tail -n "$N" "$LATEST"
else
  echo "== no logfile found under $RUNS for '$SESSION' =="
fi