#!/usr/bin/env bash
# Bundles the SDK, its programs and the decide() entry into one script for the embedded engines.
set -euo pipefail
cd "$(dirname "$0")/.."
node exp5-bridge/js/fixtures.ts
npx esbuild exp5-bridge/js/entry.ts --bundle --format=iife --platform=neutral --target=es2020 --minify \
  --outfile=exp5-bridge/fixtures/bundle.js
node exp5-bridge/js/vm-check.mjs
