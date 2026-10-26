#!/bin/sh
# Runs one fractal command against the seeded fixture daemon, in a terminal, with nothing from this seat.
stty cols 88 rows 60
exec env -i HOME=/tmp/fcap/cap/home PATH=/tmp/fcap/cap/bin:/usr/bin:/bin TERM=xterm-256color LANG=C.UTF-8 \
  ST3_DAEMON_WAIT=0 fractal --endpoint /tmp/fcap/run/st3.sock "$@"
