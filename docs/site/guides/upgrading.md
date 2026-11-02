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

## Rollback is a compatibility decision

Do not launch an older binary against a newer store unless the release explicitly supports it. When restore is required, restore the complete compatible state and matching binary, not just one SQLite file. If producer cancellation or runtime closure cannot be proved, roll forward rather than risk duplicating old operations.

See [backups](https://github.com/compoundingtech/fractal/blob/main/docs/st3/backups.md) and [Troubleshooting](troubleshooting.md).
