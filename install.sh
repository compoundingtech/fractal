#!/bin/sh
# Download a verified Fractal release, install it, and open Fractal.
set -eu
# arn:lmig:smalltalk:2026-10-16-env-fractal-dual-read
base=${FRACTAL_INSTALL_RELEASE_BASE_URL-${ST_INSTALL_RELEASE_BASE_URL:-https://github.com/compoundingtech/fractal/releases}}
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
# Retained archives predate the package installer's ownership check. Keep this
# preflight here too, so choosing an older tag cannot overwrite a Nix installation.
refuse_nix_path() (
    # Safety net for real Nix/Home Manager layouts (normally absolute links), not a hardened path resolver.
    normalize_path() (
        prefix=$1
        missing=
        while [ ! -d "$prefix" ]; do
            missing="/$(basename "$prefix")$missing"
            prefix=$(dirname "$prefix")
        done
        normalized=$(cd "$prefix" && pwd -P)
        while [ -n "$missing" ]; do
            part=${missing%%/*}
            if [ "$missing" = "$part" ]; then missing=; else missing=${missing#*/}; fi
            case "$part" in
                ''|.) ;;
                ..) normalized=${normalized%/*} ;;
                *) normalized=${normalized%/}/$part ;;
            esac
        done
        printf '%s\n' "${normalized:-/}"
    )
    path=$1
    hops=0
    # Compare roots and candidates in the same physical spelling (/var is a
    # symlink to /private/var on macOS), even when a profile leaf is missing.
    home_root=$(normalize_path "$HOME")
    default_profiles=$home_root/.local/state/nix/profiles
    xdg_profiles=$(normalize_path "${XDG_STATE_HOME:-$HOME/.local/state}/nix/profiles")
    while :; do
        case "$path" in
            /nix/store|/nix/store/*|/nix/var/nix/profiles|/nix/var/nix/profiles/*|"$home_root/.nix-profile"|"$home_root/.nix-profile/"*|"$default_profiles"|"$default_profiles/"*|"$xdg_profiles"|"$xdg_profiles/"*)
                printf 'install: refusing to overwrite Nix-owned path %s. Upgrade through the Home Manager configuration that owns this installation: https://fractal.how/docs/reference/home-manager\n' "$1" >&2
                exit 1 ;;
        esac
        parent=$(dirname "$path")
        if [ -d "$parent" ]; then
            resolved="$(cd "$parent" && pwd -P)/$(basename "$path")"
            if [ "$resolved" != "$path" ]; then path=$resolved; continue; fi
        fi
        # A GC'd generation can leave an ancestor dangling while the leaf itself
        # is not a symlink. Resolve the nearest symlink component, retaining the
        # missing suffix, rather than treating a missing parent as unowned.
        component=$path
        suffix=
        while [ ! -L "$component" ] && [ "$component" != / ] && [ "$component" != . ]; do
            suffix="/$(basename "$component")$suffix"
            component=$(dirname "$component")
        done
        [ -L "$component" ] || break
        hops=$((hops + 1))
        [ "$hops" -le 40 ] || { printf 'install: cannot resolve installation link %s\n' "$1" >&2; exit 1; }
        link=$(readlink "$component")
        case "$link" in /*) path=$link$suffix ;; *) path="$(dirname "$component")/$link$suffix" ;; esac
        path=$(normalize_path "$path")
    done
)
for name in st3 st pty fractal; do refuse_nix_path "$bin_dir/$name"; done
if [ "$target" = aarch64-apple-darwin ]; then
    install_home=${FRACTAL_MACOS_INSTALL_HOME-${ST_MACOS_INSTALL_HOME:-$HOME}}
    app=${FRACTAL_MACOS_APP_PATH-${ST_MACOS_APP_PATH:-$install_home/Applications/SmallTalk.app}}
    refuse_nix_path "$app"
fi
# Retained package installers treated fractal as optional. Refuse a foreign
# canonical command here as well: never install st and then launch a collision.
case "$bin_dir" in /*) ;; *) bin_dir=$PWD/$bin_dir ;; esac
if [ -d "$bin_dir" ]; then bin_dir=$(cd "$bin_dir" && pwd); fi
owned_fractal_link() {
    [ -L "$1" ] || return 1
    link_target=$(readlink "$1")
    case "$link_target" in
        st3|"$bin_dir/st3") return 0 ;;
    esac
    if [ "$target" = aarch64-apple-darwin ]; then
        case "$link_target" in "$app/Contents/MacOS/fractal"|"$app/Contents/MacOS/st3") return 0 ;; esac
    fi
    return 1
}
canonical=$bin_dir/fractal
if [ -e "$canonical" ] || [ -L "$canonical" ]; then
    owned=false
    if owned_fractal_link "$canonical"; then
        owned=true
    elif [ -f "$canonical" ] && [ -x "$canonical" ] && [ ! -L "$canonical" ]; then
        owned=true
        for name in st st3; do
            if [ ! -L "$bin_dir/$name" ]; then owned=false; break; fi
            case "$(readlink "$bin_dir/$name")" in fractal|"$canonical") ;; *) owned=false; break ;; esac
        done
    fi
    "$owned" || { printf 'install: refusing to replace foreign fractal command: %s\n' "$canonical" >&2; exit 1; }
fi
old_ifs=$IFS
IFS=:
set -f
for directory in ${PATH:-}; do
    [ -d "${directory:-.}" ] || continue
    directory=$(cd "${directory:-.}" && pwd)
    [ "$directory" != "$bin_dir" ] || continue
    candidate=$directory/fractal
    if [ -e "$candidate" ] || [ -L "$candidate" ]; then
        if [ -L "$candidate" ]; then
            case "$(readlink "$candidate")" in "$canonical"|"$bin_dir/st3") continue ;; esac
            if [ "$target" = aarch64-apple-darwin ]; then
                case "$(readlink "$candidate")" in "$app/Contents/MacOS/fractal"|"$app/Contents/MacOS/st3") continue ;; esac
            fi
        fi
        printf 'install: refusing to install: PATH contains a foreign fractal command: %s\n' "$candidate" >&2
        exit 1
    fi
done
set +f
IFS=$old_ifs
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
# Only a physical canonical payload selects fractal; retained forward aliases
# still install their historical st3 payload without replacing an existing fractal.
# This is an archive-format decoder, not a fallback around a foreign destination.
command_name=fractal
[ -x "$work/$package/bin/fractal" ] && [ ! -L "$work/$package/bin/fractal" ] || command_name=st3
# Retained releases keep their historical installer spelling.
"$work/$package/install.sh" --bin-dir "$bin_dir"
bin_dir=$(cd "$bin_dir" && pwd)
# A release receipt is optional display metadata. Failure must never prevent a
# successfully installed executable from launching.
write_receipt() (
    receipt_stage=
    trap '[ -z "$receipt_stage" ] || rm -f "$receipt_stage"' EXIT
    release_binary="$work/$package/bin/$command_name"
    version=$("$release_binary" --version --json) || return 1
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
        receipt=$(python3 - "$bin_dir/$command_name" <<'PY'
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
        exec "$bin_dir/$command_name" </dev/tty
    fi
    exec "$bin_dir/$command_name"
fi
