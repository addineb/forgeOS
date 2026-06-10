#!/usr/bin/env bash
# sweep.sh - launch a long ForgeOS run in a DETACHED tmux session, teed to a
# timestamped logfile under /root/runs. On completion it writes a .done marker
# (with the exit code) and, if FORGE_NOTIFY_URL is set, pings it (ntfy/Discord/
# Telegram webhook) so you get told when it finishes. Survives SSH drops.
#
# Usage:  tools/sweep.sh <session-name> <command...>
# Wait:   tools/sweep-wait.sh <session-name>     (blocks until done)
# Poll:   tools/sweep-status.sh <session-name>
set -euo pipefail

SESSION="${1:?usage: sweep.sh <session-name> <command...>}"; shift
[ "$#" -ge 1 ] || { echo "error: no command given" >&2; exit 2; }

RUNS="${FORGE_RUNS_DIR:-/root/runs}"
mkdir -p "$RUNS"
TS="$(date -u +%Y%m%dT%H%M%SZ)"
LOG="$RUNS/${SESSION}-${TS}.log"
DONE="$RUNS/${SESSION}.done"
rm -f "$DONE"

if tmux has-session -t "$SESSION" 2>/dev/null; then
  echo "error: tmux session '$SESSION' already exists." >&2
  echo "  inspect: tmux attach -t $SESSION   kill: tmux kill-session -t $SESSION" >&2
  exit 3
fi

RUNSCRIPT="$(mktemp "/tmp/forge-${SESSION}.XXXXXX.sh")"
cat > "$RUNSCRIPT" <<INNER
#!/usr/bin/env bash
source /root/.cargo/env 2>/dev/null || true
set -o pipefail
echo "[sweep] start ${TS} :: $*"
$* 2>&1 | tee "${LOG}"
ec=\${PIPESTATUS[0]}
echo "[sweep] exit \${ec}" | tee -a "${LOG}"
echo "\${ec}" > "${DONE}"
if [ -n "\${FORGE_NOTIFY_URL:-}" ]; then
  curl -fsS -m 15 -d "forgeOS sweep ${SESSION} finished (exit \${ec})" "\${FORGE_NOTIFY_URL}" >/dev/null 2>&1 || true
fi
INNER
chmod +x "$RUNSCRIPT"

tmux new-session -d -s "$SESSION" "bash '$RUNSCRIPT'"

echo "launched   session=$SESSION"
echo "logfile    $LOG"
echo "wait       tools/sweep-wait.sh $SESSION    (blocks until done)"
echo "poll       tools/sweep-status.sh $SESSION"
echo "attach     tmux attach -t $SESSION   (detach: Ctrl-b d)"