---
title: "When something is wrong"
description: "Diagnose the smallest failing boundary while preserving state and useful evidence."
order: 8
---

# When something is wrong

Start with the smallest question: is the daemon reachable, is the agent ready, or is work waiting on a dependency? The agent and run examples below come from [Your first agent](../getting-started/first-agent.md) and [Missions in practice](missions-in-practice.md); substitute an agent or run you actually created.

**Preserve first:** if historical signing, unsealed claims, or store consistency is in doubt, preserve raw state and keys before doctor, restart, or export. Maintenance may change the evidence. Do not delete databases or identities to make a failure disappear.

## Installation and daemon

```sh
command -v fractal
fractal --version --json
fractal service status
fractal doctor --json
```

Check that your shell resolves the intended installation and that the daemon's `machine_version` matches it. A newly installed client can still contact an older running daemon. Confirm `pty` is available to the daemon user's login environment, not just to your current shell.

If setup used a detached fallback because no service manager was available, do not expect it to survive reboot. If Home Manager owns the service, make changes there rather than installing a competing service.

## Agent readiness

```sh
fractal agents show agent/garden/worker
fractal agents queue agent/garden/worker
```

Read the actual readiness, end reason, and diagnostic. Native provider login, welcome screens, workspace trust, and account quota are separate from finding the harness executable. Complete native screens under the same user and profile as the daemon.

Repeated failed starts can park the agent. Fix the cause before one deliberate restart. Do not cycle the daemon as a substitute for addressing authentication or a refused launch.

## Work and review

```sh
fractal missions show mission-run/garden/readme/one
fractal work ls
fractal alerts ls --as person/ada
```

A published mission has not necessarily been started. A blocked step may be waiting on a dependency, gate, another run, or the agent's current claim. Read the displayed blockers before reassigning or starting duplicate work.

## Another machine or client

For fleet problems, inspect `fractal fleet status --json` and `fractal replication status`. Membership and synchronization are different checks. For a paired client, verify the paired-only gateway, current connection, and device scope. Never expose the privileged daemon socket to fix a client connection.

## Report safely

Include the command, sanitized error, platform, exact build, timing, and expected versus observed behavior in a [GitHub issue](https://github.com/compoundingtech/fractal/issues/new/choose). Remove credentials, invitation and pairing codes, private hostnames, provider transcripts, and user data. Do not attach a database, raw backup, full configuration, or environment dump.
