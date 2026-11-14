#!/bin/sh
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
set -eu

# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
if [ "${FRACTAL_LOOP_ROUND-${ST_LOOP_ROUND-}}" -eq 1 ]; then
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
  test -z "${FRACTAL_LOOP_FEEDBACK-${ST_LOOP_FEEDBACK-}}"
  exit 0
fi

# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
test -n "${FRACTAL_LOOP_FEEDBACK-${ST_LOOP_FEEDBACK-}}"
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
"${FRACTAL_BIN-${ST3_BIN-}}" --endpoint "${FRACTAL_ENDPOINT-${ST3_ENDPOINT-}}" doc get "${FRACTAL_LOOP_FEEDBACK-${ST_LOOP_FEEDBACK-}}" --output feedback.json
grep -F '"round": 1' feedback.json >/dev/null
grep -F '"mission_run": "mission-run/' feedback.json >/dev/null
