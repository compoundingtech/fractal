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

# st2 Codex SessionStart hook: restore fresh working state, expose the unread inbox, and remind the
# model to complete its boot ritual. Fail-open: a missing dependency never prevents Codex startup.

set -u

# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
identity="${FRACTAL_AGENT-${ST_AGENT:-}}"
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
root="${FRACTAL_ROOT-${ST_ROOT:-${CATALOG:-}}}"
ritual="Run the st2 boot ritual now: set your status to available, then drain your inbox by reading, acting on, replying when useful, and archiving each handled message. Before resuming or starting work, set your status to busy; set available only when yielding or ready for new work."

if [[ -z "$identity" || -z "$root" ]] || ! command -v st2 >/dev/null 2>&1 || ! command -v jq >/dev/null 2>&1; then
  exit 0
fi

# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
stale_s="${FRACTAL_REHYDRATE_STALE_S-${ST_REHYDRATE_STALE_S:-86400}}"
context="$(st2 context read "$identity" --root "$root" --fresh-within "$stale_s" 2>/dev/null || true)"
inbox_json="$(st2 message ls "$identity" --root "$root" --json 2>/dev/null || printf '[]')"
if ! printf '%s' "$inbox_json" | jq -e 'type == "array"' >/dev/null 2>&1; then
  inbox_json='[]'
fi

context_block=""
if [[ -n "$(printf '%s' "$context" | tr -d '[:space:]')" ]]; then
  context_block="$(
    printf '<context source="st2/context/now.md" agent="%s">\n' "$identity"
    printf '%s' "$context"
    [[ "$context" == *$'\n' ]] || printf '\n'
    printf '</context>'
  )"
fi

count="$(printf '%s' "$inbox_json" | jq 'length')"
inbox_block=""
if (( count > 0 )); then
  inbox_block="$(printf '%s' "$inbox_json" | jq -r --arg count "$count" '
    "## st2 inbox (" + $count + " unread)\n" +
    (map(
      "- " + .filename
      + "  " + (.from // "unknown")
      + (if .subject != null then "  Subject: " + .subject else "" end)
    ) | join("\n"))
  ')"
fi

additional="$ritual"
if [[ -n "$context_block" ]]; then
  additional="${context_block}"$'\n\n'"${additional}"
fi
if [[ -n "$inbox_block" ]]; then
  additional="${additional}"$'\n\n'"${inbox_block}"
fi

jq -n --arg text "$additional" '{
  continue: true,
  hookSpecificOutput: {
    hookEventName: "SessionStart",
    additionalContext: $text
  }
}'
exit 0
