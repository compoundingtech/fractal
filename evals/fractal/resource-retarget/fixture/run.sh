#!/usr/bin/env bash
set -euo pipefail

old="$(fractal resource add work://eval/target-a --as rr.worker --relation assignment)"
printf '1\tgrant\tresource/%s\n' "$old" >events.tsv
fractal resource remove "$old" --as rr.worker >/dev/null
printf '2\trevoke\tresource/%s\n' "$old" >>events.tsv
new="$(fractal resource add work://eval/target-b --as rr.worker --relation assignment)"
printf '3\tgrant\tresource/%s\n' "$new" >>events.tsv
fractal resource remove "$new" --as rr.worker >/dev/null
printf '4\tidle\tresource/%s\n' "$new" >>events.tsv
