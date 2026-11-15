# Fractal binary releases

**Fractal tag release** verifies both native builds on main, tag pushes and explicit candidate
build dispatches. The daily schedule and on-demand candidate path prepare drafts only from
successful builds (see [Daily releases](#daily-releases)); stable publication requires an explicit
product release GO. A rolling `edge` pre-release follows every green
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

Before each native Cargo build, `scripts/release-zig-preseed.py` checks out the Ghostty
revision pinned by `vendor/libghostty-vt-sys/build.rs` and reads its
`flatpak/zig-packages.json` package-cache pins, including every host and transitive
dependency. All archives retry after 1 and 3 seconds; GitHub archives additionally
fall back to the equivalent `codeload.github.com` transport. Each archive enters the default Zig global cache
through `zig fetch`; its printed content hash must equal the upstream pin or the
job fails. Cargo reuses that exact source checkout through `GHOSTTY_SOURCE_DIR`,
without modifying dependency URLs or hashes. The loopback-only regression test is
`python3 scripts/release-zig-preseed-test.py`.
Truncated bodies are transport failures and follow the same retry/fallback path.
The vaxis dependency also pins an older uucode Git source: it is fetched using
`git+https` at the manifest's exact commit, with 1- and 3-second retries and the
same content-hash comparison. Git sources never substitute a codeload archive.
Each archive attempt has a 60-second elapsed deadline, and each `zig fetch`
subprocess has a 60-second timeout. The cache phase has a shared 660-second
budget; source clone and checkout each have a 120-second timeout (15 minutes
total workload budget, plus process cleanup). Subprocesses own their process groups;
timeouts send the whole group SIGTERM, allow up to 0.2 seconds, then send SIGKILL
and reap the direct child so Git helpers cannot outlive the deadline.
Unavailable transports warn with the package name and leave fetching to
the normal Zig build; reaching the shared deadline warns and stops seeding.
Printed content-hash mismatches remain fatal integrity errors.

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
The publisher first pushes the immutable tag through Git with an absent-ref lease,
then creates the release with `--verify-tag`. An existing tag at the same source
SHA (including an annotated tag's peeled commit) is accepted; a different SHA
stops before any release write. Failed pushes reobserve the remote: a same-SHA
winner is accepted, a different SHA is refused, and transient failures with no tag
retry using the release API retry delays while retaining the absent-ref lease.
An interrupted run can leave a tag without a release; rerunning resumes against
that tag, never replaces it. The source commit must exist in the publisher's checkout.

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
its SHA256 checksum, verifies it before installation, then runs `fractal`. It restores
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

First-run `fractal` offers to install the background service. For an upgrade or changed
executable path, `fractal service install` refreshes
service definitions and restarts daemon and replication services. `fractal service restart` suffices
when those definitions already point at the intended binaries. Schedule that restart, compare
`fractal --version --json` with the daemon's `machine_version` in `fractal doctor --json`, and verify peer
health. `fractal doctor --strict` also treats warnings as failures: inspect each warning rather than
assuming an optional-tool warning means that the API is unavailable. Restarting
invalidates an ongoing continuous soak window, so coordinate that separately from downloading or
installing files.

## Build and publication proof

Every commit on `main`, PRs changing release files and manual **Run workflow** invocations run both native builds,
archive extraction, a temporary-directory installation, CLI help checks, the TUI PTY smoke suite,
and combined checksum/source validation. They upload `fractal-release-attempt-N` as an Actions artifact
(where N is `github.run_attempt`, kept 7 days) and never publish a GitHub release, so release breakage fails on `main` and not at tag
time. Each native job also uploads its own archive and checksum as `release-<target>`, for example
`release-x86_64-unknown-linux-gnu`; once a main commit's run has succeeded,
`gh run download RUN_ID --repo compoundingtech/fractal --name release-x86_64-unknown-linux-gnu`
fetches that commit's Linux archive without building it (find RUN_ID with
`gh run list --workflow release-fractal.yml --branch main --commit SHA`). Tag pushes use
the same build jobs but do not prepare drafts or publish stable releases. The daily and
on-demand candidate paths prepare untagged drafts only after successful builds.
Existing releases are not overwritten; never move a published tag.

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
one release per commit. It builds nothing: it takes the run's `fractal-release-attempt-N` artifact by its
immutable artifact ID, re-checks checksums and the source recorded in each archive, moves the tag with
a leased Git push, then uploads source-qualified staging copies and their ledger before replacing the fixed-name archives and `.sha256`
files one file at a time, then `SHA256SUMS`, then `RELEASE.json` alone as the source commit marker.
A multi-file `gh release upload` can upload concurrently; separate commands enforce alias and sidecar ordering.
It then removes stale assets and rewrites the notes, which give the exact source commit, a link to the run,
and the merged pull request titles since the previous edge (at most 30).
The ID-based download uses `merge-multiple: true` and `path: dist`, keeping archives,
`SHA256SUMS` and `RELEASE.json` directly in `dist/`, as assembly and the edge publisher require.
Archives carry no tag in `BUILD.json`; `source` identifies the commit.
If the release is absent but its tag remains, the same forward-only guard still applies. Creation
first moves the tag with a leased Git push, then creates only the release container using `--verify-tag`.
It shares the normal staging, alias replacement, verification and cleanup sequence.


- Versioned releases and tags are unchanged. `edge` is marked as a pre-release and never as latest, so the
  daily release, `latest/download` and the default installer ignore it.
- Edge has no upgrade-impact section. Read the changes since the previous edge before installing it on a
  store that holds work, and take a [claim backup](backups.md) first.
- The candidate must descend from (or equal) both the edge tag and `RELEASE.json` source; newer or
  divergent published sources refuse publication. Needed history is fetched explicitly; fetch and
  ancestry lookup errors fail closed. Equal-source runs compare every fixed asset's digest with its
  source-qualified staged copy, repair missing or incorrect assets in the same order, and finish
  cleanup/notes. Source markers alone never skip byte verification. Runs share the release mutation lock.
  Staging is one build unit: a source-qualified `STAGING-LEDGER.sha256` hashes every staged archive,
  sidecar, public `SHA256SUMS`, and `RELEASE.json`. It is distinct from the installer's public
  `SHA256SUMS` and uploads alone, last among staged files, as their commit marker. A complete set
  matching its ledger is retained even if a same-source rerun has different build bytes. A missing
  or invalid ledger, missing staged file, or digest mismatch restages the whole unit from validated
  dist before repairing aliases; partial builds are never mixed. Staging always follows the leased tag move.
  A failed tag move preserves the previous fixed-name assets and manifest.
- Between the tag move and alias replacement, installers briefly receive the previous binaries.
  The installer verifies each archive's `.sha256` sidecar, not `SHA256SUMS`: a consistent new pair
  can install before `SHA256SUMS` is replaced. A mismatched pair fails closed; retry after a minute.
  An interrupted replacement or finalization converges on the next run.
- Tag movement uses a Git push with a force-with-lease against the observed tag object SHA, rather
  than the REST ref update. First creation uses an absent-ref lease and pushes before
  creating the release with `--verify-tag`. The publisher passes `GH_TOKEN` through Git's HTTP
  authentication environment when checkout has not persisted credentials. A rejected lease
  stops before any release mutation; retrying reobserves the tag before replacing assets.
- The `edge` tag is excluded from the tag trigger, and a tag moved with the workflow's token starts no workflow.
- `scripts/release-fractal-edge --dry-run --source SHA --dist DIRECTORY` verifies a downloaded artifact
  and prints the notes without publishing.

## Daily releases

**Fractal daily release** (`release-daily.yml`) runs at 05:17 UTC and on manual dispatch
with `mode=candidate` (the default). It takes the newest `main` commit whose release run
succeeded, reads that exact attempt's metadata, and re-checks the checksums and sources of its
`fractal-release-attempt-N` artifact. It selects the next patch version and uploads those same
bytes as a **draft** targeting the exact full source SHA, without creating a Git tag. The tag
is created only when publishing. Immediately before creating the draft, it rechecks that the
run's current attempt is still the verified attempt; a rerun invalidates that selection.
The scheduled path rebuilds nothing. Dispatch with `tag` or `run` to select a candidate
version or successful main push build explicitly. To build a chosen SHA independently
of busy main-push concurrency, dispatch:

```sh
gh workflow run release-daily.yml --repo compoundingtech/fractal --ref main \
  -f mode=candidate -f sha=FULL_40_CHARACTER_SOURCE_SHA \
  -f summary='Operator-impact summary for this candidate'
```

The full lowercase SHA must belong to main's first-parent history. This path dispatches
a separate **Fractal tag release** build on that exact source, waits for success, then
creates the same draft from its verified archives. Build-only orchestration and its native
build use SHA-keyed concurrency groups with cancellation disabled; newer main pushes
cannot cancel them. The build rechecks source eligibility before executing source scripts.
The later mutation job revalidates the successful build identity. `--dry-run --sha`
never dispatches a build and requires an already successful `--run`.
To retry without rebuilding, repeat the candidate dispatch with `-f run=SUCCESSFUL_BUILD_RUN_ID -f sha=FULL_40_CHARACTER_SOURCE_SHA` (and the original tag and summary); the run must be a successful main push or an exact matching candidate build.
The optional `summary` input (`scripts/release-fractal-daily --summary "$SUMMARY"`) adds
the operator's summary to the draft notes; it defaults to empty. Publication preserves
that summary verbatim rather than regenerating it. `scripts/release-fractal-daily --dry-run`
prints the candidate notes without creating a tag or draft.

For an explicitly selected verified build, `recover_draft=true` (script option
`--recover-draft`) opts into one minimal metadata-only draft POST. It requires
explicit `tag`, full `sha`, and successful `run`; it never builds a replacement.
The response status, date and request ID are logged without request credentials.
An ambiguous create is reconciled by paginated authenticated release listing and
release-ID reads, not another POST or deletion. Recovery requires an absent tag,
the exact `Fractal <tag>` name, target SHA and candidate marker, and matching draft flags. Original
assets are uploaded only after reconciliation, checked against their SHA-256
digests, and the notes are updated while retaining draft state. Existing exact
drafts are adopted; conflicting or incomplete evidence stops without deleting
anything. Ordinary candidate creation also preserves and reconciles drafts after
an ambiguous failure rather than deleting or retrying them. Publication still
requires the unchanged exact-SHA product GO and artifact verification.

Candidate version selection, draft creation, cleanup, publication, and edge/embed release
jobs share one job-level `fractal-release-mutation` concurrency group, with cancellation
disabled. Edge and embed are not exempt: they also create or edit GitHub releases.
Within each main-push run, edge waits for both assembly and successful embed publication,
so one run never queues two development publishers against the lock's single pending slot.
Builds never hold this mutation lock. All write-permission release jobs are restricted to
`refs/heads/main`; dispatch release operations with `--ref main`. Publication needs Contents
write and Actions read only; its Actions access inspects the successful build and downloads
proof, not dispatches.

GitHub concurrency allows one running and one pending job, not an unbounded queue: a third
arrival can cancel the older pending job even when `cancel-in-progress` is false. An
always-running cancellation observer makes this a **failed workflow with an explicit
error**, never a silently dropped publication. If a build or mutation is cancelled, the
release operator must re-dispatch with all original inputs (`mode`, `tag`, `run`, `summary`,
`sha`, `go_ref`, `allow_superseded`), after checking whether the release is already
published. Do not interpret cancellation as GO or successful publication.

**Fractal tag release** has its own ungrouped, non-mutating `always()` observer for both
development publishers. A cancelled embed or edge publisher makes that observer fail with
recovery instructions. It reads the original run's latest job metadata with Actions read
permission and prints exact `gh run rerun --job JOB_DATABASE_ID --repo compoundingtech/fractal`
commands for each cancelled publisher, substituting its real database ID. Check edge/embed
state, then run those printed commands. Do **not** rerun the whole workflow: rebuilding a
successful immutable embed changes its captured timestamp and can violate its release pin.
[GitHub documents](https://docs.github.com/en/actions/how-tos/manage-workflow-runs/re-run-workflows-and-jobs)
that specific-job reruns rerun dependents while retaining successful upstream outputs.
Its docs do not explicitly guarantee that `--failed` includes cancelled jobs, so recovery
uses the cancelled job IDs instead. A fresh `workflow_dispatch` is not equivalent: it
builds but deliberately skips the push-only publishers.

The embed publisher verifies or pushes its immutable `embed-SHA` tag before
creating a draft, then uses `gh release create --verify-tag` without `--target`.
The existing tag, not `target_commitish`, fixes the source commit. Supplying the
redundant target causes the release creation POST to return HTTP 504 under the
Actions token, including for an empty body. Separating notes or asset uploads
does not repair that failure.

For an authorized publication diagnosis, set `RELEASE_GH_DIAGNOSTICS=1` on the
publisher process. `release_gh.py` captures the CLI's HTTP trace without printing it
and adds only failed request methods, route templates, HTTP statuses and GitHub
request IDs to errors. This distinguishes a GraphQL preflight from the release
creation POST or an asset upload. Repository/tag/asset names, query strings,
credentials and request/response bodies are excluded. Do not enable or publish
raw `GH_DEBUG=api` logs; they contain release notes and response bodies. Diagnostic
capture does not change the existing retry or publication policy.

Only after a newer candidate has been successfully created with its complete assets and
notes does daily cleanup delete older semantic-version unpublished drafts marked as owned
by this daily workflow. Immediately before deletion, cleanup reads that release by its
immutable release ID and confirms draft status, tag and ownership marker, then deletes
that same ID. Published releases, prereleases and unmarked drafts are untouched; failed
creation never removes older drafts.

Publication waits for an explicit product release GO on that **exact full source SHA**.
After the Fractal Product Lead records the GO in a PR, comment, or message, the
Compounding Manager publishes with:

```sh
gh workflow run release-daily.yml --repo compoundingtech/fractal --ref main \
  -f mode=publish -f tag=vX.Y.Z -f sha=FULL_40_CHARACTER_SOURCE_SHA \
  -f go_ref='URL_OR_MESSAGE_ID_OF_PRODUCT_RELEASE_GO'
```

Before dispatching publication, select a candidate built with attempt-qualified artifacts
and its recorded source SHA. If its build has been rerun, create a fresh candidate from
the new successful attempt and review that candidate before publishing; do not reuse the
previous attempt's draft. Drafts using the old proof format must also be recreated from
a new attempt-qualified build, never repaired by hand-editing `CANDIDATE.json` or assets.

The optional `allow_superseded` dispatch input defaults to empty. Publication refuses a
selected candidate if a newer semantic-version draft exists. To deliberately publish an
older candidate, append `-f allow_superseded=vX.Y.Z` to the command above, using the
exact selected older tag, not the newer tag or a boolean. The corresponding script option
is `--allow-superseded "$ALLOW_SUPERSEDED"`; it must equal the selected tag to override.

The dispatch requires a nonempty `go_ref`; it records the reference, SHA, publisher,
and workflow run in the published release notes. The Compounding Manager is accountable
for ensuring that this dispatcher attestation references the Product Lead's actual GO
for the exact full source SHA. Automation does not verify the referenced message's
contents. No human-only environment review is required.
The publisher refuses a non-draft or prerelease, a SHA that differs from the draft target,
an existing immutable tag at a different source SHA, or a differing `RELEASE.json`, incomplete notes, or invalid
checksums/archive sources. `CANDIDATE.json` records the exact full source SHA, build run ID,
build attempt number, and immutable numeric Actions artifact ID (`build_artifact`).
The publisher also validates the canonical workflow numeric ID and path
(`.github/workflows/release-fractal.yml`) through the GitHub API, not its display name.
Both candidate creation and publication read metadata from `/actions/runs/RUN_ID/attempts/N`, requiring
the recorded source and workflow/run/attempt identity and a successful completed attempt.
Candidate creation requires exactly one unexpired artifact named `fractal-release-attempt-N`;
publication requires that same artifact name and immutable ID and downloads by ID, not by
a run-wide name lookup. It verifies archive source SHAs and checksums and requires the
candidate archive bytes and checksums to match that artifact. Immediately before publishing
with `gh release edit`, it rechecks `/actions/runs/RUN_ID` and rejects a changed current
`run_attempt`, even if all earlier proof checks passed. A later attempt cannot stand in
for the recorded proof. Missing or expired artifacts, old-format proof, and unsuccessful
or mismatched workflow/run/attempt/artifact proof fail closed. Publish within the artifact's
seven-day retention, or build a fresh candidate.
After verification it creates the selected tag at the exact approved source SHA if absent
using a Git push with `--force-with-lease=refs/tags/TAG:` (an absent-ref lease), then publishes
the existing bytes and notes and proposes compatibility baselines. A pre-existing tag at
the approved SHA is accepted, including an annotated tag's peeled commit. A failed push
reobserves the tag, accepting a same-SHA winner but refusing a different SHA; transient
failures with no tag retry with the same absent-ref lease and release API retry delays.
A retry can resume after a successful push followed by an interrupted release edit.
The existing `GH_TOKEN` is passed only through Git's HTTP authentication environment.
Daily draft creation and draft recovery remain untagged; no stable tag is pushed before product GO.
Candidate runs never advance baselines. Consumers must pin only published release tags,
not draft candidate version names. Drafts are private and are not available through
published-release download URLs; stable latest discovery excludes drafts and prereleases.
Rolling `edge` and source-addressed `embed-*` prereleases remain independent development
channels, not stable releases.

Daily and on-demand candidates use the same source-pinned **Upgrade impact**
section before the change list, and attach `UPGRADE-IMPACT.json`. It describes replay,
database changes, checkpoint rules/fleet coordination, client and harness compatibility,
service interruption, manual steps, and recovery. Missing classifications stop publication;
main builds continue. PRs above the documented [adoption boundary](release-impact.md)
also need a fresh valid fragment before merging. See [release impact authoring](release-impact.md) for
committed fragments, measurements, historical backfills, and a notes-only preview.

Archive builds carry a clean release stamp: `fractal --version` reports the package
version and source revision with `(release)`, and JSON exposes the same machine
version. A tag build uses its semantic tag version; untagged main builds use the
Cargo package version and exact revision. Daily publication reuses those tested
bytes, so its later tag is a label rather than an embedded version. Use
`BUILD.json`/`RELEASE.json` for exact source identity. Publication creates the tag at the
tested source; the workflow token's tag does not trigger a second push workflow.

The runner and trigger choices follow GitHub's [runner reference](https://docs.github.com/en/actions/reference/runners/github-hosted-runners)
and [push event rules](https://docs.github.com/en/actions/reference/workflows-and-actions/events-that-trigger-workflows#push).
