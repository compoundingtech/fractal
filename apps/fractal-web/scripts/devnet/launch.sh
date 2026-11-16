#!/usr/bin/env bash
# Keep <purpose> <build-dir> for immutable-build callers; purpose is not queried.
set -euo pipefail
: "${DEV_EDGE_URL:?devnet did not supply DEV_EDGE_URL}"
: "${PORT:?devnet did not supply PORT}"
: "${WF_ST_CREDENTIAL_FILE:?set WF_ST_CREDENTIAL_FILE to the paired gateway bearer token file}"
: "${WF_ST_GATEWAY:?set WF_ST_GATEWAY to the paired gateway Unix socket address}"
: "${1:?usage: launch.sh <purpose> <build-dir>}"
build=$(realpath "${2:?usage: launch.sh <purpose> <build-dir>}")
WF_EDGE_HOST=$(python3 -c 'import os, urllib.parse; print(urllib.parse.urlparse(os.environ["DEV_EDGE_URL"]).hostname)')
export WF_EDGE_HOST
export WF_PORT=$PORT
export WF_ST_GATEWAY
export WF_ST_CREDENTIAL_FILE
exec node "$build/server/server.mjs" "$build/web"
