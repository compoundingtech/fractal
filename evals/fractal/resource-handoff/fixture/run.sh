#!/usr/bin/env bash
set -euo pipefail

uri="work://eval/shared-handoff"
ref="$(fractal resource add "$uri" --as rh.a --relation assignment)"
printf '1\trh.a\tactive\tresource/%s\n' "$ref" >events.tsv
fractal resource remove "$ref" --as rh.a >/dev/null
printf '2\trh.a\trevoked\tresource/%s\n' "$ref" >>events.tsv
same="$(fractal resource add "$uri" --as rh.b --relation assignment)"
test "$same" = "$ref"
printf '3\trh.b\tactive\tresource/%s\n' "$same" >>events.tsv
