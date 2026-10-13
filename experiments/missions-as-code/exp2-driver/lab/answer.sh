#!/usr/bin/env bash
# Lab stand-in for person/wren: waits for the ask op's child run and answers it.
# Usage: lab/answer.sh RUN_ID OP ANSWER
set -euo pipefail
cd "$(dirname "$0")/.."
run="$1"; op="$2"; answer="$3"
for _ in $(seq 1 600); do
  step="$(./lab-st missions show "mission-run/$run/op/$op" --json 2>/dev/null \
    | node -e 'let s="";process.stdin.on("data",d=>s+=d).on("end",()=>{const j=JSON.parse(s);const st=j.steps?.[0];if(st&&st.status==="ready")console.log(st.subject)})' 2>/dev/null || true)"
  if [ -n "$step" ]; then
    ./lab-as work done "$step" --as person/wren --summary "$answer" >/dev/null
    echo "answered $op on $run: $answer"
    exit 0
  fi
  sleep 1
done
echo "no ask $op appeared on $run" >&2
exit 1
