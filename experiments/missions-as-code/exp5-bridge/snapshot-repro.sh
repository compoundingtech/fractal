#!/usr/bin/env bash
# The V8 startup snapshot failure, reduced: a bundle that runs one Effect program before the
# snapshot is taken aborts V8 when the snapshot is restored, with
#   Check failed: LinearSearch(*desc->GetKey(), descriptor_number) == InternalIndex::NotFound().
# A bundle without Effect restores fine. Needs the v8 feature built.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
dir="$(mktemp -d)"
trap 'rm -rf "$dir"' EXIT
cat > "$dir/entry.ts" <<'EOF'
import { Effect } from "effect"
;(globalThis as any).value = Effect.runSync(Effect.succeed(1))
for (const n of ["__decide", "__open", "__apply", "__decision", "__close"]) (globalThis as any)[n] = () => "x"
EOF
NODE_PATH="$here/../node_modules" npx esbuild "$dir/entry.ts" --bundle --format=iife --platform=neutral \
  --target=es2020 --minify --outfile="$dir/bundle.js" --log-level=error
"$here/target/release/mission-bridge" v8-make-snapshot "$dir"
V8_SNAPSHOT="$dir/bundle.snapshot" "$here/target/release/mission-bridge" v8-snapshot "$dir" 2>&1 >/dev/null \
  | grep -m1 -E 'Check failed|stage: memory'
