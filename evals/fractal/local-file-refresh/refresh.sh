#!/bin/sh
set -eu

fractal trace wait resource/local-file-refresh/file --for ready --timeout 30s >/dev/null
printf '%s\n' 'changed content stays local' > watched.txt
fractal --json resource refresh resource/local-file-refresh/file \
  --timeout 30s \
  --as person/eval-requester > refresh.json
