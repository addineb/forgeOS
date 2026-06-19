#!/usr/bin/env bash
# ── run_anomaly_validation.sh ─────────────────────────────────────────────────
# Runs forge-anomaly validate on BTC depthscope data for a given date.
#
# Usage:
#   ./run_anomaly_validation.sh 2026-06-05              # print to stdout
#   ./run_anomaly_validation.sh 2026-06-05 --verbose    # per-event detail
#   ./run_anomaly_validation.sh 2026-06-05 --save       # save to results file
#   ./run_anomaly_validation.sh --all                   # all available dates
#   ./run_anomaly_validation.sh --help
#
# Paths (edit these if your Hetzner layout differs):
#   Binary:   /root/forgeOS/target/release/validate
#   Data dir: /root/depthscope_out
#   Data pattern: BTCUSDT_YYYY-MM-DD_vb10.csv
# ────────────────────────────────────────────────────────────────────────────────

set -euo pipefail

# ── config ────────────────────────────────────────────────────────────────────
BINARY="/root/forgeOS/target/release/validate"
DATA_DIR="/root/depthscope_out"
RESULTS_DIR="/root/forgeOS/results"

SAVE=false
VERBOSE=""
DATES=()

# ── help ──────────────────────────────────────────────────────────────────────
usage() {
    cat <<'EOF'
Usage: run_anomaly_validation.sh [OPTIONS] [DATE...]

  DATE               One or more dates as YYYY-MM-DD (e.g. 2026-06-05).
  --all              Run on all BTCUSDT_*_vb10.csv files in data dir.
  --save             Save output to results/results_YYYY-MM-DD.txt.
  --verbose          Pass --verbose to validate for per-event output.
  --help             Show this message.

Examples:
  ./run_anomaly_validation.sh 2026-06-05
  ./run_anomaly_validation.sh 2026-06-02 2026-06-03 --save
  ./run_anomaly_validation.sh --all --save
EOF
    exit 0
}

# ── parse args ────────────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
    case "$1" in
        --help|-h)
            usage
            ;;
        --all)
            # Discover all dates from CSV files in data dir.
            mapfile -t DATES < <(
                find "$DATA_DIR" -maxdepth 1 -name 'BTCUSDT_*_vb10.csv' -printf '%f\n' \
                | sed -n 's/^BTCUSDT_\([0-9]\{4\}-[0-9]\{2\}-[0-9]\{2\}\)_vb10\.csv$/\1/p' \
                | sort -u
            )
            if [[ ${#DATES[@]} -eq 0 ]]; then
                echo "ERROR: no BTCUSDT_*_vb10.csv files found in $DATA_DIR"
                exit 1
            fi
            ;;
        --save)
            SAVE=true
            ;;
        --verbose)
            VERBOSE="--verbose"
            ;;
        --*)
            echo "ERROR: unknown option: $1"
            usage
            ;;
        *)
            DATES+=("$1")
            ;;
    esac
    shift
done

# ── validate prerequisites ────────────────────────────────────────────────────
if [[ ! -x "$BINARY" ]]; then
    echo "ERROR: validate binary not found at $BINARY"
    echo "  Build it first: cargo build --release -p forge-anomaly"
    exit 1
fi

if [[ ! -d "$DATA_DIR" ]]; then
    echo "ERROR: data directory not found: $DATA_DIR"
    exit 1
fi

