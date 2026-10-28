---
title: "Configuration and KDL"
description: "Understand local configuration, atomic declaration publication, and explicit run revision."
order: 3
---

# Configuration and KDL

Local configuration selects the person, node, access mode, state paths, and runtime settings. Declarations describe agents, missions, and other graph subjects. Keep these responsibilities separate.

## Local configuration

Setup owns the ordinary configuration and user-service installation. Home Manager users should keep service settings in [Home Manager](home-manager.md), not install a competing service. Use `fractal actor-contract --effective` to inspect the effective instance and paths before editing a configuration by hand.

Agents inherit the daemon user's environment and account access. A provider permission mode is not sandboxing. Never silently switch the user to unrestricted execution while troubleshooting.

## KDL documents

Every declaration file starts with `version 2`. Declarations follow directly:

```kdl
version 2
agent "example/worker" {
  workspace "/work/example"
  restart "always"
  harness "claude" { model "sonnet"; }
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

For exact supported syntax, see the [mission runtime](https://github.com/compoundingtech/fractal/blob/main/docs/st3/mission-graph-runtime.md), [schema](https://github.com/compoundingtech/fractal/blob/main/docs/st3/schema.md), and [worked examples](https://github.com/compoundingtech/fractal/blob/main/examples/st3/README.md).
