#!/usr/bin/env bash
# Follow origin/main in a dedicated clone. Poll cadence: five minutes.
set -uo pipefail
C=${FRACTAL_FOLLOW_CLONE:?missing required input: FRACTAL_FOLLOW_CLONE}
LOG=${FRACTAL_FOLLOW_LOG:?missing required input: FRACTAL_FOLLOW_LOG}
mkdir -p "$(dirname "$LOG")"
log() {
  python3 - "$LOG" "$@" <<'PY'
import datetime, json, sys
path, event, before, after, install, detail = sys.argv[1:]
row = {'ts': datetime.datetime.now(datetime.timezone.utc).isoformat(),
       'event': event, 'from': before, 'to': after,
       'install': int(install), 'detail': detail}
with open(path, 'a') as stream:
    stream.write(json.dumps(row, separators=(',', ':')) + '\n')
PY
}
while :; do
  before=$(git -C "$C" rev-parse HEAD)
  if ! detail=$(timeout 120 git -C "$C" fetch -q origin main 2>&1); then
    log fetch-failed "$before" "" 0 "$detail"
  else
    after=$(git -C "$C" rev-parse origin/main)
    if ! changed=$(git -C "$C" diff --name-only "$before" "$after" -- pnpm-lock.yaml pnpm-workspace.yaml megarepo.kdl megarepo.lock pnpm-source-inputs.json 2>&1); then
      log diff-failed "$before" "$after" 0 "$changed"
    elif ! detail=$(git -C "$C" merge -q --ff-only origin/main 2>&1); then
      log ff-failed "$before" "$after" 0 "$detail"
    else
      install=0
      [[ -n "$changed" ]] && install=1
      # Check even an unchanged main: an unresolved member must be materialized.
      if ! member_rev=$(python3 - "$C" "$changed" <<'PY'
import json, pathlib, re, sys
root = pathlib.Path(sys.argv[1])
changed = sys.argv[2].splitlines()
members = json.loads((root / 'megarepo.lock').read_text())['members']
if any(p in changed for p in ('megarepo.kdl', 'megarepo.lock')) or any(
        not (root / 'repos' / name).is_dir() for name in members):
    rev = json.loads((root / 'flake.lock').read_text())['nodes']['effect-utils']['locked']['rev']
    if not re.fullmatch(r'[0-9a-f]{40}', rev):
        raise ValueError(f'effect-utils rev {rev!r} is not a commit id')
    print(rev)
PY
      ); then
        log members-input-failed "$before" "$after" "$install" "Cannot read members or the effect-utils pin"
      else
        members_ok=1
        if [[ -n "$member_rev" ]]; then
          install=1
          if ! detail=$(cd "$C" && timeout 1800 nix run "github:overengineeringstudio/effect-utils/$member_rev#megarepo" -- apply --worktree-mode commit --git-protocol https 2>&1); then
            log members-failed "$before" "$after" "$install" "$detail"
            members_ok=0
          fi
        fi
        if [[ "$members_ok" == 1 ]]; then
          if [[ "$install" == 1 ]]; then
            if detail=$(cd "$C" && CI=1 timeout 600 nix develop .#web -c pnpm install --frozen-lockfile --prefer-offline 2>&1); then
              log updated "$before" "$after" "$install" "$detail"
            else
              log install-failed "$before" "$after" "$install" "$detail"
            fi
          elif [[ "$before" != "$after" ]]; then
            log updated "$before" "$after" 0 ""
          fi
        fi
      fi
    fi
  fi
  sleep 300
done
