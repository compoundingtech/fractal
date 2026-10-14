#!/usr/bin/env bash
set -euo pipefail
test ! -e "$HOME/st-minimal-outside.txt"
grep -qi 'read-only file system\|operation not permitted\|permission denied' repo/outside.txt
