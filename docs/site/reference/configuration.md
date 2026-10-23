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

## Preview and gate checks

A plain dry run stops at preview: no publication, mission start, helpers, or exec gates. It reports blockers, warnings, proposed actions, and revision information. Add `--check` to deliberately run exec gate checks during preview; provide `--workspace DIR` and required inputs for the intended execution context.

Normal publication checks ordinary exec gates and refuses broken commands. An ordinary “not yet” result does not mean the declaration itself is malformed. Loop-until scripts run at round boundaries, not as publication checks.

## Omission is not deletion

Plain apply is an upsert. Removing a declaration from a file does not stop or delete the previously published subject. Retirement and cancellation are explicit. An [owned set](https://github.com/compoundingtech/fractal/blob/main/docs/st3/owned-sets.md) instead declares complete membership and can retire omitted members under source and revision fences; use it deliberately.

## Definitions, runs, and revisions

Publishing a mission creates an immutable definition revision. `fractal missions start` creates a run pinned to it. To change a live run, write the complete replacement mission and use `fractal work revise`; existing generations stay available for inspection. Do not assume reapplying the mission changes an already running plan.

Mission-owned runtime declarations follow their owner lifetime; independent top-level agents remain available after runs finish. Dependencies order steps, not their order in the file. Durations use forms such as `"90m"`.

For exact supported syntax, see the [mission runtime](https://github.com/compoundingtech/fractal/blob/main/docs/st3/mission-graph-runtime.md), [schema](https://github.com/compoundingtech/fractal/blob/main/docs/st3/schema.md), and [worked examples](https://github.com/compoundingtech/fractal/blob/main/examples/st3/README.md).
