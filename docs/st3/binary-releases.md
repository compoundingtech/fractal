# Fractal binary releases

Every pushed Git tag runs **Fractal tag release**. Both native builds must pass before a
GitHub release is published. A release is also published once a day when `main` changed since the
last one (see [Daily releases](#daily-releases)). A rolling `edge` pre-release follows every green
`main` commit (see [Edge releases](#edge-releases)). Each release contains:

- `fractal-x86_64-unknown-linux-gnu.tar.gz`: Linux x86_64, linked for glibc 2.35 or newer (Ubuntu 22.04+).
- `fractal-aarch64-apple-darwin.tar.gz`: Apple Silicon, macOS 15 or newer. The executables are
  ad-hoc signed, not Developer ID signed or notarized; macOS may require approval to open them.
- One `.sha256` checksum per archive, combined `SHA256SUMS`, and `RELEASE.json` with the exact
  source commit, target, Rust compiler version, and PTY runtime revision.

Until **2026-11-01**, each target also ships a `smalltalk-<target>.tar.gz` compatibility
archive and checksum. Its root directory keeps the historical `smalltalk-<target>`
name, while the canonical archive uses `fractal-<target>`; all files, modes and links
otherwise match, and publication verifies both layouts. This bridge is
`arn:lmig:smalltalk:2026-10-20-release-artifact-aliases` in [#3032](https://github.com/compoundingtech/fractal/issues/3032).
Old installers and release consumers can keep their asset names during this window.
The current download installer prefers `fractal-*` and falls back to `smalltalk-*`
only on HTTP 404 (for retained tags); checksum or other download failures fail closed.
The byte-pinned v0.3.17 compatibility baseline and installer fixture are unchanged.

Each archive has the physical `bin/fractal`, temporary `bin/st3` and `bin/st` relative symlinks to `fractal`, `bin/pty`, plus `install.sh`, `install-macos.py`, `BUILD.json`, and this guide. Native builds pin Rust1.97.0 in `scripts/release-rust-version`; the workflow and local builder use that same version. PTY is built from the exact
`flake.lock` runtime revision; this is distinct from the `pty-core` library dependency. Linux builds link against glibc 2.35 with pinned Zig 0.15.2, so optional newer
libc functions do not add GLIBC_2.39 requirements or loader noise. These
archives need neither Nix nor Rust installed. Harness CLIs and their logins remain separate.

The embedded web UI ships in `share/fractal-web` in both release archives and
the complete Nix `fractal` package (also selected by transitional `st3` package attributes). The archive installer preserves that tree at
`../share/fractal-web` relative to the chosen bin directory, including upgrade
rollback. On macOS it also seals a copy inside `SmallTalk.app/Contents/share/fractal-web`
before signing, so the app's real executable can resolve its own assets.
No Node runtime or separate web service is needed by installed releases.
Release builders on both Linux and macOS require Nix to build those immutable
assets; the release workflow provisions it on both platforms.
Release staging and installers make copied assets owner-writable; Nix-store
permissions must not prevent extraction cleanup, upgrades, or rollback.

### Offline web embed

The Fractal website owns the embed release job and the downstream version/checksum
pin. Fractal release maintainers retain ownership of the native release pipeline.

The release workflow also builds `fractal-web-embed-<full-source-SHA>.tar.gz`
and its `.sha256` sidecar. The archive root contains `embed.html`, all assets,
four `poster-{light,dark}-{wide,narrow}.webp` images, and `provenance.json`.
Provenance records the exact source, flake lock digest, Node/pnpm/browser versions,
capture viewports and per-file SHA-256 hashes.

Every main build publishes these files under the durable prerelease
`embed-<full-source-SHA>`; native versioned releases also carry them. Unlike the
rolling `edge` release, source-addressed embed releases are never replaced by the
workflow: creation refuses existing releases, and uploads never use `--clobber`.
Repository maintainers must preserve these releases and their tags. Consumers pin
both that full version and the archive SHA-256, not `edge`, `latest`, or a temporary
Actions artifact.

To produce the archive locally, apply the pinned workspace member with `mr apply`,
then run `nix develop .#web --command node scripts/release-fractal-embed.mjs dist`.
The web shell stages effect-utils; the builder installs frozen workspace
dependencies, generates the changelog registry, bundles the offline app and captures
the real `#web` and `#web-narrow` fixtures with the pinned Chromium. No Cargo build
or live daemon is involved. Existing output archives are not overwritten.


Home Manager enables the daemon's web listener at `127.0.0.1:7410` by default.
Set `services.smalltalk.webListen = null;` to disable it, or set an explicit
listener address. Home Manager passes its package's immutable assets explicitly
when copying the daemon into its stable state-directory bin path. Externally
managed binaries retain the release's executable-relative asset layout.

## Install or update

If a populated v0.3.4 store may have unsigned delegation grants, preserve raw state and
keys before doctor or restart. A build containing prevention #1269 can sign preserved
unsealed work; it retains signature warnings from already-sealed affected payloads.

Install the latest release with curl; neither GitHub CLI nor Rust is required:

```sh
curl --proto '=https' --tlsv1.2 -fsSL https://raw.githubusercontent.com/compoundingtech/fractal/main/install.sh | sh
```

The script detects Linux x86_64 or Apple Silicon macOS, downloads the archive and
its SHA256 checksum, verifies it before installation, then runs `st`. It restores
terminal input for the `curl | sh` form. To select an exact release or install
without opening the interface (`vX.Y.Z` below is a placeholder; choose a published tag):

```sh
curl --proto '=https' --tlsv1.2 -fsSL https://raw.githubusercontent.com/compoundingtech/fractal/main/install.sh -o install.sh
sh install.sh --tag vX.Y.Z --no-run
```

To follow `main` instead, pass `--edge`; it is the same as `--tag edge` and cannot be combined with
`--tag`. Without either option the installer still takes the latest versioned release, because
GitHub's latest release never includes a pre-release:

```sh
sh install.sh --edge --no-run
```

An extracted archive can also be installed with `./install.sh --bin-dir
"$HOME/.local/bin"`; add `--quiet` to suppress installer notices. The archive's
command payload is physical `fractal`, relative `st3` and `st` reverse aliases, and `pty`. The source migration
tool remains available as the separate `fractal-migrate` Cargo package.

The physical `fractal` executable opens the terminal interface; `fractal tui` opens
it explicitly, and `fractal web` opens the web interface. `st` and `st3` are temporary
reverse aliases, not separate executables or evidence that legacy callers migrated.
Release and source installers refuse foreign `fractal` destinations and foreign
commands elsewhere on `PATH` before replacement; move the conflicting command
before installing. Named instances install below `~/.st-instance/NAME/bin`.
Native `fractal setup` applies the same positive alias-layout check before copying
any executable or replacing aliases: a physical canonical upgrade requires both
owned `st` and `st3` reverse links; historical exact `fractal -> st3` links can
converge. It does not adopt arbitrary standalone files. Setup invoked through an
already managed executable preserves that installation, including an app-style
canonical link. Signed macOS app upgrades belong to the macOS installer; setup
from a different source build conservatively refuses that app link rather than
replacing or modifying the signed bundle.
The Nix `fractal` package is the complete product, including web assets, PTY and
canonical shell completions. Home Manager installs a physical stable `bin/fractal`;
the former `programs.smalltalk.fractalBinary.enable` opt-in is removed, requiring
consumer configuration removal in the same adoption as the package pin.

Bare `fractal` retains first-run setup and terminal/agent guards. Setup records the
canonical executable and reverse aliases in its uninstall manifest. Historical
manifests still discover exact installer-owned aliases, while foreign replacements
remain protected. Canonical executable removal requires identity with the running
image: Linux uses `/proc/self/exe`; macOS queries the mapped executable vnode,
never metadata of its replaceable pathname. Uninstall from a different source
build conservatively keeps an unverified canonical file. The macOS ABI and native
replacement/unlink proofs must pass before this candidate can be adopted.
State, config and data directory ownership is unchanged.

Put the chosen bin directory on `PATH`. The installer serializes installations in that directory,
stages both tools and the reverse aliases, preserves the previous files, then replaces each by atomic
rename. A handled failure or signal restores the previous command set. This is a series of atomic
file replacements, not a simultaneous switch of all paths; existing processes retain their old
executable. If restoration itself fails, the error names the retained transaction directory and
the `.smalltalk-install.lock` remains: restore the files from its `previous` directory before
removing that lock. macOS retains the helper's app/launchd job and uses its existing restore API,
with a separate app transaction lock shared across custom bin directories. A late rollback
checks the app/launchd generation; if another installation changed it, or a completed generation
is unconfirmed, it leaves the current installation intact and retains the job and locks for
inspection instead of overwriting it.
The installer does not restart
anything or erase state. On macOS it installs `st3` in the fixed `~/Applications/SmallTalk.app` bundle, registers it with Launch Services, and updates existing daemon/replication LaunchAgent paths. Configure `ST_MACOS_SIGNING_IDENTITY` and optionally `ST_MACOS_SIGNING_TEAM` for persistent signing; no identity uses ad-hoc signing. A configured missing identity fails rather than falling back. See [macOS installation and signing](macos-installation.md). Keep the previous archive as recovery material. Installing it again is a supported rollback
only when its database, claim and driver contracts can read the current state; see
[upgrade and recovery](../site/guides/upgrading.md#rollback-is-a-compatibility-decision) and the
[0.x compatibility policy](compatibility.md). Forward-only migrations require roll-forward
unless the release explicitly documents a downgrade. Before upgrading a healthy populated
store, make a [claim backup](backups.md) and preserve excluded local data separately.

First-run `st` offers to install the background service. For an upgrade or changed
executable path, `st service install` refreshes
service definitions and restarts daemon and replication services. `st service restart` suffices
when those definitions already point at the intended binaries. Schedule that restart, compare
`st --version --json` with the daemon's `machine_version` in `st doctor --json`, and verify peer
health. `st doctor --strict` also treats warnings as failures: inspect each warning rather than
assuming an optional-tool warning means that the API is unavailable. Restarting
invalidates an ongoing continuous soak window, so coordinate that separately from downloading or
installing files.

## Build and publication proof

Every commit on `main`, PRs changing release files and manual **Run workflow** invocations run both native builds,
archive extraction, a temporary-directory installation, CLI help checks, the TUI PTY smoke suite,
and combined checksum/source validation. They upload `fractal-release` as an Actions artifact
(kept 7 days) and never publish a GitHub release, so release breakage fails on `main` and not at tag
time. Each native job also uploads its own archive and checksum as `release-<target>`, for example
`release-x86_64-unknown-linux-gnu`; once a main commit's run has succeeded,
`gh run download RUN_ID --repo compoundingtech/fractal --name release-x86_64-unknown-linux-gnu`
fetches that commit's Linux archive without building it (find RUN_ID with
`gh run list --workflow release-fractal.yml --branch main --commit SHA`). Tag pushes use the same jobs, then verify that the tag still
points at the built commit, upload a draft, and publish it only after every asset is present.
Existing releases are not overwritten. If publication fails leaving a draft, inspect and remove
that draft before rerunning the publish job; never move a published tag.

Native builds restore Cargo dependencies and Zig's cache on both platforms; the pinned PTY
runtime uses the same cached Cargo target directory. Generated `target`, `.zig-cache`, and `dist`
files and Python bytecode are ignored by Git, so restored caches and previous archives do not mark
the baked source version dirty. The extracted, installed binary must still report the exact clean
checkout version.

The separate **Nix** workflow builds the default Linux package and every native flake check on
tags, relevant trusted PRs, manual dispatch, and daily at 01:17 UTC. It uses ci1's persistent Nix
store, retains the latest outputs per runner as GC roots, and repeats the build with downloads
and both local and remote builders disabled to prove the results are cached. It needs neither
GitHub's Actions cache nor FlakeHub. The `nix-release-proof` artifact records the exact source,
targets, and output paths. The scheduled check starts four hours before the daily release schedule
to catch package breakage early.

The Fractal Nix check uses nextest to give each test its own process, including CPU-budget and
delivery-presence fixtures. Python, Node, and the Linux process utilities are declared test inputs;
the Fractal messaging compatibility binary is the published release archive pinned by
`.github/messaging-compat-baseline.json` and supplied through `ST3_MESSAGING_COMPAT_BIN` by the check’s `preCheck` inside the sandbox.
The native runtime suite and documentation tests remain covered.

## Edge releases

After a `main` run of **Fractal tag release** passes every job, its last job (`release-publish-edge`)
moves one GitHub pre-release, tagged `edge`, to that commit. It is a single release that is updated, not
one release per commit. It builds nothing: it takes the run's `fractal-release` artifact, re-checks the
checksums and the source recorded in each archive, then replaces the assets (both target archives and
their compatibility archives, their `.sha256` files, `SHA256SUMS` and `RELEASE.json`), rewrites the notes, and moves the tag. The notes give the exact
source commit, a link to the run, and the merged pull request titles since the previous edge (at most 30).
Archives carry no tag in `BUILD.json`; `source` identifies the commit.

- Versioned releases and tags are unchanged. `edge` is marked as a pre-release and never as latest, so the
  daily release, `latest/download` and the default installer ignore it.
- Edge has no upgrade-impact section. Read the changes since the previous edge before installing it on a
  store that holds work, and take a [claim backup](backups.md) first.
- A run for a commit older than the current edge changes nothing, so a slow run cannot move edge
  backwards. Runs are serialized. While assets are being replaced, a download can briefly pair a new
  archive with an old checksum; the installer fails closed on the mismatch, so retry after a minute.
- The `edge` tag is excluded from the tag trigger, and a tag moved with the workflow's token starts no workflow.
- `scripts/release-fractal-edge --dry-run --source SHA --dist DIRECTORY` verifies a downloaded artifact
  and prints the notes without publishing.

## Daily releases

**Fractal daily release** (`release-daily.yml`) runs at 05:17 UTC and on manual dispatch. It takes the
newest `main` commit whose release run succeeded; if that commit is not already in the latest
release, it re-checks the checksums and sources of that run's `fractal-release` artifact, publishes
those same bytes under the next patch tag (the highest `vX.Y.Z` tag plus one; dispatch with `tag` to
choose another), and lists the merged pull request titles since the previous release in the notes.
Nothing is rebuilt, and the tag is created at that exact commit with the workflow's own token, so it
does not start a second tag build. There are no version-bump commits. Archives from these releases
carry no tag in `BUILD.json`; `source` identifies the commit. `scripts/release-fractal-daily
--dry-run` prints what would be published without publishing.

The downloader prefers `fractal-release` and also accepts `smalltalk-release` from older
successful runs until those 7-day artifacts expire
(`arn:lmig:smalltalk:2026-10-26-release-artifact-name`, sunset 2026-11-01).

Daily, manual dispatch, and tag publication use the same source-pinned **Upgrade impact**
section before the change list, and attach `UPGRADE-IMPACT.json`. It describes replay,
database changes, checkpoint rules/fleet coordination, client and harness compatibility,
service interruption, manual steps, and recovery. Missing classifications stop publication;
main builds continue. PRs above the documented [adoption boundary](release-impact.md)
also need a fresh valid fragment before merging. See [release impact authoring](release-impact.md) for
committed fragments, measurements, historical backfills, and a notes-only preview.

Archive builds carry a clean release stamp: `st --version` reports the package
version and source revision with `(release)`, and JSON exposes the same machine
version. A tag build uses its semantic tag version; untagged main builds use the
Cargo package version and exact revision. Daily publication reuses those tested
bytes, so its later tag is a label rather than an embedded version. Use
`BUILD.json`/`RELEASE.json` for exact source identity. Tag the tested commit (with this workflow in its tree); pushing a tag does not
wait for unrelated CI runs. Tags made with another workflow's default `GITHUB_TOKEN` do not
trigger another push workflow, and GitHub does not emit tag push events for a push of more than
three tags: push release tags individually from a person or app credential.

The runner and trigger choices follow GitHub's [runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)
and [push event rules](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#push).
