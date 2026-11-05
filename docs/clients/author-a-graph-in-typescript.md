# Author a graph in TypeScript

Use [`@fractal/declarations`](../../clients/typescript/fractal-declarations/README.md) when declarations share references or defaults. Keep the typed source in Git, render ordinary KDL, and review both before publication. This package encodes declarations; it is not the daemon HTTP client and does not compute mission revisions or publish anything.

## Keep intent separate from execution

```text
TypeScript declarations -> encodeKdl -> reviewed KDL
          |                                |
          +---------- reviewed PR ---------+
                                           |
                               authorized publisher
                                           |
                                  fractal apply -> graph
```

Declarations describe launch definitions, mission work and recurring intent, not transcripts or observed runtime state. Keep organization-specific machine inventory, workspace provisioning, models, personas, credentials and approval policy in the consuming project, outside the serializer.

## Run the example offline

The [example source](../../clients/typescript/fractal-declarations/examples/daily-review.ts) contains one area, two agents, one supervision edge, one mission and one daily schedule. The area is project metadata grouping the same agent objects; it is **not** an `area` KDL node and does not create sidebar folders or placements.

From this repository's root, with Nix and the repository dependencies available:

```sh
nix develop .#web --command pnpm install --frozen-lockfile
mkdir -p tmp/declarations-example
nix develop .#web --command node clients/typescript/fractal-declarations/examples/render.cjs > tmp/declarations-example/graph.kdl
fractal validate tmp/declarations-example/graph.kdl --json
```

Use a `fractal` binary built from the same source revision as the SDK. Validation is offline: no daemon, configuration, publication credentials or running agents are required. It reports declaration identities and native mission revisions; it does not provision workspaces, check installed launch commands or authorize deployment. The example uses `.` and `true` as neutral workspace and launch values, not a production agent harness.

The renderer transpiles the checked-in SDK and example using the workspace TypeScript compiler, with no native adapter or extra JavaScript dependencies. Package typechecking covers the example. The existing TypeScript client CI lane runs it and compares its output byte-for-byte with `fixtures/daily-review.kdl`; the Rust `validate` integration suite checks that fixture against the native parser and pinned mission revision.

## Share objects, not ID strings

The example imports the real package API:

```ts
import { encodeKdl } from '@fractal/declarations'
import type { Agent, Mission, Schedule } from '@fractal/declarations'
```

`writer.under.agent` is the `editor` object. `mission.reportTo` reuses `editor`, and `mission.steps[0].assignedTo` is `writer`. The schedule's work references `mission`. `encodeKdl` lowers these objects to native references without duplicating their IDs in authoring code. References do not implicitly emit declarations: the document explicitly includes the area's agents, the mission and the schedule.

Agent IDs contain `/` or `.`, such as `docs/writer`, so they are not folded through an unknown default host. Supervision, assignment and reporting remain distinct relationships; none grants publication authority. Types cover the SDK's supported subset, not every KDL form or semantic constraint. Validate every generated artifact.

## Bind schedules to native revisions

The example pins the revision returned by native validation for its exact mission definition. The SDK does **not** derive that revision from the mission object. After changing mission goals, steps or other revision-bearing fields, render without the schedule first:

```sh
nix develop .#web --command node clients/typescript/fractal-declarations/examples/render.cjs --mission-only > tmp/declarations-example/mission.kdl
fractal validate tmp/declarations-example/mission.kdl --json
```

Copy the mission's returned revision into `schedule.work.revision`, render the complete graph again, and validate it. Do not hash KDL text, invent a digest or bind to whichever revision happens to be live. When intentionally updating the checked-in example, update its golden fixture and Rust revision assertions together.

The daily schedule uses `host: 'local'`, meaning the daemon receiving publication, not a portable machine selector. Multi-machine publication must route schedules deliberately.

## Review and publish the artifact separately

Review the typed source, pinned dependencies and generated KDL diff. Pay particular attention to supervision, launch commands, assignees, report recipients, schedule frequency and authority changes. Repair the source rather than hand-editing generated output. TypeScript generation executes code: run it in trusted or sandboxed CI without publication credentials. Never put secrets in declarations or rendered output.

Publication is a separate authorized action after review. Retain the exact source revision and artifact; do not regenerate from an unreviewed tree just before applying it. Follow [Configuration and KDL](../site/reference/configuration.md) for preview and publication. An online apply preview is not the offline validation above. Actual publication can reconcile agent desired state, and an active schedule can later create work; publishing a mission definition alone does not start a run.

Plain apply is an upsert: removing an object from TypeScript does not retire its existing declaration. Use explicit retirement or an [owned set](../st3/owned-sets.md) for omission-driven retirement, and review its membership and source fences separately.
