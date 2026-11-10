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
| `fractal agents show ID` | Inspect one agent's current state and its charter. |
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
change, including description (charter) or grouping-only agent edits with or without a
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
| `fractal claims ls --kind KIND --subject-prefix PREFIX` | List claims by kind, exact subject, or subject prefix. |
| `fractal demo check FILE` | Validate a scripted demo file without playback. |
| `fractal web [--state-dir DIR]` | Open an authenticated session in the daemon's local web app; does not start another server. |

`fractal up` runs the foreground daemon for explicit process supervision. Do not launch it alongside another daemon using the same state. Prefer setup's user service for ordinary installations.

The daemon serves installed web assets on loopback port 7410 by default. `fractal web` reads its protected `web-launch-url` rendezvous; `FRACTAL_WEB_URL` explicitly overrides that URL. Keep launch URLs private. Use `fractal doctor` to diagnose missing assets or person configuration. Remote publishing is a separate opt-in setup; do not expose the local browser listener.

### Device renewal

Stage an audited historical device grant on the member's trusted local Unix socket:

```sh
fractal devices renew-stage --as agent/NAME device/DEVICE_ID \
  --migration-epoch EPOCH \
  --allow SHA256
```

`--allow` is required: supply the audited exact source record body's digest, encoded
as 64 lowercase hexadecimal SHA-256 characters. The source is the historical
`pairing-completed` claim's `ClaimRecord.body`. Its digest is defined by
`device_renewal::audited_grant_digest`: SHA-256 of the UTF-8 bytes returned by
`smallclaims::hash::canonical_json_text`, which recursively sorts object keys.
Do not hash insertion-order JSON, a pretty-printed document, or the whole claim.
The member validates the digest and migration epoch before staging.

The approval window defaults to 72 hours. Set `--expires-in-hours HOURS` to choose
1–168 hours. Staging accepts no scope or candidate-key overrides and is unavailable
over HTTP or Fabric endpoints. It prints only the renewal ID and expiry; `--json`
returns the stage receipt, including its audited digest, without a credential.
Renewal explicitly supports an expired or short-lived source grant. The audited
stage fixes the replacement expiry at 30 days after staging, never later than
30 days after approval. Both the signed offer and phone approval bind that exact
expiry; completion preserves it rather than starting a new lifetime.
The saved phone profile discovers its renewal when the app starts or returns to
the foreground. Review the displayed member, person, requester, and scopes there, then tap
**Approve** to finish with native user presence. No pairing code or QR scan is
needed.
A managed agent must use its own literal identity; a person operator can stage as
themselves. The historical grant determines the phone key and person being renewed.
If staging reports an unreceipted historical withdrawal, surface that record to
the operator; the command does not replace or disregard it.

## Identity and automation

Use `--as person/NAME` where a person operation requires an actor. A managed agent uses its bound identity; a flag is not permission to impersonate someone. Person-only setup and scripted demos must run outside a bound agent session.

See [Configuration and KDL](configuration.md), the [CLI implementation](https://github.com/compoundingtech/fractal/blob/main/crates/fractal/src/main.rs), and the [web URL launcher](https://github.com/compoundingtech/fractal/blob/main/crates/fractal/src/fractal.rs) for the command contracts.
