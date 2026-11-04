---
title: "Upgrade Fractal safely"
description: "Read upgrade impact, preserve recoverable state, and compare client and daemon builds."
order: 7
---

# Upgrade Fractal safely

Upgrade your own fleet deliberately and keep all members on the same release. A patch release does not promise mixed-build compatibility or safe rollback.

## Before replacing binaries

1. Read the selected release's **Upgrade impact** and the [compatibility policy](https://github.com/compoundingtech/fractal/blob/main/docs/st3/compatibility.md).
2. Record the current CLI and daemon versions, installation method, and relevant health diagnostics.
3. Preserve a recoverable backup with matching keys and runtime state. Keep the matching old binary or archive. A claim export is not necessarily a complete runtime rollback snapshot.
4. If you suspect unsigned, unsealed, or inconsistent historical state, preserve raw state and keys **before** doctor, export, or restart. Those operations can perform maintenance; do not treat them as the initial read-only capture.

Use the existing package manager. Do not overwrite a Nix-owned path with an archive installer or change service ownership mid-upgrade.

Keep `.fractal-install.json` beside your installed `bin/fractal`: installers use this
ownership receipt to recognize their executable even when compatibility aliases are
absent. It records the executable's SHA-256, schema, canonical name, installer, and
incoming build's version and commit. Before replacing bytes, installers publish
`.fractal-install.json.pending`; a retry can recognize those exact incoming bytes
if interrupted before the receipt is finalized. Keep both files with the executable;
do not delete them or copy them between builds. An invalid, symlinked, or mismatched
receipt without a matching pending intent stops an upgrade with its path and
reinstall guidance; restore the matching executable and receipt together before
reinstalling through the original installer. The separate `RELEASE.json` is release
display metadata, not an ownership receipt.

The downloader verifies ownership receipts with Python 3, independently of the
selected release's CLI protocol, so retained tags remain installable. It never
executes the destination to verify ownership. Missing, dangling or unreadable
receipt-bound executables are refused with the receipt path and reinstall guidance.

After successfully invoking a retained pre-receipt macOS installer, the downloader
refreshes an already-verified script-owned receipt for the signed fixed app.
This requires the same physical app target, no pending intent, and both aliases
still resolving to that target. Signing can change the archive's executable bytes;
the refreshed digest therefore covers the installed image, while release identity
comes only from the verified archive. This does not waive a mismatch present
before delegation. Interrupted retained installations still require restoring
the matching executable and receipt together.

## Install and restart deliberately

