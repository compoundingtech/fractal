#!/usr/bin/env bash
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
set -euo pipefail

# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
: "${FRACTAL_MISSION_RUN-${ST_MISSION_RUN:?ST_MISSION_RUN must identify the judged mission run}}"

# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
dev_agent="agent/${FRACTAL_MISSION_RUN-${ST_MISSION_RUN-}}/rc.dev"
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
sup_agent="agent/${FRACTAL_MISSION_RUN-${ST_MISSION_RUN-}}/rc.sup"
dev_messages="$(st3 conversations ls "$dev_agent" --archive --json)"
sup_messages="$(st3 conversations ls "$sup_agent" --archive --json)"
requester_messages="$(st3 conversations ls person/eval-requester --archive --json)"
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
work_tag="mission-run:mission-run/${FRACTAL_MISSION_RUN-${ST_MISSION_RUN-}}"
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
direct_tag="mission-run:${FRACTAL_MISSION_RUN-${ST_MISSION_RUN-}}"
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
injector="exec/${FRACTAL_MISSION_RUN-${ST_MISSION_RUN-}}/eval/restart-continuity/inject"

assignments="$(jq --arg tag "$work_tag" \
  '[.[] | select(.from == "daemon/runtime" and (.tags | index($tag)))]' <<<"$dev_messages")"
duplicates="$(jq --arg tag "$direct_tag" --arg injector "$injector" \
  '[.[] | select(
    .from == $injector
    and (.tags | index($tag))
    and (.tags | index("duplicate-work:process-before-restart"))
    and (.content | contains("DUPLICATE-BATCH-RC-7B9D"))
  )]' <<<"$dev_messages")"
reports="$(jq --arg tag "$direct_tag" --arg dev "$dev_agent" \
  '[.[] | select(.from == $dev and (.tags | index($tag)))]' <<<"$sup_messages")"
confirmations="$(jq --arg tag "$direct_tag" --arg sup "$sup_agent" \
  '[.[] | select(.from == $sup and (.tags | index($tag)))]' <<<"$requester_messages")"

test "$(jq 'length' <<<"$assignments")" -eq 2
test "$(jq '[.[] | select(.status == "closed")] | length' <<<"$assignments")" -eq 2
test "$(jq 'length' <<<"$duplicates")" -eq 1
test "$(jq -r '.[0].status' <<<"$duplicates")" = closed
test "$(jq 'length' <<<"$reports")" -eq 1
test "$(jq -r '.[0].status' <<<"$reports")" = closed
test "$(jq 'length' <<<"$confirmations")" -eq 1

duplicate_subject="$(jq -r '.[0].subject' <<<"$duplicates")"
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
restart_subject="$(st3 subject show "resource/mission-run/${FRACTAL_MISSION_RUN-${ST_MISSION_RUN-}}/restart" --json \
  | jq -r '.status.subjects[0].actual | (.fields // .) | .duplicate_message')"
test "$duplicate_subject" = "$restart_subject"

report_index="$(jq -r '.[0].created_index' <<<"$reports")"
confirmation_index="$(jq -r '.[0].created_index' <<<"$confirmations")"
test "$confirmation_index" -gt "$report_index"

echo "PASS: Fractal has two assignments, one closed duplicate, one report, and one later confirmation"
