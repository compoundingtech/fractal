#!/usr/bin/env bash
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
set -euo pipefail

# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
: "${FRACTAL_MISSION_RUN-${ST_MISSION_RUN:?ST_MISSION_RUN must identify the root mission run}}"
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
root="mission-run/${FRACTAL_MISSION_RUN-${ST_MISSION_RUN-}}"
work="$(env -u FRACTAL_AGENT -u ST_AGENT st3 work ls --all --json)"
root_view="$(env -u FRACTAL_AGENT -u ST_AGENT st3 --json missions show "$root")"
producer="$(jq -r '.steps[] | select(.step == "lift-mission-document") | .subject' <<<"$root_view")"
publisher="$(jq -r '.steps[] | select(.step == "lift-mission-document/work/publish-ready-graph-mission") | .subject' <<<"$root_view")"
consumer="$(jq -r '.steps[] | select(.step == "execute-lifted-mission") | .subject' <<<"$root_view")"

for step in \
  start-planner \
  lift-mission-document \
  lift-mission-document/work/read-exact-mission-document \
  lift-mission-document/work/author-complete-graph-mission \
  lift-mission-document/work/publish-ready-graph-mission \
  execute-lifted-mission; do
  jq -e --arg run "$root" --arg step "$step" \
    'any(.steps[]; .step == $step and .status == "completed")' \
    <<<"$root_view" >/dev/null
done

output="$(st trace show "$producer" --json --limit 100 | jq -s '[.[] | select(.kind == "mission.produced")] | last')"
mission="$(jq -r '.body.fields.mission' <<<"$output")"
revision="$(jq -r '.body.fields.revision' <<<"$output")"
output_index="$(jq -r '.store_index' <<<"$output")"
test "$mission" = "mission/eval/mission-document-lift/work"
test "${#revision}" -eq 64

publisher_claim="$(st trace show "$publisher" --json --limit 100 | jq -s '[.[] | select(.kind == "work.claimed")] | last | .store_index')"
publisher_complete="$(st trace show "$publisher" --json --limit 100 | jq -s '[.[] | select(.kind == "work.submitted")] | last | .store_index')"
test "$output_index" -gt "$publisher_claim"
test "$output_index" -lt "$publisher_complete"

# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
child_runs="$(jq -r --arg root "$root" --arg agent "agent/${FRACTAL_MISSION_RUN-${ST_MISSION_RUN-}}/pdl.agent" '[.[] | select(.run != $root and .assigned_to == $agent) | .run] | unique | .[]' <<<"$work")"
test "$(grep -c . <<<"$child_runs")" -eq 1
child="$(head -n 1 <<<"$child_runs")"

created="$(st trace show "$child" --json --limit 100 | jq -s '[.[] | select(.kind == "mission-run.created")] | first')"
test "$(jq -r '.body.fields.mission' <<<"$created")" = "$mission"
test "$(jq -r '.body.fields.revision' <<<"$created")" = "$revision"
test "$(jq -r '.body.fields.root_mission_run' <<<"$created")" = "$root"
test "$(jq -r '.body.fields.parent_step_run' <<<"$created")" = "$consumer"
test "$(jq -r '.store_index' <<<"$created")" -gt "$output_index"

for step in inspect-inventory write-result verify-result publish-result; do
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
  jq -e --arg run "$child" --arg step "$step" --arg agent "agent/${FRACTAL_MISSION_RUN-${ST_MISSION_RUN-}}/pdl.agent" \
    'any(.[]; .run == $run and .step == $step and .status == "completed" and .assigned_to == $agent and (.title | length > 0) and (.goals | length > 0))' \
    <<<"$work" >/dev/null
done

published="$(st trace show mission/eval/mission-document-lift/work --json --limit 100 | jq -s --arg revision "$revision" '[.[] | select(.kind == "mission.published" and .body.revision == $revision)] | first')"
jq -e '
  .body.display_order == ["inspect-inventory", "write-result", "verify-result", "publish-result"]
  and .body.steps["write-result"].dependencies[0].step == "inspect-inventory"
  and .body.steps["verify-result"].dependencies[0].step == "write-result"
  and .body.steps["publish-result"].dependencies[0].step == "verify-result"
' <<<"$published" >/dev/null

st subject show "resource/$root/mission-result" --json \
  | jq -e '.status.subjects[0].actual | (.fields // .) | .kind == "custom.st3.document-result" and .state == "published"' >/dev/null

echo "PASS: Small Talk used the exact attempt-bound mission output after publication"
