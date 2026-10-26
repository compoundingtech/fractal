#!/bin/sh
# Prints the fractal.how CLI transcript: a "$ fractal …" line for each command, then its terminal output
# byte for byte (CRLF line ends and ANSI colours included). run.sh adds --endpoint, which the "$" lines omit.
#
#   nix develop --command cargo test -p fractal --features test-support --test website_cli_capture --no-run
#   nix develop --command cargo build -p fractal --bin st3
#   CAPTURE_DIR=/tmp/cli-capture <website_cli_capture test binary> --ignored --nocapture &   # until CAPTURE READY
#   CAPTURE_DIR=/tmp/cli-capture scripts/website-cli-capture/capture.sh > cli.ansi
#   touch /tmp/cli-capture/stop
set -eu
here=$(dirname "$(realpath "$0")")
# The invoice-index run: the one with a person waiting on it.
run=$(env -i HOME="$CAPTURE_DIR/cli-home" PATH=/usr/bin:/bin "$(realpath "${FRACTAL:-target/debug/st3}")" \
  --endpoint "$CAPTURE_DIR/st3.sock" --daemon-wait 0 --json missions ls |
  sed -n '/"mission\/marrowpine\/invoice-index"/,/mission-run\//p' | grep -o 'mission-run/[0-9a-f]*' | head -n 1)
for args in "agents ls" "missions show $run"; do
  printf '$ fractal %s\n' "$args"
  script -qec "$here/run.sh $args" /dev/null
done
