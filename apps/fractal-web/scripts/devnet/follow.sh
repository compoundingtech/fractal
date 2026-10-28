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
record_key() {
  printf '%s\n' "$2" > "$1.tmp.$$" && mv "$1.tmp.$$" "$1"
}
while :; do
  before=$(git -C "$C" rev-parse HEAD)
  if ! detail=$(timeout 120 git -C "$C" fetch -q origin main 2>&1); then
    log fetch-failed "$before" "" 0 "$detail"
  else
    after=$(git -C "$C" rev-parse origin/main)
    if ! detail=$(git -C "$C" merge -q --ff-only origin/main 2>&1); then
      log ff-failed "$before" "$after" 0 "$detail"
    else
      # Successful keys survive ff-merge and process restarts; failures stay pending.
      if ! inputs=$(python3 - "$C" "$LOG" 2>&1 <<'PY'
import hashlib, json, pathlib, re, sys
root, log = pathlib.Path(sys.argv[1]), sys.argv[2]
def key(paths):
    digest = hashlib.sha256()
    for path in paths:
        digest.update(path.encode())
        digest.update((root / path).read_bytes())
    return digest.hexdigest()
def previous(suffix):
    path = pathlib.Path(log + suffix)
    return path.read_text().strip() if path.exists() else None
member_key = key(('megarepo.kdl', 'megarepo.lock'))
install_key = key(('pnpm-lock.yaml', 'pnpm-workspace.yaml', 'megarepo.kdl',
                   'megarepo.lock', 'pnpm-source-inputs.json'))
members = json.loads((root / 'megarepo.lock').read_text())['members']
rev = '-'
if previous('.members-key') != member_key or any(
        not (root / 'repos' / name).is_dir() for name in members):
    rev = json.loads((root / 'flake.lock').read_text())['nodes']['effect-utils']['locked']['rev']
    if not re.fullmatch(r'[0-9a-f]{40}', rev):
        raise ValueError(f'effect-utils rev {rev!r} is not a commit id')
install = previous('.install-key') != install_key or pathlib.Path(log + '.install-pending').exists()
print(member_key, install_key, rev, int(install))
PY
      ); then
        log members-input-failed "$before" "$after" 0 "$inputs"
      else
        read -r member_key install_key member_rev install <<< "$inputs"
        members_ok=1
        if [[ "$member_rev" != - ]]; then
          install=1
          touch "$LOG.install-pending"
          if ! detail=$(cd "$C" && timeout 1800 nix run "github:overengineeringstudio/effect-utils/$member_rev#megarepo" -- apply --worktree-mode commit --git-protocol https 2>&1); then
            log members-failed "$before" "$after" "$install" "$detail"
            members_ok=0
          else
            record_key "$LOG.members-key" "$member_key"
          fi
        fi
        if [[ "$members_ok" == 1 ]]; then
          if [[ "$install" == 1 ]]; then
            if detail=$(cd "$C" && CI=1 timeout 600 nix develop .#web -c pnpm install --frozen-lockfile --prefer-offline 2>&1); then
              record_key "$LOG.install-key" "$install_key"
              rm -f "$LOG.install-pending"
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
