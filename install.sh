#!/bin/sh
# Download a verified Fractal release, install it, and open st.
set -eu
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
base=${FRACTAL_INSTALL_RELEASE_BASE_URL-${ST_INSTALL_RELEASE_BASE_URL:-https://github.com/compoundingtech/smalltalk/releases}}
tag=
edge=false
bin_dir=${HOME:?HOME is required}/.local/bin
run=true
while [ "$#" -gt 0 ]; do
    case "$1" in
        --tag) tag=${2:?--tag needs a release tag}; shift 2 ;;
        --edge) edge=true; shift ;;
        --bin-dir) bin_dir=${2:?--bin-dir needs a directory}; shift 2 ;;
        --no-run) run=false; shift ;;
        -h|--help) printf 'Usage: install.sh [--tag TAG | --edge] [--bin-dir DIRECTORY] [--no-run]\n'; exit 0 ;;
        *) printf 'install: unknown option: %s\n' "$1" >&2; exit 2 ;;
    esac
done
case "$(uname -s):$(uname -m)" in
    Linux:x86_64) target=x86_64-unknown-linux-gnu ;;
    Darwin:arm64) target=aarch64-apple-darwin ;;
    *) printf 'install: supported platforms are Linux x86_64 and Apple Silicon macOS\n' >&2; exit 1 ;;
esac
# --edge selects the rolling pre-release built from the newest green main commit.
if "$edge"; then
    [ -z "$tag" ] || { printf 'install: choose either --edge or --tag\n' >&2; exit 2; }
    tag=edge
fi
case "$tag" in *[!A-Za-z0-9._-]*) printf 'install: invalid release tag\n' >&2; exit 2 ;; esac
case "$base" in
    https://*) protocol=https ;;
    http://127.0.0.1:*|http://localhost:*) protocol=https,http ;;
    *) printf 'install: release downloads need HTTPS\n' >&2; exit 2 ;;
esac
# Pin latest before downloading either asset, so the receipt and checksum name
# the same release even when a new daily release is published during installation.
if [ -z "$tag" ]; then
    release_url=$(curl --fail --silent --show-error --location --proto "=$protocol" --proto-redir "=$protocol" \
        --write-out '%{url_effective}' --output /dev/null "$base/latest")
    tag=${release_url##*/tag/}
    case "$tag" in ''|*[!A-Za-z0-9._-]*) printf 'install: cannot resolve latest release tag\n' >&2; exit 1 ;; esac
fi
channel=download/$tag
package=fractal-$target
archive=$package.tar.gz
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
trap 'exit 1' HUP INT TERM
download() {
    curl --fail --silent --show-error --location --proto "=$protocol" --proto-redir "=$protocol" \
        "$base/$channel/$1" --output "$work/$1"
}
# arn:lmig:smalltalk:2026-10-20-release-artifact-aliases — sunset 2026-11-01 (#3032).
# Retained tags predate the Fractal asset name. Only a missing asset selects the
# legacy name; network failures and integrity failures must not trigger fallback.
status=$(curl --silent --show-error --location --proto "=$protocol" --proto-redir "=$protocol" \
    --write-out '%{http_code}' "$base/$channel/$archive" --output "$work/$archive")
case "$status" in
    200) ;;
    404) package=smalltalk-$target; archive=$package.tar.gz; download "$archive" ;;
    *) printf 'install: archive download failed (HTTP %s)\n' "$status" >&2; exit 1 ;;
esac
download "$archive.sha256"
read -r expected filename extra < "$work/$archive.sha256"
case "$expected" in ''|*[!0-9a-fA-F]*) printf 'install: invalid archive checksum\n' >&2; exit 1 ;; esac
[ "${#expected}" -eq 64 ] && [ "${filename#\*}" = "$archive" ] && [ -z "$extra" ] || {
    printf 'install: invalid archive checksum\n' >&2; exit 1;
}
if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$work/$archive")
elif command -v shasum >/dev/null 2>&1; then
    actual=$(shasum -a 256 "$work/$archive")
else
    printf 'install: sha256sum or shasum is required\n' >&2; exit 1
fi
actual=${actual%% *}
expected=$(printf '%s' "$expected" | tr A-F a-f)
[ "$actual" = "$expected" ] || { printf 'install: archive checksum mismatch\n' >&2; exit 1; }
tar -tzf "$work/$archive" > "$work/members"
while IFS= read -r member; do
    case "$member" in
        "$package"|"$package/"*) ;;
        *) printf 'install: archive contains a path outside its package\n' >&2; exit 1 ;;
    esac
    case "/$member/" in */../*) printf 'install: archive contains a parent path\n' >&2; exit 1 ;; esac
done < "$work/members"
tar -xzf "$work/$archive" -C "$work"
# Older retained releases predate --quiet; keep their installer spelling and
# suppress notices here while preserving failures and stderr.
# Current archive installers add fractal only when neither the destination nor
# PATH contains a foreign fractal command; keep their collision guidance on stderr.
"$work/$package/install.sh" --bin-dir "$bin_dir" >/dev/null
bin_dir=$(cd "$bin_dir" && pwd)
# A release receipt is optional display metadata. Failure must never prevent a
# successfully installed executable from launching.
write_receipt() (
    receipt_stage=
    trap '[ -z "$receipt_stage" ] || rm -f "$receipt_stage"' EXIT
    version=$("$work/$package/bin/st3" --version --json) || return 1
    case "$version" in '{"machine_version":"'*'"}') ;; *) return 1 ;; esac
    version=${version#'{"machine_version":"'}
    version=${version%'"}'}
    commit=${version##*+}
    case "$commit" in ''|*[!0-9a-f]*) return 1 ;; esac
    [ "${#commit}" -ge 7 ] || return 1
    receipt="$bin_dir/RELEASE.json"
    platform=$(uname -s) || return 1
    if [ "$platform" = Darwin ]; then
        # Keep the receipt outside the signed application bundle.
        receipt=$(python3 - "$bin_dir/st3" <<'PY'
from pathlib import Path
import sys
binary = Path(sys.argv[1]).resolve()
print(str(binary.parents[2]) + '.RELEASE.json')
PY
) || return 1
    fi
    installed_at=$(date -u +%Y-%m-%dT%H:%M:%SZ) || return 1
    receipt_stage=$(mktemp "$receipt.XXXXXX") || return 1
    printf '{"tag":"%s","commit":"%s","target":"%s","installed_at":"%s"}\n' \
        "$tag" "$commit" "$target" "$installed_at" > "$receipt_stage" || return 1
    mv -f "$receipt_stage" "$receipt" || return 1
)
if ! write_receipt 2>/dev/null; then
    printf 'install: warning: release receipt unavailable; continuing with the installed binary\n' >&2
fi
rm -rf "$work"
trap - EXIT HUP INT TERM
if "$run"; then
    # curl | sh leaves stdin on the script pipe; the terminal UI needs the person's terminal.
    if [ -t 1 ] && [ -r /dev/tty ]; then
        exec "$bin_dir/st" </dev/tty
    fi
    exec "$bin_dir/st"
fi
