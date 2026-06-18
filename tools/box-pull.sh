#!/usr/bin/env bash
# Pull latest main on the Hetzner box after you've pushed from laptop.
set -euo pipefail
ssh -o BatchMode=yes -o StrictHostKeyChecking=no root@167.233.57.140 \
  'cd /root/forgeOS && git pull --ff-only origin main && git log -1 --oneline'