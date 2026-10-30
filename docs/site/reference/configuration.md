---
title: "Configuration and KDL"
description: "Understand local configuration, atomic declaration publication, and explicit run revision."
order: 3
---

# Configuration and KDL

KDL is the runnable format. Effect and TS are preview APIs that illustrate the design target, not an installed SDK. Use KDL for the runnable commands in these guides.

Local configuration selects the person, node, access mode, state paths, and runtime settings. Declarations describe agents, missions, and other graph subjects. Keep these responsibilities separate.

## Local configuration

Setup owns the ordinary configuration and user-service installation. Home Manager users should keep service settings in [Home Manager](home-manager.md), not install a competing service. Use `fractal actor-contract --effective` to inspect the effective instance and paths before editing a configuration by hand.

Agents inherit the daemon user's environment and account access. A provider permission mode is not sandboxing. Never silently switch the user to unrestricted execution while troubleshooting.

## KDL documents

Every declaration file starts with `version 2`. Declarations follow directly:

```ts group=worker variant=effect
import { Fractal } from 'fractal/effect'

export const worker = Fractal.agent({
  id: 'example/worker',
  lifecycle: 'standing',
  host: 'local',
  workspace: '/work/example',
  harness: { kind: 'claude', model: 'sonnet' },
})
```

```ts group=worker variant=ts
import { Fractal } from 'fractal'

export const worker = Fractal.agent({
  id: 'example/worker',
  lifecycle: 'standing',
  host: 'local',
  workspace: '/work/example',
  harness: { kind: 'claude', model: 'sonnet' },
})
```

```kdl group=worker variant=kdl
version 2
agent "example/worker" {
  workspace "/work/example"
  restart "always"
  harness "claude" {
    model "sonnet"
  }
}
```

Publish as an authorized actor:

```sh
fractal apply worker.kdl --as person/ada --dry-run
fractal apply worker.kdl --as person/ada
```

Multiple input files form one atomic publication. Parsing, reference resolution, validation, and current-subject checks happen before the transaction commits. One failure rejects the operation rather than partially applying it.

## Harnesses

Supported typed harnesses are `claude`, `codex`, `pi`, `omp`, and `opencode`.
All have defaults: a bare `harness "codex"` is equivalent to `harness "codex" {}`.
The harness starts idle; a person can type into its terminal or send it a message.
Omitting `model` and `effort` leaves those choices to the native harness.

For example, a Codex seat using its native defaults:

```kdl
version 2
agent "example/codex-worker" {
  workspace "/work/example"
  restart "always"
  harness "codex"
}
```

Choose one harness per agent. These are alternative harness nodes, not a single
agent with multiple harnesses:

```kdl
harness "claude" { model "sonnet"; }
harness "codex" { effort "high"; }
harness "pi"
harness "omp"
harness "opencode"
```

`model`, `args` (a list of individual arguments), and `executable` are supported
by every harness. `effort` is supported by all except `opencode`. Native
authentication, supported-version requirements, and provider configuration still
apply; an empty body does not install a harness or sign it in.

### Executable selection and PATH

By default the native program is named after the harness: `claude`, `codex`, `pi`,
`omp`, or `opencode`. Fractal launches a typed driver wrapper which then invokes
that program; selecting an executable does not replace the wrapper or change
the driver protocol.

Managed seats use the daemon user's default account shell environment, captured
by running an **interactive login shell** (`-l -i`). Thus shell startup files,
including version-manager initialization, affect `PATH`; this is not necessarily
the PATH of the terminal running `fractal apply`, nor just launchd's or systemd's
service environment. Agent `env` entries overlay that environment. Fractal also
prepends its own executable directory so its wrapper can be found.

To change lookup for one agent, set its `PATH` (the literal `${PATH}` expands to
the captured shell PATH):

```kdl
env { PATH "/opt/codex/bin:${PATH}"; }
```

To bypass PATH lookup and version-manager shims entirely for the native program,
set `executable` inside the harness:

```kdl
harness "codex" {
  executable "/opt/codex/bin/codex"
  effort "high"
}
```

`executable` is one executable name or path, not a shell command. Spaces in a
path are preserved; put arguments in `args`, not in this string. An absolute
path is recommended for a pinned installation. A plain name uses the seat's
PATH. Fractal does not interpret `CODEX_BIN` or `CLAUDE_BIN` as executable
overrides, and there is no separate per-harness executable setting in local
daemon configuration. A native harness may still read its own configuration
and environment variables after it starts.

## Preview and gate checks

A plain dry run stops at preview: no publication, mission start, helpers, or exec gates. It reports blockers, warnings, proposed actions, and revision information. Add `--check` to deliberately run exec gate checks during preview; provide `--workspace DIR` and required inputs for the intended execution context.

Normal publication checks ordinary exec gates and refuses broken commands. An ordinary “not yet” result does not mean the declaration itself is malformed. Loop-until scripts run at round boundaries, not as publication checks.

## Omission is not deletion

