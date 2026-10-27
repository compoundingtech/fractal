#!/usr/bin/env bash
# Prints one fractal.how TUI screen: `fractal tui` against the seeded fixture daemon in $CAPTURE_DIR, in a
# detached tmux pane 100 columns by 32 rows, as `tmux capture-pane -e` reads it once the screen settles:
# each row's text with its SGR colours. Each call starts the TUI fresh, in a HOME with no saved space, so
# it opens on Home. The arguments are tmux send-keys keys, typed after the first screen settles, each
# followed by a settle. With none, it prints that first screen.
#
#   CAPTURE_DIR=/tmp/cli-capture scripts/website-cli-capture/tui.sh > tui-now.ansi
#   CAPTURE_DIR=/tmp/cli-capture scripts/website-cli-capture/tui.sh C-k > tui-open.ansi
#
# Start the fixture daemon first, as in capture.sh. Needs tmux. CAPTURE_PERSON is the TUI's person
# (run.sh, default person/robin).
set -euo pipefail
: "${CAPTURE_DIR:?set CAPTURE_DIR to the directory the capture test serves from}"
here=$(dirname "$(realpath "$0")")
# A fresh state directory each time: the TUI restores the last space and its tabs from there.
rm -rf "$CAPTURE_DIR/cli-home/.local" "$CAPTURE_DIR/cli-home/.cache"
# Its own tmux server, on CAPTURE_TMUX_SOCKET (default $CAPTURE_DIR/tmux.sock).
tmux=(tmux -S "${CAPTURE_TMUX_SOCKET:-$CAPTURE_DIR/tmux.sock}" -f /dev/null)
"${tmux[@]}" kill-server 2>/dev/null || true
trap '"${tmux[@]}" kill-server 2>/dev/null || true' EXIT
"${tmux[@]}" new-session -d -x 100 -y 32 -c "$PWD" \
  "env CAPTURE_DIR=$(printf '%q' "$CAPTURE_DIR") CAPTURE_PERSON=$(printf '%q' "${CAPTURE_PERSON:-person/robin}") CAPTURE_COLS=100 CAPTURE_ROWS=32 $(printf '%q' "$here/run.sh") tui"
# Waits until two captures a second apart match, so live data has arrived. A screen with a spinner
# never matches: after 10 seconds it is taken as it stands.
settle() {
  local before="" now
  for _ in $(seq 10); do
    sleep 1
    now=$("${tmux[@]}" capture-pane -p -e -t 0)
    if [ "$now" = "$before" ]; then return; fi
    before=$now
  done
}
settle
for key in "$@"; do
  "${tmux[@]}" send-keys -t 0 "$key"
  settle
done
"${tmux[@]}" capture-pane -p -e -t 0
