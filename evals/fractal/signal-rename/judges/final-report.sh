#!/usr/bin/env bash
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
set -euo pipefail

# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
: "${FRACTAL_MISSION_RUN-${ST_MISSION_RUN:?ST_MISSION_RUN must identify the judged mission run}}"

# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
messages="$("${FRACTAL_BIN-${ST3_BIN-}}" message ls "agent/${FRACTAL_MISSION_RUN-${ST_MISSION_RUN-}}/sig.base" --archive --from "agent/${FRACTAL_MISSION_RUN-${ST_MISSION_RUN-}}/sig.sup" --json)"
matching="$(
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
  jq \
    --arg tag "mission-run:${FRACTAL_MISSION_RUN-${ST_MISSION_RUN-}}" \
    '[.[] | select(.tags | index($tag))]' \
    <<<"$messages"
)"

count="$(jq 'length' <<<"$matching")"
[ "$count" -eq 1 ] || {
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
  echo "FAIL: expected one final report for mission run ${FRACTAL_MISSION_RUN-${ST_MISSION_RUN-}}, found $count"
  exit 1
}

body="$(jq -r '.[0].content' <<<"$matching")"
grep -qi 'beacon' <<<"$body"
grep -qiE 'commit|revision' <<<"$body"
grep -qiE 'test|green|pass' <<<"$body"

echo "PASS: the base owner received one tagged final report with revision and test evidence"
