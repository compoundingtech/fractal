#!/usr/bin/env bash
# Runs one fractal command against the seeded fixture daemon in $CAPTURE_DIR. It runs in a terminal with an
# empty environment and its own HOME, so nothing from the caller's machine or agent session leaks in.
# FRACTAL is the binary to run (default target/debug/fractal). It is linked as `fractal`. The config in that HOME
# names the fixture's person, CAPTURE_PERSON (default person/robin; the `app` scenario's is person/ada), so
# `fractal now` shows what waits on them without `--as`. It sits in the config directory this build reads
# when no migration receipt exists. The terminal is 88 columns by 60 rows unless CAPTURE_COLS and
# CAPTURE_ROWS say otherwise (tui.sh runs the TUI at 100 by 32).
set -euo pipefail
: "${CAPTURE_DIR:?set CAPTURE_DIR to the directory the capture test serves from}"
binary=$(realpath "${FRACTAL:-target/debug/fractal}")
mkdir -p "$CAPTURE_DIR/bin" "$CAPTURE_DIR/cli-home/.config/st3"
printf 'person = "%s"\n' "${CAPTURE_PERSON:-person/robin}" >"$CAPTURE_DIR/cli-home/.config/st3/config.toml"
ln -sf "$binary" "$CAPTURE_DIR/bin/fractal"
stty cols "${CAPTURE_COLS:-88}" rows "${CAPTURE_ROWS:-60}"
# --endpoint goes after the command: `fractal` reads `tui` as `ui` only as its first argument.
exec env -i HOME="$CAPTURE_DIR/cli-home" PATH="$CAPTURE_DIR/bin:/usr/bin:/bin" TERM=xterm-256color LANG=C.UTF-8 \
  ST3_DAEMON_WAIT=0 fractal "$@" --endpoint "$CAPTURE_DIR/st3.sock"
