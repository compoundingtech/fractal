#!/usr/bin/env bash
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
set -euo pipefail

# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
: "${FRACTAL_MISSION_RUN-${ST_MISSION_RUN:?ST_MISSION_RUN must identify the judged mission run}}"
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
run="mission-run/${FRACTAL_MISSION_RUN-${ST_MISSION_RUN-}}"
mission="$(env -u FRACTAL_AGENT -u ST_AGENT st3 --json missions show "$run")"

completed_steps=(
  materialize-megarepo
  start-worker
  repair-feature-worktree
  repair-feature-worktree/work/resolve-checkout
  repair-feature-worktree/work/reproduce-failure
  repair-feature-worktree/work/fix-root-cause
  repair-feature-worktree/work/verify-complete-suite
  repair-feature-worktree/work/publish-feature-revision
  repair-feature-worktree/work/report-requester
)

for step in "${completed_steps[@]}"; do
  count="$(jq --arg run "$run" --arg step "$step" \
    '[.steps[] | select(.step == $step and .status == "completed")] | length' \
    <<<"$mission")"
  test "$count" -eq 1
done

while read -r name kind; do
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
  subject="resource/mission-run/${FRACTAL_MISSION_RUN-${ST_MISSION_RUN-}}/$name"
  status="$(st3 subject show "$subject" --json)"
  jq -e --arg kind "$kind" '
    .status.subjects[0].actual | (.fields // .)
      | (.kind == $kind) and (.state == "published")' \
    <<<"$status" >/dev/null
  bindings="$(st3 trace "$subject" --json --limit 20 \
    | jq -s '[.[] | select(.kind == "resource.observed")] | length')"
  test "$bindings" -ge 1
done <<'PRODUCTS'
feature-revision vcs.commit
final-report custom.st3.message-receipt
PRODUCTS

# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
published="$(st3 subject show "resource/mission-run/${FRACTAL_MISSION_RUN-${ST_MISSION_RUN-}}/feature-revision" --json \
  | jq -r '.status.subjects[0].actual | (.fields // .) | .sha')"
current="$(git -C "$CATALOG/wt/feature" rev-parse HEAD)"
test "$published" = "$current"

echo "PASS: the graph records the complete Weird Git Setup mission and products"
