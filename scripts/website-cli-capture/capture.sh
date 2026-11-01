#!/usr/bin/env bash
# Prints the fractal.how CLI transcript: a "$ fractal …" line for each command, then its terminal output
# byte for byte (CRLF line ends and ANSI colours included). run.sh adds --endpoint, which the "$" lines omit.
#
#   nix develop --command cargo test -p fractal --features test-support --test website_cli_capture --no-run
#   nix develop --command cargo build -p fractal --bin fractal
#   CAPTURE_DIR=/tmp/cli-capture <website_cli_capture test binary> --ignored --nocapture &   # until CAPTURE READY
#   CAPTURE_DIR=/tmp/cli-capture scripts/website-cli-capture/capture.sh > cli.ansi
#   touch /tmp/cli-capture/stop
set -euo pipefail
: "${CAPTURE_DIR:?set CAPTURE_DIR to the directory the capture test serves from}"
here=$(dirname "$(realpath "$0")")
# The invoice-index run: the one with a person waiting on it. `sed -n 1p` reads all input, so no SIGPIPE.
run=$(env -i HOME="$CAPTURE_DIR/cli-home" PATH=/usr/bin:/bin "$(realpath "${FRACTAL:-target/debug/fractal}")" \
  --endpoint "$CAPTURE_DIR/st3.sock" --daemon-wait 0 --json missions ls |
  sed -n '/"mission\/marrowpine\/invoice-index"/,/mission-run\//p' | grep -o 'mission-run/[0-9a-f]*' | sed -n 1p)
# Run in a pseudo-terminal so the CLI colours its output. util-linux script takes a shell command string,
# BSD/macOS script takes the file and then the command's argv. With its stdin at EOF, macOS script first
# writes "^D" and two backspaces. The sed drops them.
in_terminal() {
  if [ "$(uname)" = Linux ]; then
    script -qec "$(printf '%q ' "$@")" /dev/null
  else
    script -q /dev/null "$@" </dev/null | LC_ALL=C sed "1s/^\^D$(printf '\b\b')//"
  fi
}
for args in "now" "agents ls" "missions show $run"; do
  printf '$ fractal %s\n' "$args"
  read -ra words <<<"$args"
  in_terminal "$here/run.sh" "${words[@]}"
done
