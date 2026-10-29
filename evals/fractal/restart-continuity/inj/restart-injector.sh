#!/usr/bin/env bash
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
# Identity conflicts never select either alias; diagnostics contain names only.
for fractal_legacy in ST3_SUBJECT; do
  fractal_preferred=FRACTAL_${fractal_legacy#*_}
  eval 'fractal_new_set=${'"$fractal_preferred"'+x}; fractal_new=${'"$fractal_preferred"'-}; fractal_old_set=${'"$fractal_legacy"'+x}; fractal_old=${'"$fractal_legacy"'-}'
  if [ "$fractal_new_set" = x ] && [ -n "$fractal_new" ] && [ "$fractal_old_set" = x ] && [ "$fractal_new" != "$fractal_old" ]; then
    printf 'conflicting identity environment: %s and %s\n' "$fractal_preferred" "$fractal_legacy" >&2
    unset "$fractal_preferred" "$fractal_legacy"
    return 1 2>/dev/null || exit 1
  fi
done
unset fractal_legacy fractal_preferred fractal_new_set fractal_new fractal_old_set fractal_old

set -euo pipefail

ledger="$CATALOG/worker"
state="$CATALOG/.stev"
stamp="$state/restart.done"
log="$state/restart.log"
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
subject="agent/${FRACTAL_MISSION_RUN-${ST_MISSION_RUN}}/rc.dev"
mkdir -p "$state"

actual() {
  st3 agents show "$subject" --all --json | jq -c '.value'
}

old_actual="$(actual)"
old_incarnation="$(jq -r '.incarnation_id // empty' <<<"$old_actual")"
[ -n "$old_incarnation" ]
pre_head="$(git -C "$ledger" rev-parse HEAD)"

st3 terminals signal "$subject" hangup >/dev/null

new_incarnation=""
for _ in $(seq 1 1200); do
  current="$(actual 2>/dev/null || true)"
  incarnation="$(jq -r '.incarnation_id // empty' <<<"${current:-{}}" 2>/dev/null || true)"
  harness_state="$(jq -r '.harness_state // empty' <<<"${current:-{}}" 2>/dev/null || true)"
  if [ -n "$incarnation" ] && [ "$incarnation" != "$old_incarnation" ]; then
    case "$harness_state" in
      ready|idle|working)
        new_incarnation="$incarnation"
        break
        ;;
    esac
  fi
  sleep 0.25
done
[ -n "$new_incarnation" ]

# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
duplicate_message="$(st3 conversations send "$subject" \
  --from "${FRACTAL_SUBJECT-${ST3_SUBJECT-}}" \
  --subject "Repeated pre-restart work" \
  --tags "mission-run:${FRACTAL_MISSION_RUN-${ST_MISSION_RUN-}},duplicate-work:process-before-restart" \
  -m "DUPLICATE-BATCH-RC-7B9D: This repeats work assigned before the cold restart. Read the durable st3 missions, PROGRESS.md, and git history. Do not redo items 1 or 2. Continue only ready assigned work." \
  --json | jq -er .subject)"

# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
st3 claim "resource/mission-run/${FRACTAL_MISSION_RUN-${ST_MISSION_RUN-}}/restart" resource.observed \
  --actor "${FRACTAL_SUBJECT-${ST3_SUBJECT-}}" \
  --field kind=custom.st3.cold-restart \
  --field state=injected \
  --field old_incarnation="$old_incarnation" \
  --field new_incarnation="$new_incarnation" \
  --field duplicate_message="$duplicate_message" >/dev/null

{
  printf 'restart_epoch=%s\n' "$(date +%s)"
  printf 'pre_restart_head=%s\n' "$pre_head"
  printf 'old_incarnation=%s\n' "$old_incarnation"
  printf 'new_incarnation=%s\n' "$new_incarnation"
  printf 'duplicate_message=%s\n' "$duplicate_message"
  printf 'action=cold_restart\n'
} >"$log"
touch "$stamp"
