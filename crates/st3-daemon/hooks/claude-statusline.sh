#!/usr/bin/env bash
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
# st3 Claude status line: record the status-line payload (stdin JSON) in the seat's harness
# context, then hand the same bytes to the operator's own renderer. Claude's statusLine is a
# single slot, so st3 chains rather than replaces it. It never looks a program up on PATH.
# Without this seat's st3 binary it drains the payload and shows the fault as the status line.

set -u

# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
if [[ -z "${FRACTAL_BIN-${ST3_BIN:-}}" || ! -x "${FRACTAL_BIN-${ST3_BIN-}}" ]]; then
  cat >/dev/null
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
  echo "st: hooks cannot run: this seat's st3 binary is missing (ST3_BIN=${FRACTAL_BIN-${ST3_BIN:-unset}})"
  exit 0
fi

# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
exec "${FRACTAL_BIN-${ST3_BIN-}}" driver-hook claude-statusline "$@"
