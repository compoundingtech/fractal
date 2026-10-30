---
title: "CLI reference"
description: "Find the right Fractal command and inspect its exact options."
order: 2
---

# CLI reference

The `fractal` CLI reads and updates the graph from your shell, scripts, and coding agents. For a graphical client, use the [web app](../guides/web-and-ios.md).

```sh
fractal --help
fractal help --all
fractal agents --help
fractal missions start --help
```

Use the installed build's help for exact flags, required arguments, and actor rules. `--json` returns structured output where supported. Current lists focus on active state; use each command's history options for finished work.

## Everyday work

| Command | Purpose |
| --- | --- |
| `fractal now` | Understand what needs action. |
| `fractal agents ls` | Inspect agents and readiness. |
| `fractal agents show ID` | Inspect one agent's current state. |
| `fractal agents queue ID` | Inspect its ordered runs. |
| `fractal missions ls` | List missions and current work. |
| `fractal missions show ID` | Inspect a definition or run. |
| `fractal work ls` | Inspect ready and active steps. |
| `fractal alerts ls` | List requests and approvals. |
| `fractal conversations` | Read and send durable conversation. |
| `fractal usage` | Inspect observed usage and estimated cost. |

## Publication and planning

`fractal validate FILE...` checks authored KDL offline, parsing each file independently with the literal default host `local`. It prints declared subject kinds and IDs plus mission IDs and revisions; `--json` returns `subjects` and `missions` arrays. It needs no configuration, identity, or daemon. Validation is not an authorization or deployment preview: it does not run gates, resolve live graph references, or publish anything.

`fractal apply FILE --as ACTOR --dry-run` previews KDL without publishing. Omit `--dry-run` to publish it. `fractal missions start` creates a run; publishing a definition does not start it. `fractal launch` creates and reviews a planner-backed candidate.

With `--json`, the preview names a `predicted_actions` entry for every changed
subject. `publish-only` means publishing a declaration without a member launch
change, including description/grouping-only agent edits with or without a
harness. Launch changes retain their existing actions (such as
`observe-or-start` or `restart`); runtime interruption and suspension decisions
still take precedence.

For small independent authorized work, use `fractal work start`. Claim before executing assigned work, record progress, and complete with evidence. Use `fractal work revise` for an explicit new generation of an existing run.

## Operating a machine

| Command | Purpose |
| --- | --- |
| `fractal setup` | Configure identity, integrations, and startup. |
| `fractal doctor` | Check daemon and runtime dependencies. |
| `fractal service status` | Inspect the user service. |
| `fractal fleet status` | Inspect fleet membership. |
| `fractal replication status` | Inspect graph synchronization. |
| `fractal devices` | Pair, inspect, and revoke clients. |
| `fractal subject show ID` | Inspect one typed graph subject. |
| `fractal demo check FILE` | Validate a scripted demo file without playback. |
| `fractal web [--state-dir DIR]` | Open an authenticated session in the daemon's local web app; does not start another server. |

`fractal up` runs the foreground daemon for explicit process supervision. Do not launch it alongside another daemon using the same state. Prefer setup's user service for ordinary installations.

The daemon serves installed web assets on loopback port 7410 by default. `fractal web` reads its protected `web-launch-url` rendezvous; `FRACTAL_WEB_URL` explicitly overrides that URL. Keep launch URLs private. Use `fractal doctor` to diagnose missing assets or person configuration. Remote publishing is a separate opt-in setup; do not expose the local browser listener.

## Identity and automation

Use `--as person/NAME` where a person operation requires an actor. A managed agent uses its bound identity; a flag is not permission to impersonate someone. Person-only setup and scripted demos must run outside a bound agent session.

See [Configuration and KDL](configuration.md), the [CLI implementation](https://github.com/compoundingtech/fractal/blob/main/crates/fractal/src/main.rs), and the [web URL launcher](https://github.com/compoundingtech/fractal/blob/main/crates/fractal/src/fractal.rs) for the command contracts.
