#!/usr/bin/env bash
# hunt.sh - slow-horizon edge hunt across ALL *.forge windows, both bots.
# Runs OFI then WALL (sequential to bound RAM). Teed by sweep.sh to a logfile.
set -uo pipefail
source /root/.cargo/env 2>/dev/null || true
cd /root/forgeOS
WINS=$(ls /root/forgeOS/data/*.forge)
NW=$(echo "$WINS" | wc -l)
echo "########## SLOW HUNT over $NW windows ##########"
echo "########## OFI MOMENTUM (slow) ##########"
cargo run --release -q -p forge-sweep --bin forge-sweep -- $WINS --strategy ofi --preset slow --leverage 20 --top 30
echo "########## WALL / IMBALANCE (slow) ##########"
cargo run --release -q -p forge-sweep --bin forge-sweep -- $WINS --strategy wall --preset slow --leverage 20 --top 30
echo "########## SLOW HUNT DONE ##########"