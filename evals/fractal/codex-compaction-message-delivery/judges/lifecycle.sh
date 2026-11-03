#!/usr/bin/env bash
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
set -euo pipefail

# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
: "${FRACTAL_MISSION_RUN-${ST_MISSION_RUN:?ST_MISSION_RUN must identify the eval mission run}}"
test -s controller-state.json
test -s compact-control.json

jq -e '
  .result == "passed"
  and (.messages | keys | sort) == (["anchor", "post_compact", "steer"] | sort)
  and .compaction.after > .compaction.before
  and .compaction.trigger == "manual"
' controller-state.json >/dev/null

# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
agent="agent/${FRACTAL_MISSION_RUN-${ST_MISSION_RUN-}}/compact.codex"
for stage in anchor steer post_compact; do
  subject="$(jq -er --arg stage "$stage" '.messages[$stage]' controller-state.json)"
  st3 trace show "$subject" --json --limit 100 \
    | jq -se '
        map(.kind) as $kinds
        | ($kinds | index("message.staged")) != null
        and ($kinds | index("message.delivered")) != null
        and ($kinds | index("message.read")) != null
      ' >/dev/null
done

st3 trace show "$agent" --json --limit 500 \
  | jq -se '
      any(
        .kind == "harness.usage"
        and .body.fields.semantics == "context_occupancy"
        and .body.fields.last_compaction_trigger == "manual"
      )
    ' >/dev/null

input_count="$(st3 trace show "$agent" --json --limit 500 \
  | jq -s '[.[] | select(.kind == "terminal.input.requested")] | length')"
test "$input_count" -eq 1

echo "PASS: one manual /compact separated a real working-turn steer from a fully staged, delivered, and read post-compaction message"
