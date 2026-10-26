#!/bin/sh
# Writes the website CLI transcript: each command after a "$ fractal …" line, its terminal output
# byte for byte. run.sh adds --endpoint <fixture socket>; the shown command omits it.
RUN=mission-run/16e622ffd8ad8390c076f86320533345
for args in "agents ls" "missions show $RUN"; do
  printf '$ fractal %s\n' "$args"
  script -qec "/tmp/fcap/cap/run.sh $args" /dev/null
done
