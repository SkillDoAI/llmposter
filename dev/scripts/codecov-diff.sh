#!/usr/bin/env bash
# codecov-diff.sh — Show coverage gaps for PR diff files
# Usage: ./dev/scripts/codecov-diff.sh [branch] [pr_number]
#
# Reads Codecov token from ~/.codecov
# Requires: curl, jq

set -euo pipefail

BRANCH="${1:-feat/v0.2.4}"
PR="${2:-48}"
TOKEN="$(cat ~/.codecov 2>/dev/null)" || { echo "Error: No ~/.codecov token found"; exit 1; }
API="https://api.codecov.io/api/v2/github/SkillDoAI/repos/llmposter"

echo "=== PR #${PR} Coverage ==="
curl -sf -H "Authorization: token ${TOKEN}" "${API}/pulls/${PR}" | \
  jq -r '"Base: \(.base_totals.coverage)%  Head: \(.head_totals.coverage)%"'

echo ""
echo "=== Files with misses (${BRANCH}) ==="
echo ""
printf "%-45s %8s %6s\n" "FILE" "COVERAGE" "MISSES"
printf "%-45s %8s %6s\n" "----" "--------" "------"

curl -sf -H "Authorization: token ${TOKEN}" "${API}/report/?branch=${BRANCH}" | \
  jq -r '
    .files[]
    | select(.totals.misses > 0)
    | "\(.name)\t\(.totals.coverage)%\t\(.totals.misses)"
  ' | sort -t$'\t' -k3 -rn | \
  while IFS=$'\t' read -r name cov misses; do
    printf "%-45s %8s %6s\n" "$name" "$cov" "$misses"
  done
