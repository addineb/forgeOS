#!/usr/bin/env bash
# sweep.sh - launch a long ForgeOS run (sweep/backtest) inside a DETACHED tmux
# session, teed to a timestamped logfile under /root/runs. Survives SSH drops,
# fixes the long-run timeout/truncation pain (see .kiro/steering/environment.md).
#
# Usage:  tools/sweep.sh <session-name> <command...>
# Example tools/sweep.sh sweep1 cargo run --release -p forge-sweep -- --config sweep.toml
#
# Poll:   tools/sweep-status.sh <session-name>
# Attach: tmux attach -t <session-name>      (detach again with Ctrl-b d)
set -euo pipefail

SESSION="${1:?usage: sweep.sh <session-name> <command...>}"; shift
[ "$#" -ge 1 ] || { echo "error: no command given" >&2; exit 2; }

RUNS="${FORGE_RUNS_DIR:-/root/runs}"
mkdir -p "$RUNS"
TS="$(date -u +%Y%m%dT%H%M%SZ)"
LOG="$RUNS/${SESSION}-${TS}.log"

if tmux has-session -t "$SESSION" 2>/dev/null; then
  echo "error: tmux session '$SESSION' already exists." >&2
  echo "  inspect: tmux attach -t $SESSION" >&2
  echo "  kill:    tmux kill-session -t $SESSION" >&2
  exit 3
fi

# Write the run body to a temp script so tmux always executes it under bash with
# a real PATH (cargo), pipefail, and a durable tee'd log. Avoids dash/quoting traps.
RUNSCRIPT="$(mktemp "/tmp/forge-${SESSION}.XXXXXX.sh")"
cat > "$RUNSCRIPT" <<INNER
#!/usr/bin/env bash
source /root/.cargo/env 2>/dev/null || true
set -o pipefail
echo "[sweep] start ${TS} :: $*"
$* 2>&1 | tee "${LOG}"
ec=\${PIPESTATUS[0]}
echo "[sweep] exit \${ec}" | tee -a "${LOG}"
INNER
chmod +x "$RUNSCRIPT"

tmux new-session -d -s "$SESSION" "bash '$RUNSCRIPT'"

echo "launched   session=$SESSION"
echo "logfile    $LOG"
echo "poll       tools/sweep-status.sh $SESSION"
echo "follow     tail -f $LOG"
echo "attach     tmux attach -t $SESSION   (detach: Ctrl-b d)"