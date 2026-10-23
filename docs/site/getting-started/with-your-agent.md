---
title: "Set up with your agent"
description: "An unattended procedure for installing Fractal and running a first mission."
order: 3
---

# Set up with your agent

Give this procedure to an AI coding agent that can run shell commands on the machine you want to use. It installs Fractal, starts its daemon, declares a worker, and runs a small mission. Use an ordinary coding-agent session operating with the user's authorization, **not an agent already managed by Fractal**: setup is a person-only operation and managed agents must not remove or override their bound identity to run it.

## 1. Establish prerequisites

Confirm the user has authorized installation, background execution, local harness integration, and the small file-writing task below. Use the user's supplied person and machine names; if none were supplied for a fresh installation, use the local login name and machine hostname after normalizing them to lowercase letters, digits, hyphens, and underscores. Do not use the reserved machine name `local`.

Check the [platform requirements](install.md). A Claude Code installation with completed native welcome, login, and workspace trust is required for the example below. Do not manufacture credentials, change permission modes, or try to answer provider authentication without the user. If authentication is missing, finish reachable installation steps and report that prerequisite precisely; the mission is not successful until it runs.

If Fractal is already installed or has working state, inspect it first and follow [upgrading](../guides/upgrading.md). Do not overwrite identities, reinstall over a different package manager, delete state, or start a second daemon on the same store.

## 2. Install without opening a UI

```sh
curl --proto '=https' --tlsv1.2 -fsSL \
  https://raw.githubusercontent.com/compoundingtech/fractal/main/install.sh \
  | sh -s -- --no-run
export PATH="$HOME/.local/bin:$PATH"
command -v fractal
fractal --version
```

**Success:** the installer completes, the resolved command is the intended installation, and `--version` prints its build. The release checksum checks integrity; it is downloaded from the same release source as the archive.

## 3. Configure and start the daemon

Set `PERSON` and `NODE` in this shell to the names established in step 1, without `person/` prefixes. For a fresh machine, one possible normalization is:

```sh
PERSON=$(id -un | tr -d '\n' | tr '[:upper:]' '[:lower:]' | tr -c 'a-z0-9_-' '-')
NODE=$(hostname -s | tr -d '\n' | tr '[:upper:]' '[:lower:]' | tr -c 'a-z0-9_-' '-')
```

Prefer explicitly supplied names over those defaults. If `NODE` is `local`, use a different stable name. Run:

```sh
fractal setup --yes --person "$PERSON" --node "$NODE" \
  --harness claude --assistant false --service true --start true --install false
fractal doctor
fractal service status
```

`--assistant false` avoids creating a second onboarding mission. `--yes` supplies defaults without interactive setup; it does not supply provider login. Setup installs the local harness integration and starts a per-user service where supported. If no service manager is available, report the detached-daemon fallback and its lack of reboot persistence rather than claiming a persistent service.

**Success:** doctor reports the daemon reachable and `pty` available; service status agrees with the actual startup method. Do not continue through a failed health check.

## 4. Declare the first agent

```sh
mkdir -p "$HOME/fractal/garden"
cd "$HOME/fractal/garden"
cat > worker.kdl <<EOF
version 2
agent "garden/worker" {
  workspace "$PWD"
  restart "always"
  harness "claude" { model "sonnet"; }
}
EOF
fractal apply worker.kdl --as "person/$PERSON" --dry-run
fractal apply worker.kdl --as "person/$PERSON"
fractal agents show agent/garden/worker
```

**Success:** the declaration preview has no blockers and the worker becomes ready. An installed harness or running process alone does not establish readiness. Inspect the agent's diagnostics if it waits on native login, trust, or permissions; do not silently change the access mode.

## 5. Declare and start the first mission

```sh
mkdir -p missions
cat > missions/first-note.kdl <<'EOF'
version 2
mission "garden/first-note" state="ready" {
  goal "WELCOME.md introduces this workspace in one short paragraph."
  constraint "Only create WELCOME.md. Do not modify other files or publish anything."
  step "write" timeout="15m" {
    assigned-to "agent/garden/worker"
    goal "WELCOME.md exists and contains a short introduction to this workspace."
  }
}
EOF
fractal apply missions/first-note.kdl --as "person/$PERSON" --dry-run
fractal apply missions/first-note.kdl --as "person/$PERSON"
fractal missions start garden/first-note --id garden/first-note/one \
  --workspace "$PWD" --as "person/$PERSON"
fractal missions show mission-run/garden/first-note/one
```

Use a fresh workspace, or inspect existing declarations and the run before repeating these steps. Do not overwrite an existing `WELCOME.md` or start duplicate work after an ambiguous response.

## 6. Verify and hand back

```sh
fractal missions show mission-run/garden/first-note/one
fractal agents queue agent/garden/worker
cat WELCOME.md
```

Wait for the finite run to finish, respecting its declared timeout. **Success:** the run reports completion with its step result, `WELCOME.md` contains the requested paragraph, and no other user files were changed. If it fails or stalls, use [troubleshooting](../guides/troubleshooting.md) and report the actual blocker rather than declaring setup complete.

Tell the user the installed version, startup method, worker ID, run ID, workspace, and observed result. Leave the declared worker available for later missions. To stop it when authorized:

```sh
fractal agents stop agent/garden/worker --as "person/$PERSON"
```

The user can inspect work and messages with the CLI or run `fractal web` to open the daemon's [local web app](../guides/web-and-ios.md). The daemon serves its installed web assets by default; no separate JavaScript server is needed for local access. Treat the browser launch URL as a credential and do not include it in your report. Continue with [Missions in practice](../guides/missions-in-practice.md) to add review and dependencies.
