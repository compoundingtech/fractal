---
title: "Home Manager"
description: "Let one declarative configuration own installation and the user daemon."
order: 4
---

# Home Manager

Fractal exports `homeManagerModules.default` from its Nix flake. Import it from the Fractal input in your Home Manager configuration and let that configuration own the daemon package, person identity, runtime settings, and startup service.

Use the exported module's [option definitions](https://github.com/compoundingtech/fractal/blob/main/nix/hm-module.nix) for the exact configuration keys supported by your selected revision. Read that revision's release upgrade impact before changing a populated installation.

## Ownership

The module installs the tools, writes user configuration, and manages a systemd user service on Linux or a launchd agent on macOS. Do not install a second archive-owned service over it. Native harness login, workspace trust, macOS permissions, and Linux lingering remain host prerequisites.

The module can publish declared agent and mission files after the daemon starts. Reapplying a file is safe, but removing it from that list does not delete its previously published graph declaration. Use explicit retirement, or deliberately manage membership through an owned set.

## Stable runtime path

The module maintains a stable user-state executable location so running integrations can follow a replaced build. Package and service changes can still restart the daemon. Inspect the daemon version and agent readiness after activation; changing your shell's package does not prove a running daemon changed.

If another deployment mechanism owns the binary, use the module's external-binary setting rather than having both mechanisms replace it.

## Memory and startup

On Linux, size the module's daemon memory limit above measured peak usage, including replication catch-up after downtime. An out-of-memory kill followed by the same catch-up can become a restart loop. Use the module's high and maximum memory options deliberately rather than masking the failure with repeated restarts.

After activation:

```sh
fractal --version --json
fractal doctor --json
fractal service status
fractal agents ls
```

Then continue with [Your first agent](../getting-started/first-agent.md) or [Upgrade Fractal safely](../guides/upgrading.md).
