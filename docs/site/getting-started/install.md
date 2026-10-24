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
curl --proto '=https' --tlsv1.2 -fsSL https://raw.githubusercontent.com/compoundingtech/fractal/main/install.sh | sh -s -- --no-run
export PATH="$HOME/.local/bin:$PATH"
fractal --version
```

The installer downloads the latest release and checks its SHA256 sidecar. The archive and checksum come from the same release endpoint; this is an integrity check, not an independent signature. `--no-run` installs the commands without launching a client.

The download installer records the selected release tag and the verified binary's
commit in `RELEASE.json` beside the Linux executable. On macOS the receipt is
beside the signed application bundle, named `SmallTalk.app.RELEASE.json`; it does
not change the app's signature. `fractal --version` displays, for example,
`fractal v0.3.18 (abcdef123)` only when the receipt matches the executable's
embedded commit. Missing, unreadable, corrupt, or stale receipts fall back to the
baked crate version and commit. `fractal --version --json` always reports the
unchanged machine version, not the installed release tag.

Nix flake packages and Home Manager install the built package directly, without
running the download installer or writing a release receipt. They therefore
display the baked version and commit, not a daily release tag. Copying a binary
without its receipt has the same fallback behavior.

## Set up your machine

Run `fractal setup` in your terminal. Choose your person name, a persistent machine name, which installed harnesses to integrate, and whether to keep the daemon running as a user service. The machine name `local` is reserved. Native harness login and workspace trust may need your interaction.

Setup describes each integration before changing it. A new installation uses the `auto` access mode: provider-managed permission checks reduce accidents but are not a security boundary. `bypass` removes those prompts. Do not change that setting without the user's authorization.

Use the CLI to inspect agents and missions, or run `fractal web` to open the daemon's [local web app](../guides/web-and-ios.md). The daemon serves installed web assets on loopback port 7410 by default; the launcher opens an authenticated browser session without starting another server. If web access is unavailable, check `fractal doctor` for missing assets or person configuration.

## Check success

```sh
fractal --version
fractal doctor
fractal service status
```

Doctor should report a reachable daemon and available `pty`. A machine without a fleet is healthy. An installed harness is not necessarily authenticated: check the agent's readiness and finish its native first-run screens before continuing.

Next: declare your [first agent](first-agent.md). For declarative installation, see [Home Manager](../reference/home-manager.md). If a check fails, use [troubleshooting](../guides/troubleshooting.md).