Plain apply is an upsert. Removing a declaration from a file does not stop or delete the previously published subject. Retirement and cancellation are explicit. An [owned set](https://github.com/compoundingtech/fractal/blob/main/docs/st3/owned-sets.md) instead declares complete membership and can retire omitted members under source and revision fences; use it deliberately.

## Definitions, runs, and revisions

Publishing a mission creates an immutable definition revision. `fractal missions start` creates a run pinned to it. To change a live run, write the complete replacement mission and use `fractal work revise`; existing generations stay available for inspection. Do not assume reapplying the mission changes an already running plan.

Mission-owned runtime declarations follow their owner lifetime; independent top-level agents remain available after runs finish. Dependencies order steps, not their order in the file. Durations use forms such as `"90m"`.

## Free mode

Within a fleet, an agent may publish, start, and revise missions and move agent queues as itself without a separate grant. This is broad authority, not isolation: review declarations before publication and give agents only access you intend them to use. A bound harness must retain its own identity; free mode does not allow it to impersonate a person or bypass operation-specific fences, human review, or a claimed producing step.

## Definitions do not start work

Publication saves a definition; it does not create a run. Start it explicitly with `fractal missions start`. To make a new run wait for another run to complete, use `--after RUN`; a failed or cancelled prerequisite is not successful completion. See [Missions in practice](../guides/missions-in-practice.md#queue-follow-on-work) for the runnable sequence.

For exact supported syntax, see the [mission runtime](https://github.com/compoundingtech/fractal/blob/main/docs/st3/mission-graph-runtime.md), [schema](https://github.com/compoundingtech/fractal/blob/main/docs/st3/schema.md), and [worked examples](https://github.com/compoundingtech/fractal/blob/main/examples/fractal/README.md).

## Declared resources

A seat or a top-level mission names the resources it works with as `resource` children:

```kdl
version 2

agent "ada/client" {
  harness "omp"
  resource "goal" uri="urn:agent-goal:ada%2Fclient" reason="seat goal"
  resource "worktree" uri="file:///workspace/client"
}

mission "ada/release" state="ready" {
  goal "Release the client."
  resource "tracker" uri="https://github.com/compoundingtech/fractal/issues/752"
  step "ship" {
    agentless
    goal "Ship it."
  }
}
```

Each name is unique within its declaration. `uri` is an absolute URI of any scheme, up to 4096
bytes, and is kept byte for byte; Fractal does not open it or start an observer. Use percent escapes
for whitespace and non-ASCII characters. `reason` is optional and, when present, must not be blank.
These `resource` children accept no child block.

A named resource is an ordinary addressable smallclaims subject in the graph, not a list kept
beside it. The publication declares `resource/uri/SHA256`, where `SHA256` is the lowercase hex
SHA-256 of the exact URI's bytes, of kind `uri.reference` with an immutable `uri`. There is no URI
normalization: spelling differences, including percent-escape case, produce different subjects.
Every declaration of the same exact URI shares one subject, regardless of its local name or
reason, and no run owns it. The seat or mission stores only typed edges `{name, subject, kind, reason?}`;
the URI lives on the referenced subject. Client v0 resolves these edges and shows them read-only
as `resources` on agents and missions. `resource "tracker" subject="resource/github/example/issue/1"`
instead names an existing or not-yet-replicated graph resource without declaring or fetching it.
Exactly one of `uri` and `subject` is required. `fractal subject show AGENT --kdl` writes URI edges
back with `uri=` when their locator is available and degrades to `subject=` when it is not,
so incomplete replication never prevents reading the declaration. Explicit subject references
remain `subject=` even when the referenced resource has a URI.

Reapplying a seat declaration or publishing a new mission revision replaces that owner's edge
list. Omitting a resource drops its edge, not the shared subject or another owner's references.
Changing a resource's URI points the edge at a different subject; it does not mutate the old
subject's URI. A nested or loop mission body cannot name resources, because only a published
top-level mission declares them.

An owned-set publication (`fractal apply --set NAME`) publishes these shared URI targets alongside
its declarations, but excludes them from the set's membership and ownership. Dropping edges
or retiring the seats and missions that name them never retires the targets.

Mission-only planning candidates, run revisions, and produced-mission outputs publish the
shared URI targets synthesized from that mission's own references. They still reject explicit
immediate declarations, including an explicit declaration of the same URI target.
Run materialization preserves the complete authored resource metadata: names, reasons, and
explicit subject targets are not run-local aliases. For example, `worker` and `agent/worker`
remain distinct resource names even when the mission declares a local agent named `worker`.
An assigned step-local agent's URI targets are published with its declaration before member
startup; this early phase does not publish other agents' targets or unrelated execution state.

To inspect incoming edges, run `fractal subject show RESOURCE --references` (add the global
`--json` flag for JSON), or request `GET /v1/resource-references/{subject}`. Each incoming
reference contains `owner`, local `name`, and nullable `reason`. The lookup covers current agent
declarations and selected published mission definitions, including references to unavailable
targets. Its target-indexed SQLite projection is derived from those declarations, backfilled for
existing stores, and maintained as replicated selection and replay change the graph.

