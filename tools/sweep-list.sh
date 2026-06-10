#!/usr/bin/env bash
# sweep-list.sh - list live ForgeOS tmux sessions and the most recent run logs.
set -euo pipefail
RUNS="${FORGE_RUNS_DIR:-/root/runs}"
echo "== live tmux sessions =="
tmux ls 2>/dev/null || echo "(none)"
echo "== recent logs in $RUNS =="
ls -t "$RUNS"/*.log 2>/dev/null | head -20 || echo "(none)"