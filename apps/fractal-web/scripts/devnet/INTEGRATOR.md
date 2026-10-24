# Web integration and main follower

`integrator.py` selects Fractal Web and Fractal UI pull requests from
`compoundingtech/fractal`, integrates them in a disposable clone, materializes
locked megarepo members, and publishes an immutable build through Devnet.
`follow.sh` fast-forwards a separate main clone every five minutes, materializes
members with the effect-utils revision from `flake.lock`, and installs changed
workspace dependencies in the web shell. Both scripts write JSONL operational
logs. The follower includes failed install output in its log.

The integrator runs frozen installs through
`nix develop .#web --option builders "" -c pnpm install --prefer-offline --frozen-lockfile`
so the web shell publishes `.devenv/pnpm-source-inputs` before installation.
Its generated-metadata cache includes `pnpm-source-inputs.json`.

## Integrator inputs

Variables below are required unless marked conditional. Paths identify
deployment-owned resources; scripts do not provision deployments or credentials.

| Environment variable | Value |
| --- | --- |
| `FRACTAL_INTEGRATOR_STATE_ROOT` | Existing state root; logs and caches remain under `live/` and recorded stack clones under `clones/` |
| `FRACTAL_INTEGRATOR_CLONE` | Disposable integration clone; the script resets this clone |
| `FRACTAL_INTEGRATOR_PROD_ROOT` | Immutable production builds, including the `current` symlink |
| `FRACTAL_INTEGRATOR_PURPOSE` | Devnet purpose for the production service |
| `FRACTAL_INTEGRATOR_SOURCE_CLONE` | Local source clone used when initializing the integration clone |
| `FRACTAL_INTEGRATOR_PRIVATE_OVERLAY` | Deployment-owned private web overlay directory |
| `FRACTAL_INTEGRATOR_VENDOR` | Deployment-owned web vendor directory |
| `FRACTAL_INTEGRATOR_BUILD_SCRIPT` | Existing build-and-swap shell script; receives clone, revision and build name |
| `FRACTAL_INTEGRATOR_PRIORITY_JSON` | Ordered JSON array of required PR numbers; `[]` explicitly selects none |
| `FRACTAL_INTEGRATOR_TRUST_MODE` | Required: `authors` or `collaborator-permission`; no default |
| `FRACTAL_INTEGRATOR_TRUSTED_AUTHORS` | Conditional: explicit JSON array of trusted GitHub logins, required in `authors` mode; no built-in authors |

Run `python3 apps/fractal-web/scripts/devnet/integrator.py` for the five-minute
loop, or append `--once` for one tick. `--dry-run` additionally requires
`FRACTAL_DRY_RUN_CLONE`; it merges and regenerates dependencies without building
or swapping production. A dry run still mutates its disposable clone and caches.

Existing state names remain unchanged: `live-r3.jsonl`, `live-r3-drop.json`,
`live-r3.lock`, `live-r3-required-stack.json`, pin/state/tree caches and
`<clone-name>-members-key.json`. Keep the integration clone basename unchanged
when adopting the script to reuse the existing member key. The production build
helper and generated launchers remain deployment-owned inputs, not replacements
for the separate launcher scripts.

## Trust boundary

Only same-repository PR heads are eligible: `isCrossRepository` must be false
and the head owner must match the canonical `compoundingtech/fractal` owner.
Fork heads are never eligible. Deployment owners must choose a trust mode:
`FRACTAL_INTEGRATOR_TRUST_MODE=authors` requires author membership in
`FRACTAL_INTEGRATOR_TRUSTED_AUTHORS`; `[]` explicitly trusts no authors.
`FRACTAL_INTEGRATOR_TRUST_MODE=collaborator-permission` instead requires GitHub
repository `write`, `maintain` or `admin` permission, queried once per author per
tick. API failures and read-only permissions are untrusted. This mode does not
require an author allowlist. Missing or unknown trust modes fail before a tick.
Rejected PRs log `fork-head` or `untrusted-author`, including required PRs.
After fetching, a moved head logs `head-moved`; merges use only the exact selected
and verified SHA. Recorded semantic stacks cannot restore rejected or moved heads.

## Publication recovery and retention

Cached successful artifacts under `<prod-root>/builds/<sha>` remain publishable
after an activation failure: the integrator reselects the artifact and retries
publication without rebuilding. An unchanged candidate is not considered served
until the published edge reports that exact revision. Reduced-stack cache entries
record the SHA that actually built successfully, including the ejected PR result.
After confirmed activation, the integrator keeps the current build and one previous
rollback build and deletes older build directories. `integrator-activation.json`
under the production root preserves the confirmed current/previous pair across
unchanged ticks and restarts. Failed activation does not prune builds.
The deployment-owned build helper must leave build cleanup to the integrator;
it must not prune artifacts before publication or remove the rollback target.

## Build helper inputs and snapshot I/O

Set `FRACTAL_INTEGRATOR_BUILD_SCRIPT` to this directory's `build-and-swap.sh`.
The helper takes `SOURCE_DIR SHA NAME` arguments and requires:

| Environment variable | Value |
| --- | --- |
| `FRACTAL_BUILD_ROOT` | Deployment-owned parent of per-name production directories (`<root>/<name>/builds/<sha>`) |
| `FRACTAL_BUILD_SNAPSHOT_ROOT` | Deployment-owned snapshot parent under the repository store's `worktrees/` directory, outside the source clone |
| `FRACTAL_DEP_CACHE` | Dedicated immutable dependency cache outside the mutable source clone, on the snapshot filesystem |

All three root inputs are canonicalized against the caller's directory before the helper
changes into a snapshot, so relative roots retain their original meaning.

