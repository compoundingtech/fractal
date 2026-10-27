#!/bin/bash
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
# Build StListen.app (st-listen, the terminal client's speech helper) into OUT (default ./build), signed with
# ST_MACOS_SIGNING_IDENTITY or ad hoc. macOS 26 or later, with Xcode's Swift.
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
out=${1:-$here/build}
app=$out/StListen.app
rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp "$here/Info.plist" "$app/Contents/Info.plist"
xcrun swiftc -O -target "$(uname -m)-apple-macos26.0" -o "$app/Contents/MacOS/st-listen" "$here/main.swift"
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
codesign --force --sign "${FRACTAL_MACOS_SIGNING_IDENTITY-${ST_MACOS_SIGNING_IDENTITY:--}}" --identifier com.compoundingtech.smalltalk.listen \
  --options runtime --entitlements "$here/listen.entitlements" --timestamp=none "$app"
echo "$app"