For an archive installation, use the [installer](https://github.com/compoundingtech/fractal/blob/main/install.sh) and its release-selection options. Install without opening the interface so you can inspect the selected build before restarting. For Home Manager, update the selected input and activate through the same configuration that owns the service.

The archive installer prints the installed CLI's old and new versions before replacing an
existing installation. It inspects `service status --json` without running doctor or restarting
services. If the service manager reports a running daemon, the installer prints the exact
installed command to restart it and links this guide. That notice does not establish the
daemon's current build: inspect it after preserving the state described above. Unmanaged
daemons are not reported by the service-manager probe. macOS app backups are retained, but
they do not roll back database migrations.

Use the selected release's restart instructions. Confirm the daemon actually changed, not only the client on your shell PATH:

```sh
fractal --version --json
fractal doctor --json
fractal service status
fractal agents ls
fractal replication status
```

Compare the CLI version with the daemon's `machine_version`. Inspect agent readiness, queued work, and replication after the upgrade. A surviving harness session does not alone prove that its integration is current.

## Default-host st3 → Fractal cutover

`fractal migrate host` moves the default host's XDG config and state roots from
`st3` to `fractal`, preserving their contents by same-filesystem renames. It is
not a binary installer, a database-copy tool, or a fleet-wide migration. Named
instances and remote API endpoints are not supported. Other members can keep
their own local directory names; this is not a mixed-build compatibility guarantee.

Use a release that includes this command and its matching service definitions.
Preserve state, keys, the matching binary and installation receipts as described
above before diagnostics or activation. Record the service owner, daemon arguments,
enabled workers and live seat incarnations. Keep binary installation, host cutover
and any later database relocation as separate operations.

### 1. Prepare and stage through the service owner

For Home Manager, stage through the configuration that owns the services:

```nix
services.smalltalk.fractalMigration.phase = "stage";
```

Stage installs disabled target definitions without moving state. The product module
forces its legacy config entry's `force` setting off in the staged generation,
overriding a consumer's `force = true`; that newest generation is safe. Assertions
still reject other forced ownership of protected legacy config/state paths.
Older retained generations are a separate concern: the mover scans every
`home-manager-N-link` in all inspected profile directories, without a generation
depth limit, and refuses forced collision prefixes that could overwrite the fence.
Dry-run lists the unsafe generation numbers. Preserve their records and recovery
material, then remove those specific retained generations through the owner before
cutover. Do not bypass the guard or indiscriminately garbage-collect generations.

For native service ownership instead, use:

```sh
fractal service install --stage-fractal
```

Do not substitute native installation for Home Manager activation. See the
[installation and staging details](https://github.com/compoundingtech/fractal/blob/main/docs/st3/macos-installation.md)
for platform restrictions, mounts and separate stores.

Before proceeding:

- Keep the recognized legacy daemon and workers running. The mover needs the
  legacy API to check quiescence; do not manually stop them immediately before it.
- Require exactly one installed legacy daemon definition. Corresponding target
  definitions must already be installed, disabled and stopped. Resolve duplicate
  legacy worker aliases and any unrelated service ownership separately.
- Require real legacy directories and absent or empty ordinary destinations on
  the same filesystem. A destination receipt does not authorize merging contents.
- Resolve mounts at or below either source or destination product root before
  initial cutover, preserving data without hiding it. The mover never mounts,
  unmounts or copies data. An explicitly configured external store stays external;
  it must already be reachable. Follow the linked store procedure rather than
  remounting populated data directly onto the destination.
- If the daemon uses `up --config`, `--state-dir` or `--store-dir`, pass the same
  corresponding overrides to every migration command below. These describe the
  running daemon; they do not change service definitions.

### 2. Check, then migrate

```sh
fractal migrate host --dry-run
fractal migrate host --json
```

Save stdout and errors in operator-chosen evidence files. The command does not
choose a receipt filename. A dry-run checks staging, paths, store access, checkpoint
and replication quiescence, and effective memory protection without stopping
services or writing a migration journal. A pending checkpoint, pending replication
records, an unverified first sync, or a peer worker not observed idle/backoff
refuses cutover. Resolve the cause and retry; do not suppress the check.

Actual migration checks quiescence again, disables legacy units, stops recognized
workers before the daemon, fences state/store ownership, exchanges roots and
service drop-ins, and leaves `st3` ancestor symlinks to the migrated roots. It starts
the target daemon, compares effective Linux memory protection and probes the local
API for the same node before committing readiness. It then retires legacy
service-definition links, enables target units and starts recognized auxiliaries.
Other adapters or carriers remain their owner's responsibility.

**Service restart is not seat restart.** The mover deliberately restarts daemon
and recognized worker units, not agent PTYs or harness sessions. Healthy isolated
seats are intended to survive and be adopted. Linux tasks left inside the daemon's
service cgroup can instead die when that unit stops; degraded isolation and
platform-specific process ownership need qualification before cutover. Existing
API connections can break during the gap. Drivers retry, and legacy root bridges
keep old paths resolvable; socket basenames are not globally renamed. Do not
remove those bridges or claim that API readiness proves every seat survived.
See [seats across deploys](https://github.com/compoundingtech/fractal/blob/main/docs/st3/seat-deploys.md).

### 3. Check the evidence and live seats

Keep these three objects distinct:

- **Command stdout:** JSON includes `migration`, `host`, `status`, `manager`,
  `local_api`, `effective` memory protection and `gap_ms`. A successful cutover
  reports `completed` and `local API listening`; this is not per-seat readiness.
- **Root markers:** each migrated config/state root contains
  `.fractal-migration-completed`, plain text consisting of
  `arn:lmig:smalltalk:2026-10-16-state-dirs-fractal` followed by one newline,
  not JSON. Verify both markers and both legacy ancestor symlinks.
- **Recovery journal:** `${XDG_CONFIG_HOME:-$HOME/.config}/.fractal-host-cutover.json`
  records the service/path plan and undo progress, protection and commit state.
  Preserve it; deleting or editing it is not a recovery procedure.

Run the version, doctor, service, agent and replication diagnostics above after
preservation and migration. Compare effective memory protection and daemon build
with the recorded baseline. For every previously live seat, verify the same
incarnation and provider/PTY processes, agent readiness, reconnect through the
retained socket paths and resumed message delivery. A healthy daemon or a surviving
harness alone is insufficient. Record any continuity failure separately from the
migration receipt; do not silently restart seats to make that evidence pass.

### 4. Recover according to readiness, not just command exit status

| Observed boundary | Recovery |
| --- | --- |
| Preflight refused before the swap | Resolve the reported staging, ownership, path or quiescence problem; no service swap or migration journal was made. |
| Swap, target start, protection check or API probe failed before readiness | The mover attempts automatic restoration of legacy paths and prior service running/enablement state. If it reports restoration, inspect legacy health before retrying. |
| Interrupted swap or failed restoration, with readiness known **not** to have succeeded | Preserve the journal and run `fractal migrate host --rollback --json` with the same relevant overrides. It stops/disables targets, restores recorded paths and legacy service state, and removes the journal only on success. Invalid plans, changed paths or product-root mounts refuse; repair through the owner, never forge the journal. |
| API probe succeeded but commit-journal persistence failed | **Do not roll back.** The command explicitly reports this boundary. The on-disk journal may still say `committed: false`; that is not permission to undo state already served by the new daemon. Preserve output and journal and escalate to the service/product owner for forward recovery. |
| Readiness committed, including a later retirement/enablement failure | **Forward-only.** `--rollback` refuses a committed journal. Restore target service health and ownership, then rerun `fractal migrate host --json` to finish retirement without exchanging roots again. The target daemon must run, legacy services must be stopped, and same-node API and effective protection must pass. |

If readiness is uncertain, stop and establish that boundary with the owner rather
than guessing from a nonzero exit or the journal bit. Unmounting a later store
does not reopen rollback after readiness. A binary rollback, Home Manager
generation rollback or macOS app restore is not a host-cutover undo and cannot
restore claims or delivery state subsequently written.

### 5. Complete declarative ownership

After successful migration, set Home Manager's phase to `"completed"` **before
the next activation**. Completed activation requires exact migration markers,
migrated roots, the persistent config fence and an inactive legacy daemon.
Legacy/stage activation after marker publication refuses. Ensure the owning
configuration also adopts the target worker names and all dependent services;
the mover cannot prevent a later configuration from regenerating legacy units.
Verify service ownership, replication and per-seat health again after activation.

Preserve legacy payload, history and recovery material; the mover retires legacy
service-definition links. Only after completed ownership and health acceptance
should the owner separately inventory and archive obsolete material. Retain live
bridges, sockets, config fences, migration evidence and paths still referenced by
running seats. Any later database relocation needs its own stopped-writer,
complete-store preservation and restore plan, not just a copy of one SQLite file.

## Rollback is a compatibility decision

Do not launch an older binary against a newer store unless the release explicitly supports it. When restore is required, restore the complete compatible state and matching binary, not just one SQLite file. If producer cancellation or runtime closure cannot be proved, roll forward rather than risk duplicating old operations.

See [backups](https://github.com/compoundingtech/fractal/blob/main/docs/st3/backups.md) and [Troubleshooting](troubleshooting.md).