The helper creates detached worktrees at `<snapshot-root>/<name>/<sha>-<pid>`.
Each run holds an exclusive `<snapshot>.lock` flock for its snapshot's lifetime.
Reclamation removes a matching revision/PID snapshot only after acquiring that
snapshot's lock non-blocking, so runs with different build roots can safely share
the snapshot namespace. Host Git remains the deployment's PATH Git, including its
worktree-location policy. The source clone need not share the snapshot filesystem;
the immutable dependency cache must share it.

`dependency_cache.py` hashes the path and bytes of these inputs, in this exact order:
`pnpm-lock.yaml`, `pnpm-workspace.yaml`, `megarepo.kdl`, `megarepo.lock`,
`pnpm-source-inputs.json`. This five-file key names `<cache>/<key>`.
Under an exclusive `<cache>/.cache.lock`, initial population copies installed
dependency trees from the mutable clone into a staging directory and atomically
renames it into place. Source-to-cache copying never hardlinks mutable source files.
Published generations are never rewritten. `.fractal-dep-key` and
`.node-modules.json` inside each generation identify its key and safe source-relative
dependency directories; mismatched markers or escaping inventories fail closed.

`.selection.json` outside the generations records the selected current and previous
generation. Reusing an older generation updates selection lineage without mutating it.
The helper takes a shared generation `.lease` before releasing the cache lock and
holds that lease through preparation, build and snapshot cleanup. Before publishing
a new generation, pruning holds the cache lock and waits for an exclusive lease on
each obsolete generation; running builds release leases without reacquiring the
cache lock. This preserves current plus the previously selected generation and never
deletes one in use. A new dependency selection can therefore wait for an older build.

`prepare-dependencies.py SOURCE_DIR SNAPSHOT_DIR MANIFEST_PATH GENERATION KEY`
checks the source's five-file key and generation marker before linking. It copies
the private/vendor overlay, hardlinks installed files from the immutable generation,
and redirects workspace symlinks using their original clone-relative locations,
not the cache parent. An `EXDEV` hardlink failure emits an immediate warning before
copying; the final warning and `dependencyFiles.linked` / `dependencyFiles.copied`
manifest/receipt fields count actual hardlinks and fallback copies.
`dependencyKey` in the manifest and receipt identifies the selected generation.
Production state, manifests, snapshots, private overlay and credentials remain
deployment-owned. The helper stages `current`; it never prunes immutable builds
or publishes Devnet. The integrator alone prunes current plus one rollback build
after the published edge confirms activation.

The helper uses `gate-slot` admission by default; `FRACTAL_BUILD_ADMISSION=fallback`
explicitly reuses a caller's admission timeout and builds with `nice -n 19`.
The deployment supplies Git, GNU coreutils, `flock`, Python, Node and the private
build entry point. These scripts do not provision the private build code.

During an operator-approved cache rollout, any `snapshot-failed` or `build-failed`
tick, or a first built tick above 400 MB of process `write_bytes`, requires reverting
to the deployment's copy-mode helpers. This is an operator rollback criterion,
not an automatic fallback that hides a failed immutable-cache build.

## Tick observability

Every tick reads `servedSha` from `/build-identity.json` (`rev`) at the published `devnet url <purpose> --remote` edge.
`current` requires no `error`, a non-null `sha`, and equality with `servedSha`, not a disk build.
A failed lookup/fetch/identity decode sets `servedSha=null`, `current=false`, and a short `servedError`; lock-held ticks are false.
The `--local` probe records `localServedSha`/`localServedError` for diagnostics only; it never determines `current`.
`build` is `built`, `skipped`, `failed`, or `dry-run`; it is not a freshness signal.
Skipped `reason` is `unchanged`, `lock-held`, or `known-built-tree`/`known-failed-tree`.
Dry-run `reason` is `merged-and-regenerated-no-build-no-swap`; errors retain `build=failed`.
Observability alerts when the last `current=true` timestamp is over 24 hours old, or no tick arrives for 30 minutes.

## Follower inputs

Set `FRACTAL_FOLLOW_CLONE` to a dedicated main clone and `FRACTAL_FOLLOW_LOG` to
its JSONL log path, then run `bash apps/fractal-web/scripts/devnet/follow.sh`.
The follower checks unresolved member directories after every successful
fast-forward (including an unchanged HEAD). It installs after member application
or changes to `pnpm-lock.yaml`, `pnpm-workspace.yaml`, `megarepo.kdl`,
`megarepo.lock`, or `pnpm-source-inputs.json`, using:

```sh
nix develop .#web -c pnpm install --frozen-lockfile --prefer-offline
```

Successful member and install input hashes persist beside the log as
`<log>.members-key` and `<log>.install-key`. Member application also creates
`<log>.install-pending` until installation succeeds. A failed apply or install
therefore retries on later ticks even with unchanged HEAD or after restart;
JSONL records each failure's output. Keep these deployment-owned state files
alongside the log when restarting the follower.
The integrator's installed-input key covers the same five files and retains its
existing `<clone-name>-lockhash` state filename; old lockfile-only keys invalidate.

The deployment provides GitHub/Git access, Devnet and build credentials, and the
`git`, `gh`, `nix`, `pnpm`, `node`, `curl`, Python and shell tooling needed by the
integrator and its external build helper. The follower needs Git, Nix, Python
and GNU coreutils. Production state, credentials, private overlays and machine
configuration stay outside this repository. Deployment owners review and activate
new script revisions explicitly; committing these scripts does not restart a service.

## Known gaps

Candidate genie, install and build commands still inherit the agent environment.
A separate follow-up PR allowlists the environment passed to candidate commands;
the trust policy here does not provide environment isolation.
