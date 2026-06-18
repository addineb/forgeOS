#!/usr/bin/env bash
set -euo pipefail
BOX=root@167.233.57.140
SSH="ssh -o BatchMode=yes -o StrictHostKeyChecking=no"
SCP="scp -o BatchMode=yes -o StrictHostKeyChecking=no -r"

echo "=== sync tools/ ==="
$SCP /c/Users/User/.kiro/forgeOS/tools/ $BOX:/root/forgeOS/tools/

echo "=== sync crates (depth pipeline + sweepscope) ==="
for c in sweepscope depthscope forge-depth forge-core forge-book forge-data forge-metrics; do
  $SCP /c/Users/User/.kiro/forgeOS/crates/$c $BOX:/root/forgeOS/crates/
done

echo "=== sync workspace root ==="
$SCP /c/Users/User/.kiro/forgeOS/Cargo.toml $BOX:/root/forgeOS/Cargo.toml
$SCP /c/Users/User/.kiro/forgeOS/Cargo.lock $BOX:/root/forgeOS/Cargo.lock

echo "=== verify on box ==="
$SSH $BOX 'ls /root/forgeOS/tools/*.py | wc -l; ls /root/forgeOS/crates/; head -5 /root/forgeOS/Cargo.toml'