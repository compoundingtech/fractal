#!/usr/bin/env bash
set -euo pipefail
test "$(cat repo/greeting.txt)" = "hello from the seat"
git -C repo/remote.git rev-parse --verify -q refs/heads/seat-branch >/dev/null
test "$(git -C repo/remote.git show seat-branch:greeting.txt)" = "hello from the seat"
