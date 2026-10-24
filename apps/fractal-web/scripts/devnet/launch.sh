#!/usr/bin/env bash
# Keep <purpose> <build-dir> for immutable-build callers; purpose is not queried.
set -euo pipefail
: "${DEV_EDGE_URL:?devnet did not supply DEV_EDGE_URL}"
: "${PORT:?devnet did not supply PORT}"
: "${WF_ST_CREDENTIAL_FILE:?set WF_ST_CREDENTIAL_FILE to the paired gateway bearer token file}"
: "${1:?usage: launch.sh <purpose> <build-dir>}"
build=$(realpath "${2:?usage: launch.sh <purpose> <build-dir>}")
WF_EDGE_HOST=$(python3 -c 'import os, urllib.parse; print(urllib.parse.urlparse(os.environ["DEV_EDGE_URL"]).hostname)')
export WF_EDGE_HOST
export WF_PORT=$PORT
export WF_ST_GATEWAY=${WF_ST_GATEWAY:-unix:/run/user/$(id -u)/st3-client.sock}
export WF_ST_CREDENTIAL_FILE
exec node "$build/server/server.mjs" "$build/web"
