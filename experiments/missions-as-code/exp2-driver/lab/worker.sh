#!/usr/bin/env bash
# Lab stand-in for a seat or an external check: sleeps, then exits as the scenario says.
# scenario.json maps an op ID to {"sleep": seconds, "exit": code}; the default is 3s, exit 0.
op="$1"
here="$(cd "$(dirname "$0")" && pwd)"
read -r secs code < <(node -e '
  const s = JSON.parse(require("fs").readFileSync(process.argv[1], "utf8"))[process.argv[2]] ?? {}
  console.log(`${s.sleep ?? 3} ${s.exit ?? 0}`)' "$here/scenario.json" "$op")
echo "lab worker $op: sleeping ${secs}s, then exit $code"
sleep "$secs"
exit "$code"
