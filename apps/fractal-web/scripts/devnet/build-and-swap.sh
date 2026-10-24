#!/usr/bin/env bash
# Build/activate only. The integration-loop owner manages the new devnet purpose.
set -Eeuo pipefail
ROOT=${FRACTAL_BUILD_ROOT:?missing required input: FRACTAL_BUILD_ROOT}
SNAPSHOT_ROOT=${FRACTAL_BUILD_SNAPSHOT_ROOT:?missing required input: FRACTAL_BUILD_SNAPSHOT_ROOT}
script_dir=$(dirname -- "$(realpath -- "${BASH_SOURCE[0]}")")
SOURCE=${1:?Usage: build-and-swap.sh SOURCE_DIR SHA NAME}
requested=${2:?Usage: build-and-swap.sh SOURCE_DIR SHA NAME}
name=${3:?Usage: build-and-swap.sh SOURCE_DIR SHA NAME}
[[ $name =~ ^[a-zA-Z0-9][a-zA-Z0-9._-]*$ && $name != . && $name != .. ]] || { echo 'Invalid deployment name' >&2; exit 2; }
SOURCE=$(realpath "$SOURCE")
P=$ROOT/$name
started=$(date +%s%3N)
sha=$requested
outcome=failed
reason=unexpected-error
snapshot=
snapshot_lock=
build=
previous=
activated=0
mkdir -p "$P/builds"
log_run() {
  TS=$(date -u +%FT%TZ) SHA=$sha OUTCOME=$outcome REASON=$reason MS=$(($(date +%s%3N)-started)) python3 -c 'import json,os; print(json.dumps({"ts":os.environ["TS"],"sha":os.environ["SHA"],"outcome":os.environ["OUTCOME"],"reason":os.environ["REASON"],"ms":int(os.environ["MS"])}))' >> "$P/builds.jsonl"
}
finish() {
  local rc=$?
  trap - EXIT
  if [[ $outcome == failed ]]; then
    echo "$(date -u +%FT%TZ) failed sha=$sha reason=$reason exit=$rc" >&2
    if ((activated)); then
      if [[ -n $previous ]]; then
        ln -s "$previous" "$P/.current-rollback-$$"
        mv -Tf "$P/.current-rollback-$$" "$P/current"
      else rm -f "$P/current"; fi
    fi
    [[ -z $build ]] || rm -rf -- "$build"
  fi
  rm -f "$P/admitted-$$" "$P/overlay-manifest-$$"
  if [[ -n $snapshot ]]; then git -C "$SOURCE" worktree remove --force "$snapshot" || true; fi
  [[ -z $snapshot_lock ]] || rm -f -- "$snapshot_lock"
  log_run
  [[ $outcome != failed ]]
}
trap finish EXIT
exec 9>"$P/build.lock"
if ! flock -n 9; then outcome=skipped; reason=lock-held; exit 0; fi
reason=revision-unavailable
sha=$(git -C "$SOURCE" rev-parse --verify "$requested^{commit}")
if [[ -f $P/current-sha && $(<"$P/current-sha") == "$sha" && -f $P/current/build-receipt.json ]]; then outcome=skipped; reason=already-current; exit 0; fi
previous=$(readlink -f "$P/current" || true)
[[ -d $previous ]] || previous=
# Transient service snapshots share the clone's dataset so dependencies can hardlink.
snapshot_root=$SNAPSHOT_ROOT/$name
mkdir -p "$snapshot_root"
snapshot_root=$(realpath -- "$snapshot_root")
case "$snapshot_root/" in
  "$SOURCE/"*) reason=recursive-snapshot-root; echo 'Snapshot root must be outside the source clone' >&2; exit 2 ;;
esac
if [[ $(stat -c %d "$SOURCE") != $(stat -c %d "$snapshot_root") ]]; then
  reason=cross-device-snapshot-root
  echo 'Snapshot root must share the source clone filesystem' >&2
  exit 2
