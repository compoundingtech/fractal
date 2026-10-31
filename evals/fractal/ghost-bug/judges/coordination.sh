#!/usr/bin/env bash
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
set -euo pipefail

# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
: "${FRACTAL_MISSION_RUN-${ST_MISSION_RUN:?ST_MISSION_RUN must identify the judged mission run}}"
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
supervisor="agent/${FRACTAL_MISSION_RUN-${ST_MISSION_RUN-}}/gbx.sup"
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
worker="agent/${FRACTAL_MISSION_RUN-${ST_MISSION_RUN-}}/gbx.fix"

messages_to() {
  local recipient=$1
  jq -s 'add | unique_by(.subject)' \
    <(st3 conversations ls "$recipient" --json) \
    <(st3 conversations ls "$recipient" --archive --json)
}

worker_messages="$(messages_to "$worker")"
supervisor_messages="$(messages_to "$supervisor")"
requester_messages="$(messages_to person/eval-requester)"

jq -e --arg sender "$supervisor" 'any(.[]; .from == $sender)' \
  <<<"$worker_messages" >/dev/null
jq -e --arg sender "$worker" 'any(.[]; .from == $sender)' \
  <<<"$supervisor_messages" >/dev/null
jq -e --arg sender "$supervisor" 'any(.[]; .from == $sender)' \
  <<<"$requester_messages" >/dev/null

echo "PASS: delegation, worker report, and final confirmation are visible"
