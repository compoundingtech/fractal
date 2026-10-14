#!/usr/bin/env bash
# One demo of missions as code, about five minutes, on a throwaway st instance of its own.
#
#   experiments/missions-as-code/demo.sh
#
# 1. The plan of a program, read from its source before it runs (experiment 3).
# 2. The program run through st: a driver exec replays the program's history and starts each
#    step as an ordinary run (experiment 2). Agents and CI are stand-in scripts, and the 4-hour
#    review deadline passes in 24 seconds. The driver is killed with SIGKILL while the review
#    and its deadline are open; st restarts it and it carries on by replay.
# 3. The journal: every event once, in order, and no step started twice.
#
# Nothing touches your own st instance. KEEP=1 leaves the demo instance running so you can look
# at it: ST_INSTANCE=<name> st ui, or st missions ls with the same variable.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
cd "$here"
export LAB_INSTANCE="${LAB_INSTANCE:-mac-demo-$(date +%s)}"
export LAB_ST="${LAB_ST:-$(command -v st3 || command -v st || echo "$HOME/.local/bin/st3")}"
lab="$here/exp2-driver"
dir="$HOME/.st-instance/$LAB_INSTANCE"
section() { printf '\n\033[1m== %s\033[0m\n' "$1"; }

[ -d node_modules ] || npm install --silent

section "1. The plan, from the source of exp1-sdk/ship-change.ts, before anything runs"
node exp3-plan/print.ts exp1-sdk/ship-change.ts 2>/dev/null | grep -E '^\s+(op|branch|exit|loop)[0-9]+[\[\{\(]' | sed 's/<br\/>/ · /g'

section "2. A throwaway st instance: $LAB_INSTANCE ($LAB_ST)"
"$lab/lab-st" setup --person wren --node "$LAB_INSTANCE" --yes --service false --install false \
  --harness none --expert false --claude-channel false --start false >/dev/null
nice -n 10 "$lab/lab-st" up > "$dir.daemon.log" 2>&1 &
daemon=$!
stop() {
  if [ -z "${KEEP:-}" ]; then
    kill -TERM "$daemon" 2>/dev/null || true
    wait "$daemon" 2>/dev/null || true
    echo "stopped the demo daemon; its data is in $dir (remove it with rm -rf)"
  else
    echo "the demo instance is still running (pid $daemon): ST_INSTANCE=$LAB_INSTANCE $LAB_ST ui"
  fi
}
trap stop EXIT
for _ in $(seq 1 60); do "$lab/lab-st" missions ls >/dev/null 2>&1 && break; sleep 1; done
host="$(mktemp "${TMPDIR:-/tmp}/mac-demo-host.XXXXXX.kdl")"
node "$lab/host.ts" exp1-sdk/ship-change.ts --lab > "$host"
"$lab/lab-as" apply "$host" --as person/wren --no-gate-check | tail -1
rm -f "$host"
echo "published orchard/ship-change: one agentless step whose exec is the driver"

section "3. Run it, and SIGKILL the driver while the review and its 4h deadline are open"
echo "(each step is its own st run, about 12 s each in a fresh instance; the person's answer is scripted)"
LAB_INSTANCE="$LAB_INSTANCE" "$lab/lab/kill-test.sh" orchard/ship-change/demo-1
