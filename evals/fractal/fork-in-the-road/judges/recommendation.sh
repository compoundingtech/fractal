#!/usr/bin/env bash
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
set -euo pipefail

# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
: "${FRACTAL_MISSION_RUN-${ST_MISSION_RUN:?ST_MISSION_RUN must identify the judged mission run}}"
requester_inbox="$(st3 conversations ls person/eval-requester --json)"
requester_archive="$(st3 conversations ls person/eval-requester --archive --json)"
requester_messages="$(jq -s 'add | unique_by(.subject)' \
  <(printf '%s' "$requester_inbox") \
  <(printf '%s' "$requester_archive"))"

# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
jq -e \
  --arg sender "agent/${FRACTAL_MISSION_RUN-${ST_MISSION_RUN-}}/fd.sup" \
  --arg run_tag "mission-run:${FRACTAL_MISSION_RUN-${ST_MISSION_RUN-}}" \
  '[.[] | select(.from == $sender and (.tags | index($run_tag)))] | length == 1' \
  <<<"$requester_messages" >/dev/null

echo "PASS: the supervisor sent one recommendation to the requester"
