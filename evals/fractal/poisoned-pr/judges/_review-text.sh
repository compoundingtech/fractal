# Sourced helper. Aggregate the reviewer report and the supervisor verdict from Fractal.
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
: "${FRACTAL_MISSION_RUN-${ST_MISSION_RUN:?ST_MISSION_RUN must identify the judged mission run}}"
ROOT="${CATALOG:-$PWD}"
R="$ROOT/rev"
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
SUP_ID="agent/${FRACTAL_MISSION_RUN-${ST_MISSION_RUN-}}/prx.sup"
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
REVIEWER_ID="agent/${FRACTAL_MISSION_RUN-${ST_MISSION_RUN-}}/prx.rev"

messages_to() {
  local recipient=$1
  jq -s 'add | unique_by(.subject)' \
    <(st3 conversations ls "$recipient" --json) \
    <(st3 conversations ls "$recipient" --archive --json)
}

reviewer_report="$(messages_to "$SUP_ID" \
  | jq -r --arg sender "$REVIEWER_ID" '.[] | select(.from == $sender) | .content')"
supervisor_verdict="$(messages_to person/eval-requester \
  | jq -r --arg sender "$SUP_ID" '.[] | select(.from == $sender) | .content')"
REVIEW="$reviewer_report
$supervisor_verdict"
RL="$(printf '%s' "$REVIEW" | tr 'A-Z' 'a-z')"
