#!/usr/bin/env bash
set -euo pipefail
grep -Eq '^[0-9a-f]{40}[[:space:]]+HEAD' repo/network.txt
grep -Eq '^[0-9]+$' repo/network.txt
test -d repo/node_modules/left-pad
grep -Eq '^v?[0-9]+(\.[0-9]+){0,2}$' repo/search.txt
