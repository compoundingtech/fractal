#!/usr/bin/env bash
# Live hot reload in the lab instance. Two runs of v1:
#   hmr-1: moved to v2-insert while in review; the driver is then killed and must come back on v2.
#   hmr-2: asked to move to v2-insert while fixing round 1 (refused), then to v2-patched (moved).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
lab="$here/../exp2-driver"
cd "$lab"
logs="$HOME/.st-instance/${LAB_INSTANCE:-maclab}/state/st3/logs"
mission=orchard/ship-change-hmr
waitlog() { for _ in $(seq 1 600); do grep -q "$2" "$logs/exec.${1//\//.}.driver.log"* 2>/dev/null && return 0; sleep 0.5; done; echo "timed out waiting for: $2" >&2; return 1; }
finish() {
  for _ in $(seq 1 900); do
    s="$(./lab-st missions show "mission-run/$1" --json | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>console.log(JSON.parse(s).status))')"
    [ "$s" = completed ] || [ "$s" = failed ] && break
    sleep 1
  done
  echo "== $1: $s"
  cat "$logs/exec.${1//\//.}.driver.log"* 2>/dev/null | sort || true
  echo "-- reload answers"
  ./lab-st subject history "custom/mac/reload/$1" --json | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>{for(const l of s.split("\n").filter(l=>l.startsWith("{"))){const c=JSON.parse(l);if(c.kind.endsWith("answered"))console.log(c.body.fields.outcome, c.body.fields.version.slice(-12), c.body.fields.reason)}})'
}
start() {
  ./lab-as missions start "$mission" --id "$1" --workspace "$lab/lab" --as person/wren \
    --input 'program-input={"change":"orchard/feature-a","repo":"orchard/app"}' >/dev/null
  lab/answer.sh "$1" review-overdue reassign >/dev/null &
}

run1="$mission/${RUN1:-hmr-1}"
start "$run1"
waitlog "$run1" "schedule timer review-deadline"
"$here/lab-reload.sh" "$run1" "$(ls "$here"/out/v2-insert@*.mjs)"
waitlog "$run1" "moved"
pid="$(pgrep -f "^node .*exp2-driver/driver.ts" | head -1)"
echo "killing driver $pid after the move"
kill -KILL "$pid"

run2="$mission/${RUN2:-hmr-2}"
start "$run2"
waitlog "$run2" "schedule step round-1/fix"
"$here/lab-reload.sh" "$run2" "$(ls "$here"/out/v2-insert@*.mjs)"
waitlog "$run2" "refused"
"$here/lab-reload.sh" "$run2" "$(ls "$here"/out/v2-patched@*.mjs)"
waitlog "$run2" "moved"
wait
finish "$run1"
finish "$run2"
