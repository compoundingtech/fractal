#!/usr/bin/env bash
# Devnet supplies the edge URL and local listener port before startup.
set -euo pipefail
: "${DEV_EDGE_URL:?devnet did not supply DEV_EDGE_URL}"
: "${PORT:?devnet did not supply PORT}"
: "${WF_ST_CREDENTIAL_FILE:?set WF_ST_CREDENTIAL_FILE to the paired gateway bearer token file}"

app_dir=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
cd -- "$app_dir"
WF_EDGE_HOST=$(python3 -c 'import os, urllib.parse; print(urllib.parse.urlparse(os.environ["DEV_EDGE_URL"]).hostname)')
export WF_EDGE_HOST
export WF_ALLOWED_HOSTS="127.0.0.1,localhost,$WF_EDGE_HOST"
export WF_ST_GATEWAY=${WF_ST_GATEWAY:-unix:/run/user/$(id -u)/st3-client.sock}
WF_ST_AUTHORIZATION="Bearer $(<"$WF_ST_CREDENTIAL_FILE")"
export WF_ST_AUTHORIZATION
exec "$app_dir/node_modules/.bin/vite" --config "${WF_VITE_CONFIG:-vite.config.ts}" --configLoader runner --port "$PORT" --host 127.0.0.1 --strictPort