fi
# Snapshot leases are keyed by the shared namespace, not by the build root.
for stale in "$snapshot_root"/*; do
  [[ -d $stale && ! -L $stale && ${stale##*/} =~ ^[0-9a-f]{40}-[0-9]+$ ]] || continue
  (
    exec 7>"$stale.lock"
    if flock -n 7; then
      git -C "$SOURCE" worktree remove --force "$stale" || rm -rf -- "$stale"
      rm -f -- "$stale.lock"
    fi
  )
done
next_snapshot=$snapshot_root/$sha-$$
exec 8>"$next_snapshot.lock"
if ! flock -n 8; then outcome=skipped; reason='snapshot-lock-held'; exit 0; fi
snapshot=$next_snapshot
snapshot_lock=$snapshot.lock
build=$P/builds/$sha
if [[ -e $build ]]; then build=; reason=immutable-build-already-exists; exit 1; fi
reason='snapshot-failed'
git -C "$SOURCE" worktree add --detach "$snapshot" "$sha"
python3 "$script_dir/prepare-dependencies.py" "$SOURCE" "$snapshot" "$P/overlay-manifest-$$"
cd "$snapshot/apps/fractal-web"
identity=$snapshot/apps/fractal-web/vendor/utils/src/node/cli-build-identity.js
reason='build-failed'
# A bisect caller may reuse this tick's observed admission timeout for later candidates.
if [[ ${FRACTAL_BUILD_ADMISSION:-gate} == fallback ]]; then
  echo 'Caller reuses this tick admission timeout; building with plain nice -n 19' >&2
  nice -n 19 node private/build-live.mjs "$build" "$identity" "$sha"
  mode=caller-fallback-nice-19
elif [[ ${FRACTAL_BUILD_ADMISSION:-gate} != gate ]]; then
  reason=invalid-admission-mode
  exit 2
elif gate-slot --class gate --max-wait 600 -- nice -n 10 bash -c "touch \"\$1\"; exec node private/build-live.mjs \"\$2\" \"\$3\" \"\$4\"" -- "$P/admitted-$$" "$build" "$identity" "$sha"; then
  mode=gate-nice-10
else
  status=$?
  if [[ ! -e $P/admitted-$$ && $status != 75 ]]; then
    echo 'Gate admission unavailable within 10 minutes; building with plain nice -n 19' >&2
    nice -n 19 node private/build-live.mjs "$build" "$identity" "$sha"
    mode=fallback-nice-19
  else exit "$status"; fi
fi
cp "$P/overlay-manifest-$$" "$build/overlay-snapshot.json"
reason=receipt-invalid
python3 - "$build/build-receipt.json" "$sha" "$build/overlay-snapshot.json" <<'PY'
import json,sys
r=json.load(open(sys.argv[1])); assert r['buildIdentity']['rev']==sys.argv[2] and r['buildIdentity']['dirty'] is False and r['stackHead']==sys.argv[2]
r['dependencyFiles']=json.load(open(sys.argv[3]))['dependencyFiles']
with open(sys.argv[1],'w') as f: f.write(json.dumps(r,indent=2)+'\n')
PY
chmod +x "$build/launch.sh"
reason=bundle-invalid
python3 - "$build/web/index.html" <<'PY'
import re,sys
s=open(sys.argv[1]).read(); assert '/@vite/client' not in s and re.search(r'/assets/[^"\s]+\.js',s)
PY
reason=activation-failed
ln -s "$build" "$P/.current-$$"
mv -Tf "$P/.current-$$" "$P/current"
activated=1
printf '%s\n' "$sha" > "$P/.current-sha-$$"
mv -f "$P/.current-sha-$$" "$P/current-sha"
outcome=built
reason=$mode
# The integrator prunes only after the published edge confirms activation.
printf 'BUILD_DIR=%s\nCURRENT=%s\nSHA=%s\n' "$build" "$P/current" "$sha"
