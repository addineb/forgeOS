#!/usr/bin/env bash
# pull-data.sh - pull cryptohftdata day(s) and convert each into a *.forge
# window for sweeps. Pulls 24h of trade/bookDelta/hlquote parquet, then converts
# hours 00-01 (2h) to data/btc-<date>-00_01.forge. Run detached via sweep.sh.
# Usage: pull-data.sh <YYYY-MM-DD> [more dates...]
set -uo pipefail
source /root/.cargo/env 2>/dev/null || true
KEY="$(cat /root/.chd_key)"
ROOT=/root/chd/data
OUT=/root/forgeOS/data
mkdir -p "$OUT"
cd /root/forgeOS

for d in "$@"; do
  echo "=== $d : pulling parquet (trade+bookDelta+hlquote, 24h) ==="
  if CRYPTOHFTDATA_API_KEY="$KEY" python3 /root/forgeOS/tools/chd-to-parquet.py --date "$d" --data-dir "$ROOT"; then
    echo "=== $d : converting hours 00,01 -> forge ==="
    cargo run --release -q -p forge-data --features convert --bin forge-convert -- \
      --root "$ROOT/ticks" --symbol BTCUSDT --coin BTC --date "$d" --hours 00,01 \
      --feed-latency-ns 2000000 --out "$OUT/btc-${d}-00_01.forge" --verify \
      || echo "!! convert failed for $d"
  else
    echo "!! pull failed for $d (skipping)"
  fi
done

echo "=== DONE; forge windows available ==="
ls -la "$OUT"/*.forge