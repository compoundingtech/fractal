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
| `fractal availability` | Read or set your status, interruptions and deadline clock. |

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

### Where declarations come from

A set publication may carry a source map (`fractal apply --set … --source-map FILE`), which records, per new agent or mission revision, the files it was rendered from, the span of each KDL node in them, and the helper and value frames that fed it. These commands read it; each takes `--json`.

- `fractal subject show SUBJECT --source [--revision REV] [--node NODE]` prints the revision's commit, its declaration span with a GitHub permalink pinned to that commit, every step and gate with the line that declares it, and the other files it uses. `--node` takes a KDL node path or a short form such as `step:review/goal` and prints that node's span and frames (declared → helper → value). A `step-run/…` subject resolves through the revision its run pinned.
- `fractal source where PATH[:LINE]` (or a local file in a git checkout, or a GitHub blob URL with `#L…`) lists the live agents and missions that file, or that line of it, declares, feeds or is imported by. Spans recorded against an older version of the file are carried into the newest one by a line diff; `--repository OWNER/REPO` narrows a path present in several repositories.
- `fractal source diff SUBJECT [A..B]` compares two mapped revisions (unique prefixes; either side may be empty; no range compares the two newest): a diff of every contributing file whose text changed, the canonical KDL diff, and a Why line that says whether the subject's own file, only a file it uses, or neither changed.

`fractal work claim` prints a `SOURCE` row under the step's definition when its run's revision has a map, and the faults for a broken gate or a step out of time, and a failing gate's reason, name the line that declares it.

### Bulk desired declarations

```sh
fractal subject desired --json --all
fractal subject desired --json agent/garden/worker mission/garden
fractal subject desired --json --all --kind agent
```

The existing noun is singular: `fractal subject desired --json [--all | SUBJECT...] [--kind K]`.
No subjects (with or without `--all`) selects all declarations. Named subjects are exact IDs,
not fuzzy matches; `--all` cannot be combined with names. `--kind` filters either selection
by the literal desired kind: stopped and terminally retired agents are `stop`, not
`agent`; unfiltered all includes their retained declarations.
The server accepts at most 1,000 supplied names and returns the typed error
`desired-batch-limit` above that limit.

The CLI follows `GET /v1/desired?subject=ID&subject=ID&kind=K&limit=N&cursor=TOKEN`,
with repeated URL-encoded `subject` parameters; omitting them selects all declarations.
`kind` and `cursor` are optional. The default and maximum page limit are both 1,000;
zero or a larger limit returns HTTP 422 `desired-page-limit`.
Each API page is `{"items":[...],"next_cursor":null,"store_index":123}`, with a
string `next_cursor` when another page follows and a `u64` store index. Each item
flattens `DesiredSubject` (`subject`, `kind`, `desired`, `member`, and ownership fields)
alongside `desired_token` (a selected declaration claim ID or `null`) and optional
`actual` (an extensible JSON summary). This is not a client-v0 envelope.

An opaque cursor binds the store origin, pinned claim read cut, shared graph projection
progress, desired row-write generation, selection filters, page limit and last subject.
Continuations must keep the same subjects, kind and limit. The claim cut, graph projection
health/frontier and desired row-write generation must all remain unchanged; otherwise the
API returns typed `desired-cursor-expired`. The frontier covers selected desired declarations
and mission definitions, including backlog projection without claim appends. Transactional
row-write triggers also fence repairs and corrections that leave both `store_index` and graph
health unchanged. Validation reads these bounded witnesses in the page's pinned snapshot;
it does not scan the declarations or use SQLite's connection-relative `data_version`.
There is no historical snapshot or pagination cache, silent restart or automatic retry.
The CLI uses limit 1,000 throughout, collects every page and prints one final JSON array
only after all reads succeed. Cursor invalidation fails the command without partial stdout.

Each page uses one store read snapshot. This GET uses the existing desired-read
`read.projections` scope, deadline and read admission, and remains available during fleet
drain. It does not reduce status or cold-fold cards.
Published mission definitions are included as synthesized `DesiredSubject` entries:
`desired` is the canonical published `MissionSpec`; optional `member` and ownership
fields are absent, including for retained retired definitions.
Their `desired_token` is `null`, matching mission cards: publication claim IDs are
not selected desired-table tokens. Compare mission declarations using
`actual.revision` and `actual.digest`, not a substituted publication claim ID.

Agent `actual` currently includes only the maintained published declaration `revision`
when available. Non-agents include the published `revision`, declaration `digest`,
and maintained `state` when available (missions: `draft`, `ready`, `retired`), never
the body. Runtime and card state fields are intentionally absent. For bulk roster
status, use `fractal agents ls --all --json`, whose status is not card state. See
[the operational query contract](../../st3/operational-queries.md#bulk-desired-declarations)
for exact omissions and the SHA-256 digest's canonical input.

If a retained member contains an unsupported enum variant anywhere in its schema,
or an unknown field in a structure that rejects unknown fields, only that item's `actual` becomes
`{"unavailable":{"code":"member-version-unproven"}}`; normal items and the page
still succeed. This marker is not a revision or a negative runtime observation.
Malformed JSON, wrong types, missing required fields, and other read failures still
fail the page. The authority is the full `MemberSpec` serde schema, including nested
discriminators; it has no numeric wire version. The CLI preserves extensible item JSON,
including member payloads from newer daemons, rather than decoding each member locally.

This automation read deliberately does not reduce status or fold cards. It is not a
replacement for `fractal subject show SUBJECT` or `fractal agents show AGENT`: fetch
the card for fields absent from the cheap summary. These raw `/v1/desired` routes are
outside the generated client-v0 contract; no generated client or OpenAPI artifact
is needed for this command.

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
