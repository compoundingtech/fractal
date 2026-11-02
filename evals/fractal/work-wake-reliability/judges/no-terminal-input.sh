#!/usr/bin/env bash
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
set -euo pipefail

# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
: "${FRACTAL_MISSION_RUN-${ST_MISSION_RUN:?ST_MISSION_RUN must identify the judged mission run}}"

if [ -s pty-audit.log ]; then
  printf 'direct terminal interaction was observed:\n' >&2
  sed -n '1,40p' pty-audit.log >&2
  exit 1
fi

# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
worker="agent/${FRACTAL_MISSION_RUN-${ST_MISSION_RUN-}}/wake.worker"
count="$(st3 trace show "$worker" --json --limit 500 \
  | jq -s '[.[] | select(.kind == "terminal.input.requested")] | length')"
test "$count" -eq 0

echo "PASS: no graph or executable terminal-input path participated"
