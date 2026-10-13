#!/usr/bin/env bash
# Kills the driver with SIGKILL while ops are open, then checks the run finished by replay:
# the same journal, one child run per op, and no op enacted twice.
# Usage: [MISSION=...] [NO_KILL=1] lab/kill-test.sh RUN_ID [KILL_AFTER_LOG_LINE]
set -euo pipefail
cd "$(dirname "$0")/.."
run="$1"; marker="${2:-schedule timer review-deadline}"
log="$HOME/.st-instance/maclab/state/st3/logs/exec.${run//\//.}.driver.log"
./lab-as missions start "${MISSION:-orchard/ship-change}" --id "$run" --workspace "$PWD/lab" \
  --input 'program-input={"change":"orchard/feature-a","repo":"orchard/app"}' --as person/wren >/dev/null
lab/answer.sh "$run" review-overdue reassign &
for _ in $(seq 1 300); do grep -q "$marker" "$log" 2>/dev/null && break; sleep 0.5; done
if [ -z "${NO_KILL:-}" ]; then
  pid="$(pgrep -f "^node .*exp2-driver/driver.ts" | head -1)"
  echo "killing driver $pid after: $marker"
  kill -KILL "$pid"
fi
for _ in $(seq 1 600); do
  state="$(./lab-st missions show "mission-run/$run" --json | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>console.log(JSON.parse(s).status))')"
  [ "$state" = completed ] || [ "$state" = failed ] && break
  sleep 1
done
wait
echo "run $run: $state"
echo "--- driver log"
cat "$log"
echo "--- journal"
./lab-st subject history "custom/mac/journal/$run" --limit 500 --json | node -e '
  let s=""; process.stdin.on("data",d=>s+=d).on("end",()=>{
    const claims = s.split("\n").filter(l=>l.startsWith("{")).map(l=>JSON.parse(l))
    const events = claims.map(c=>c.body.fields).sort((a,b)=>a.seq-b.seq)
    const scheduled = events.filter(e=>e.event.type==="scheduled").map(e=>e.event.op)
    const dupes = scheduled.filter((op,i)=>scheduled.indexOf(op)!==i)
    console.log(`${claims.length} journal claims, ${events.length} events, seqs contiguous: ${events.every((e,i)=>e.seq===i)}`)
    console.log(`ops scheduled: ${scheduled.join(", ")}`)
    console.log(`ops scheduled twice: ${dupes.length ? dupes.join(", ") : "none"}`)
  })'
echo "--- child runs"
./lab-st missions ls --all --limit 200 2>/dev/null | grep -c "mission-run/$run/op/" || true
