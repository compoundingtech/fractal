---
title: "Install Fractal"
description: "Install the CLI, configure your identity, and start the daemon."
order: 2
---

# Install Fractal

## Requirements

- Linux x86_64 with glibc 2.35 or later, or Apple Silicon macOS 15 or later.
- `curl`, `tar`, and `sha256sum` or `shasum`; macOS also needs Python 3.
- A supported coding harness installed and signed in as the same user who will run the daemon.

Neither Rust nor Nix is required for binary installation. Agents consume your provider's usage and quota and run commands with your user's access. Choose workspaces and tasks accordingly.

## Download

Review the [installer](https://github.com/compoundingtech/fractal/blob/main/install.sh) before running it if you prefer to inspect downloaded scripts.

```sh
curl --proto '=https' --tlsv1.2 -fsSL https://raw.githubusercontent.com/compoundingtech/fractal/main/install.sh | sh
export PATH="$HOME/.local/bin:$PATH"
fractal --version
```

The installer downloads the latest release and checks its SHA256 sidecar. The archive and checksum come from the same release endpoint; this is an integrity check, not an independent signature. To install without opening the interface, pass `--no-run` to the installer, as shown in [agent setup](with-your-agent.md).

## Set up your machine

Run `fractal setup` in your terminal. Choose your person name, a persistent machine name, which installed harnesses to integrate, and whether to keep the daemon running as a user service. The machine name `local` is reserved. Native harness login and workspace trust may need your interaction.

Setup describes each integration before changing it. A new installation uses the `auto` access mode: provider-managed permission checks reduce accidents but are not a security boundary. `bypass` removes those prompts. Do not change that setting without the user's authorization.

The terminal interface opens with your agents and conversations. **Ctrl+H** opens Now; **Ctrl+K** finds an agent or mission; **Ctrl+Q** quits the interface without stopping background agents.

## Check success

```sh
fractal --version
fractal doctor
fractal service status
```

Doctor should report a reachable daemon and available `pty`. A machine without a fleet is healthy. An installed harness is not necessarily authenticated: check the agent's readiness and finish its native first-run screens before continuing.

Next: declare your [first agent](first-agent.md). For declarative installation, see [Home Manager](../reference/home-manager.md). If a check fails, use [troubleshooting](../guides/troubleshooting.md).
