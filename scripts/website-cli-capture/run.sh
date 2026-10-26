#!/usr/bin/env bash
# Runs one fractal command against the seeded fixture daemon in $CAPTURE_DIR. It runs in a terminal with an
# empty environment and its own HOME, so nothing from the caller's machine or agent session leaks in.
# FRACTAL is the binary to run (default target/debug/st3). It is linked as `fractal`.
set -euo pipefail
: "${CAPTURE_DIR:?set CAPTURE_DIR to the directory the capture test serves from}"
binary=$(realpath "${FRACTAL:-target/debug/st3}")
mkdir -p "$CAPTURE_DIR/bin" "$CAPTURE_DIR/cli-home"
ln -sf "$binary" "$CAPTURE_DIR/bin/fractal"
stty cols 88 rows 60
exec env -i HOME="$CAPTURE_DIR/cli-home" PATH="$CAPTURE_DIR/bin:/usr/bin:/bin" TERM=xterm-256color LANG=C.UTF-8 \
  ST3_DAEMON_WAIT=0 fractal --endpoint "$CAPTURE_DIR/st3.sock" "$@"
