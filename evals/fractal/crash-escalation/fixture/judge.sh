#!/usr/bin/env bash
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
set -euo pipefail

# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
crash="exec/${FRACTAL_MISSION_RUN-${ST_MISSION_RUN-}}/eval/crash-escalation/crash"
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
clean="exec/${FRACTAL_MISSION_RUN-${ST_MISSION_RUN-}}/eval/crash-escalation/clean"
fractal trace "$crash" --json --limit 30 | jq -s -e '[.[] | select(.kind == "runtime.reconcile-decision") | .body.fields.decision] | index("raise") != null' >/dev/null
fractal trace "$clean" --json --limit 30 | jq -s -e '[.[] | select(.kind == "runtime.reconcile-decision") | .body.fields.decision] | index("raise") == null' >/dev/null
