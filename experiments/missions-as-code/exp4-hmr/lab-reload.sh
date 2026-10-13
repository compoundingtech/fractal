#!/usr/bin/env bash
# Lab: store a version bundle as a document and ask a running program to move to it.
# Usage: exp4-hmr/lab-reload.sh RUN_ID BUNDLE_FILE
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
lab="$here/../exp2-driver"
run="$1"; file="$(cd "$(dirname "$2")" && pwd)/$(basename "$2")"
put="$("$lab/lab-as" documents put "$file" --as doc/mac/programs/orchard-ship-change --json)"
ref="$(node -e 'const t=process.argv[1]; const j=JSON.parse(t.slice(t.indexOf("{"))); console.log(`${j.name}@${j.hash}`)' "$put")"
"$lab/lab-as" claim "custom/mac/reload/$run" custom.mac.reload.requested --actor person/wren \
  --field "version=$ref" >/dev/null
echo "asked $run to move to $ref"
