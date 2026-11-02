#!/usr/bin/env bash
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
# Identity conflicts never select either alias; diagnostics contain names only.
for fractal_legacy in ST_AGENT; do
  fractal_preferred=FRACTAL_${fractal_legacy#*_}
  eval 'fractal_new_set=${'"$fractal_preferred"'+x}; fractal_new=${'"$fractal_preferred"'-}; fractal_old_set=${'"$fractal_legacy"'+x}; fractal_old=${'"$fractal_legacy"'-}'
  if [ "$fractal_new_set" = x ] && [ -n "$fractal_new" ] && [ "$fractal_old_set" = x ] && [ "$fractal_new" != "$fractal_old" ]; then
    printf 'conflicting identity environment: %s and %s\n' "$fractal_preferred" "$fractal_legacy" >&2
    unset "$fractal_preferred" "$fractal_legacy"
    return 1 2>/dev/null || exit 1
  fi
done
unset fractal_legacy fractal_preferred fractal_new_set fractal_new fractal_old_set fractal_old

# st2 Codex PreCompact hook. Never block compaction. If the agent has not captured any durable
# working state, write a reconstruction stub; never overwrite non-whitespace state.

set -u

# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
identity="${FRACTAL_AGENT-${ST_AGENT:-}}"
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
root="${FRACTAL_ROOT-${ST_ROOT:-${CATALOG:-}}}"
if [[ -z "$identity" || -z "$root" ]] || ! command -v st2 >/dev/null 2>&1; then
  exit 0
fi

existing="$(st2 context read "$identity" --root "$root" 2>/dev/null || true)"
if [[ -n "$(printf '%s' "$existing" | tr -d '[:space:]')" ]]; then
  exit 0
fi

timestamp="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
stub="# now — pre-compact stub — ${timestamp}

PreCompact fired before the model captured durable working state. Reconstruct from git status,
recent commits, and the st2 inbox, then write a real checkpoint with \`st2 context write\`."

# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
timeout_s="${FRACTAL_PRECOMPACT_TIMEOUT_S-${ST_PRECOMPACT_TIMEOUT_S:-5}}"
if command -v timeout >/dev/null 2>&1; then
  printf '%s\n' "$stub" | timeout "${timeout_s}s" st2 context write "$identity" --root "$root" >/dev/null 2>&1 || true
elif command -v gtimeout >/dev/null 2>&1; then
  printf '%s\n' "$stub" | gtimeout "${timeout_s}s" st2 context write "$identity" --root "$root" >/dev/null 2>&1 || true
else
  printf '%s\n' "$stub" | st2 context write "$identity" --root "$root" >/dev/null 2>&1 || true
fi

exit 0