if [[ ${#DATES[@]} -eq 0 ]]; then
    echo "ERROR: no dates specified. Use --all or pass one or more dates."
    usage
fi

# ── create results dir if saving ──────────────────────────────────────────────
if $SAVE; then
    mkdir -p "$RESULTS_DIR"
fi

# ── run ───────────────────────────────────────────────────────────────────────
TOTAL_SIGNALS=0
TOTAL_PATTERN=0
TOTAL_BARS=0
EXIT_CODE=0

for DATE in "${DATES[@]}"; do
    CSV_FILE="${DATA_DIR}/BTCUSDT_${DATE}_vb10.csv"

    if [[ ! -f "$CSV_FILE" ]]; then
        echo "WARNING: skipping $DATE — file not found: $CSV_FILE"
        continue
    fi

    echo ""
    echo "────────────────────────────────────────────────────────────────"
    echo "  DATE: $DATE"
    echo "  CSV:  $CSV_FILE"
    echo "────────────────────────────────────────────────────────────────"

    if $SAVE; then
        OUT_FILE="${RESULTS_DIR}/results_${DATE}.txt"
        echo "  → saving to $OUT_FILE"
        "$BINARY" --input "$CSV_FILE" --date "$DATE" $VERBOSE --output "$OUT_FILE" || {
            echo "ERROR: validate failed for $DATE (exit code $?)"
            EXIT_CODE=1
            continue
        }
        # Print a quick summary line from the saved file.
        if grep -q "total signals:" "$OUT_FILE" 2>/dev/null; then
            grep -E "(total signals:|pattern-based:|avg confidence:)" "$OUT_FILE" | sed 's/^/  /'
        fi
    else
        "$BINARY" --input "$CSV_FILE" --date "$DATE" $VERBOSE || {
            echo "ERROR: validate failed for $DATE (exit code $?)"
            EXIT_CODE=1
            continue
        }
    fi

    # Parse counts from output for aggregate summary.
    if $SAVE; then
        SIGS=$(grep "total signals:" "$OUT_FILE" | grep -oP '\d+' | head -1) || SIGS=""
        PATT=$(grep "pattern-based:" "$OUT_FILE" | grep -oP '\d+' | head -1) || PATT=""
        BARS=$(grep "bars processed:" "$OUT_FILE" | grep -oP '\d+' | head -1) || BARS=""
    else
        # We can't re-read stdout — run a second quick pass just for stats.
        STATS=$("$BINARY" --input "$CSV_FILE" --date "$DATE" 2>/dev/null | grep -E "(total signals:|pattern-based:|bars processed:)" || true)
        SIGS=$(echo "$STATS" | grep "total signals:" | grep -oP '\d+' | head -1) || SIGS=""
        PATT=$(echo "$STATS" | grep "pattern-based:" | grep -oP '\d+' | head -1) || PATT=""
        BARS=$(echo "$STATS" | grep "bars processed:" | grep -oP '\d+' | head -1) || BARS=""
    fi
    TOTAL_SIGNALS=$((TOTAL_SIGNALS + ${SIGS:-0}))
    TOTAL_PATTERN=$((TOTAL_PATTERN + ${PATT:-0}))
    TOTAL_BARS=$((TOTAL_BARS + ${BARS:-0}))
done

# ── aggregate summary ─────────────────────────────────────────────────────────
if [[ ${#DATES[@]} -gt 1 ]]; then
    echo ""
    echo "════════════════════════════════════════════════════════════════"
    echo "  AGGREGATE (${#DATES[@]} dates)"
    echo "────────────────────────────────────────────────────────────────"
    echo "  bars processed:     $TOTAL_BARS"
    echo "  total signals:      $TOTAL_SIGNALS"
    if [[ $TOTAL_SIGNALS -gt 0 ]]; then
        echo "  pattern-based:      $TOTAL_PATTERN  ($(awk "BEGIN {printf \"%.1f\", $TOTAL_PATTERN/$TOTAL_SIGNALS*100}")% of signals)"
    fi
    if [[ $TOTAL_BARS -gt 0 ]]; then
        echo "  signal rate:        $(awk "BEGIN {printf \"%.2f\", $TOTAL_SIGNALS/$TOTAL_BARS*100}")%"
    fi
    echo "════════════════════════════════════════════════════════════════"
fi

if $SAVE; then
    echo ""
    echo "Results saved to: $RESULTS_DIR/"
    ls -la "$RESULTS_DIR"/results_*.txt 2>/dev/null || echo "  (no files written)"
fi

exit $EXIT_CODE
