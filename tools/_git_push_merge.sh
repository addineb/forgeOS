#!/usr/bin/env bash
set -euo pipefail
cd /c/Users/User/.kiro/forgeOS
export GIT_SSH_COMMAND="ssh -o BatchMode=yes -o StrictHostKeyChecking=no"

git config user.name "addineb" 2>/dev/null || true
git config user.email "B00900250@studentmail.uws.ac.uk" 2>/dev/null || true

echo "=== commit on depth-pattern ==="
git add -A
git commit -m "$(cat <<'MSG'
chore: project cleanup — delete dead scripts, prune box junk

- Remove 33 one-off tools/ scripts (probes, checks, superseded Python analysis)
- gitignore mcps/ and .kilo/worktrees/
- Keep pipeline tools only: chd-to-parquet, enrich, stitch, batch pull, ops shells
MSG
)"

echo "=== push depth-pattern ==="
git push origin depth-pattern

echo "=== delete remote lag-subspace ==="
git push origin --delete lag-subspace 2>/dev/null || echo "lag-subspace already gone or no permission"

echo "=== merge depth-pattern into main ==="
git checkout main
git merge depth-pattern -m "merge depth-pattern: cleanup + active depth pipeline"
git push origin main

echo "=== back to depth-pattern ==="
git checkout depth-pattern

echo "=== verify ==="
echo "local main:"; git rev-parse main
echo "remote main:"; git ls-remote origin refs/heads/main
echo "remote depth-pattern:"; git ls-remote origin refs/heads/depth-pattern