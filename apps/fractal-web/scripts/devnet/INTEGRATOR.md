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

All variables below are required. Paths identify deployment-owned resources;
the scripts do not create a production deployment or provision credentials.

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

## Tick observability

Every tick records `current`: no `error`, a non-null `sha`, and equality with the
stripped production `current-sha` file; a missing file or lock-held tick is false.
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

The deployment provides GitHub/Git access, Devnet and build credentials, and the
`git`, `gh`, `nix`, `pnpm`, `node`, `curl`, Python and shell tooling needed by the
integrator and its external build helper. The follower needs Git, Nix, Python
and GNU coreutils. Production state, credentials, private overlays and machine
configuration stay outside this repository. Deployment owners review and activate
new script revisions explicitly; committing these scripts does not restart a service.
