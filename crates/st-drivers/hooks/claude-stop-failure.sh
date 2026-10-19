#!/usr/bin/env bash

# st2 Claude StopFailure hook: surface infrastructure wedges through presence and, when declared,
# the agent's supervisor inbox. Fail-open; lifecycle reporting must never wedge the harness.

set -u

# Older running providers retain their original environment across binary replacement.
for name in ST_CLAUDE_IDENTITY ST_CLAUDE_RUNTIME_ID ST_CLAUDE_SESSION ST_CLAUDE_SESSION_SEQ ST_CLAUDE_EXPECTED_NATIVE_SESSION ST_CLAUDE_RESUME_GENERATION; do
  legacy="ST2_${name#ST_}"
  if [[ ${!name+x} != x && ${!legacy+x} == x ]]; then
    printf -v "$name" '%s' "${!legacy}"
    export "$name"
  fi
done
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
# Identity conflicts never select either alias; diagnostics contain names only.
for fractal_legacy in ST_AGENT ST_CLAUDE_EXPECTED_NATIVE_SESSION ST_CLAUDE_IDENTITY ST_CLAUDE_RESUME_GENERATION ST_CLAUDE_RUNTIME_ID ST_CLAUDE_SESSION ST_CLAUDE_SESSION_SEQ ST_SUPERVISOR; do
  fractal_preferred=FRACTAL_${fractal_legacy#*_}
  eval 'fractal_new_set=${'"$fractal_preferred"'+x}; fractal_new=${'"$fractal_preferred"'-}; fractal_old_set=${'"$fractal_legacy"'+x}; fractal_old=${'"$fractal_legacy"'-}'
  if [ "$fractal_new_set" = x ] && [ -n "$fractal_new" ] && [ "$fractal_old_set" = x ] && [ "$fractal_new" != "$fractal_old" ]; then
    printf 'conflicting identity environment: %s and %s\n' "$fractal_preferred" "$fractal_legacy" >&2
    unset "$fractal_preferred" "$fractal_legacy"
    return 1 2>/dev/null || exit 1
  fi
done
unset fractal_legacy fractal_preferred fractal_new_set fractal_new fractal_old_set fractal_old

# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
identity="${FRACTAL_CLAUDE_IDENTITY-${ST_CLAUDE_IDENTITY:-${FRACTAL_AGENT-${ST_AGENT:-}}}}"
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
root="${FRACTAL_ROOT-${ST_ROOT:-${CATALOG:-}}}"
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
supervisor="${FRACTAL_SUPERVISOR-${ST_SUPERVISOR:-}}"
if [[ -z "$identity" || -z "$root" ]] || ! command -v st2 >/dev/null 2>&1 || ! command -v jq >/dev/null 2>&1; then
  exit 0
fi

input="$(cat)"
# Claude spells the closed error word `error` on the StopFailure payload (`error_type` is not a
# field it ever emits), and the sibling observe hook reads the same field.
error_type="$(printf '%s' "$input" | jq -r '.error // "unknown"' 2>/dev/null || printf unknown)"
status="away"
notify="yes"
case "$error_type" in
  rate_limit)
    notify=""
    ;;
  authentication_failed | oauth_org_not_allowed | billing_error)
    status="offline"
    ;;
  max_output_tokens | invalid_request | model_not_found)
    exit 0
    ;;
esac

st2 status "$identity" --root "$root" --set "$status" >/dev/null 2>&1 || true
if [[ -n "$notify" && -n "$supervisor" ]]; then
  st2 message send "$supervisor" \
    --root "$root" \
    --as "$identity" \
    --subject "agent ${identity} stopped: ${error_type}" \
    -m "Agent ${identity} ended a Claude turn with error_type=${error_type}. Status set to ${status}; inspect or nudge it when appropriate." \
    >/dev/null 2>&1 || true
fi
exit 0
