#!/usr/bin/env bash
# Build on a developer Mac; run the resulting directory only in an authorized GUI session.
set -euo pipefail
command=${1:?usage: probe-panel-mac.sh build|run ABSOLUTE_STAGE_DIRECTORY}
stage=${2:?absolute staging directory required}
[[ $stage == /* ]] || { echo 'stage must be absolute' >&2; exit 2; }
if [[ $command == build ]]; then
  root=$(git -C "$(dirname "$0")" rev-parse --show-toplevel)
  package="$root/apps/macos/Fractal"
  swift build --package-path "$package" --product Fractal -j 4
  mkdir -p "$stage/Fractal.app/Contents/MacOS" "$stage/Fractal.app/Contents/Resources"
  cp "$package/.build/debug/Fractal" "$stage/Fractal.app/Contents/MacOS/Fractal"
  for bundle in "$package/.build/debug/"*.bundle; do
    [[ ! -d $bundle ]] || cp -R "$bundle" "$stage/Fractal.app/Contents/Resources/"
  done
  cat >"$stage/Fractal.app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict><key>CFBundleExecutable</key><string>Fractal</string><key>CFBundleIdentifier</key><string>tech.compounding.fractal.mac</string><key>CFBundleName</key><string>Fractal</string><key>CFBundlePackageType</key><string>APPL</string><key>LSUIElement</key><true/><key>NSHighResolutionCapable</key><true/></dict></plist>
PLIST
  codesign --force --deep --sign - "$stage/Fractal.app"
  swiftc "$package/scripts/native-panel-fixture.swift" -o "$stage/native-panel-fixture"
  cp -R "$root/docs/st3/client-v0/fixtures" "$stage/"
  cp "$package/scripts/probe-panel-mac.sh" "$stage/run.sh"
  git -C "$root" rev-parse HEAD >"$stage/source-revision"
elif [[ $command == run ]]; then
  cd "$stage"
  ./native-panel-fixture "$stage" >member.log 2>&1 & member=$!
  trap 'kill "$member" 2>/dev/null || true' EXIT
  : >frames.jsonl
  open -W -n --stdout "$stage/app.log" --stderr "$stage/frames.jsonl" \
    --env PANEL_NATIVE_PROBE=1 --env FRACTAL_SOCKET="$stage/member.sock" \
    --env FRACTAL_PERSON=person/probe --env FRACTAL_URL=http://localhost Fractal.app
  # LaunchServices does not propagate the app's exit code. The in-process oracle is authoritative.
  result=$(/usr/bin/grep '"event":"probe.result"' frames.jsonl)
  /usr/bin/plutil -extract pass raw -o - -- /dev/stdin <<<"$result" | /usr/bin/grep -qx true
else
  echo "unknown command: $command" >&2; exit 2
fi
