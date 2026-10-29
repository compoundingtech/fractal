#!/usr/bin/env bash
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
set -euo pipefail

# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
: "${FRACTAL_MISSION_RUN-${ST_MISSION_RUN:?ST_MISSION_RUN must identify the judged mission run}}"
agents="$(env -u FRACTAL_AGENT -u ST_AGENT st3 agents --json)"
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
run="mission-run/${FRACTAL_MISSION_RUN-${ST_MISSION_RUN-}}"
mission="$(env -u FRACTAL_AGENT -u ST_AGENT st3 --json missions show "$run")"
grouping_reason="the supervisor combines the panel recommendation"

for member in fd.a fd.b fd.c; do
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
  jq -e --arg subject "agent/${FRACTAL_MISSION_RUN-${ST_MISSION_RUN-}}/$member" --arg sup "agent/${FRACTAL_MISSION_RUN-${ST_MISSION_RUN-}}/fd.sup" --arg reason "$grouping_reason" '
    [.[] | select(.subject == $subject)] as $agents
    | ($agents | length) == 1
      and ($agents[0].under == [{"agent":$sup,"reason":$reason}])
  ' <<<"$agents" >/dev/null
done

completed_steps=(
  start-team
  draft-per-human
  draft-shared
  draft-federated
  critique-per-human
  critique-shared
  critique-federated
  revise-per-human
  revise-shared
  revise-federated
  synthesize
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
proposal-a-draft vcs.commit
proposal-b-draft vcs.commit
proposal-c-draft vcs.commit
proposal-a-final vcs.commit
proposal-b-final vcs.commit
proposal-c-final vcs.commit
recommendation vcs.commit
final-report custom.st3.message-receipt
PRODUCTS

while read -r role name; do
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
  subject="resource/mission-run/${FRACTAL_MISSION_RUN-${ST_MISSION_RUN-}}/$name"
  published="$(st3 subject show "$subject" --json \
    | jq -r '.status.subjects[0].actual | (.fields // .) | .sha')"
  current="$(git -C "$CATALOG/$role" rev-parse HEAD)"
  test "$published" = "$current"
done <<'FINAL_REVISIONS'
a proposal-a-final
b proposal-b-final
c proposal-c-final
sup recommendation
FINAL_REVISIONS

echo "PASS: the graph records each panel stage, required product, and panel grouping"
