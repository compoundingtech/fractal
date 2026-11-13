# fractal

Typed authoring for Fractal declarations: agents, missions, schedules and objectives kept as TypeScript modules in a directory tree, validated with Effect schemas or the native readers' rules and rendered to KDL v2. This is the private workspace package `fractal`; it is not published to npm.

| Entrypoint | Runtime | Contents |
|---|---|---|
| `fractal` | isomorphic | The `Fractal` authoring namespace; declaration schemas and the KDL lowering (`agent`, `mission`, `schedule`, `objectiveNode` nodes, `emit`, `fractalKdl`) as named exports |
| `fractal/tree` | Node | `loadSubjectTree`, `programMission`, the define forms and reference types |
| `fractal/testing` | Node | `scratchDaemonLayer`, an isolated daemon for conformance tests |
| `fractal/source-map` | Node; the read trace needs Bun | `sourceMap` and `traceSourceReads`: the `fractal.source-map/v1` document of a rendered tree |
| `fractal/effect` | Bun program host, isomorphic | The `Fractal` namespace with the program forms of `mission` and `step`, plus `ask`, `run`, `command`, `env`, `cwd` and `Worker`; the host contract (`ProgramEntry`, `HostContextV1`) as named exports |

## Authoring

Each declaration is the default export of `<id>/agent.ts`, `<id>/mission.ts`, `<id>/schedule.ts` or `<id>/objective.ts` below a tree root. `meta: import.meta` ties the declaration to its module; the loader derives the ID from the module's directory, so declarations never repeat it. Imported declarations are typed references.

```ts
// garden/worker/agent.ts
import { Fractal } from 'fractal'

export default Fractal.agent({
  meta: import.meta,
  workspace: '/work/garden',
  restart: 'always',
  harness: { kind: 'claude', model: 'sonnet' },
})
```

```ts
// garden/first-note/mission.ts
import { Fractal } from 'fractal'

import worker from '../worker/agent.ts'

export default Fractal.mission({
  meta: import.meta,
  state: 'ready',
  reportTo: worker,
  goal: 'WELCOME.md introduces this workspace in one short paragraph.',
  steps: [{ id: 'write', timeout: '15m', assignedTo: worker, goal: 'WELCOME.md exists.' }],
})
```

An objective (`docs/st3/objectives.md`) is a target over something Fractal measures. A fleet declares intervals and counts over claims, named `NAMESPACE/NAME` by their directory; `objective.ts` needs no `meta` because nothing references an objective, and a `name`, if given, must equal the path ID.

```ts
// ops/change-rollout/objective.ts
import { Fractal } from 'fractal'

import releaseManager from '../release-manager/agent.ts'

export default Fractal.objective({
  about: 'A change, from its author marking it ready to every member that serves it',
  indicator: Fractal.indicator.interval({
    subjectPrefix: 'custom/acme/change/',
    start: { kind: 'custom.acme.change-ready', at: 'ready_at' },
    expect: { kind: 'custom.acme.change-merged', set: 'hosts', at: 'merged_at' },
    stop: { kind: 'custom.acme.change-applied', member: 'host', at: 'applied_at' },
    cancel: 'custom.acme.change-abandoned',
    labels: ['class'],
  }),
  window: { span: '7d', minSamples: 20 },
  targets: [
    { stat: 'p50', within: '60s' },
    { stat: 'p95', within: '5m' },
  ],
  onBreach: Fractal.response.fault({ owner: releaseManager, condition: 'change-rollout-slo-missed' }),
})
```

A count takes one `per-hour` target with `min` or `max`: `Fractal.indicator.count({ subjectPrefix: 'custom/factory/merged-pr/', event: { kind: 'custom.factory.pr-merged', at: 'merged_at' }, labels: ['repository'] })` with `window: { span: '24h' }` and `targets: [{ stat: 'per-hour', min: 10 }]`. Only the count's own row is judged against the target; each label row is a part of it, shown without a verdict.

An interval or count keeps a sample or event only when its start (or event) claim has one of the values of each `only: [{ field, values }]`, one per field; a claim without the field is left out, and left-out ones are counted with the excluded. An interval's `stop` may name its `start` kind when both name different `at` fields: one claim is then the whole sample, as `start: { kind: 'github.workflow-run', at: 'created_at' }, stop: { kind: 'github.workflow-run', at: 'completed_at' }` reads a CI run from creation to completion. Such a stop takes no `expect` or `member`.

`Fractal.objective` checks the rules the native reader enforces and throws on the first broken one. The other indicators (`latency`, `share`, `statement`, `transaction`, `writerHold`, `cpu`, `database`, `divergence`) are native telemetry: Fractal ships them in [`slo/objectives.kdl`](../../slo/objectives.kdl), rendered by [`slo/objectives.kdl.genie.ts`](../../slo/objectives.kdl.genie.ts), and the tree loader refuses them. `src/objective.ts` and `src/kdl.ts` import nothing outside the package, so Genie bundles that file without installing dependencies.

The namespace also holds `step`, `gate`, `person`, `input`, `product`, `t`, `doc`, `pr`, `childMission`, `runId`, `completed`/`failed`/`terminal`, `Timing`, `Launch`, `Review`, `Fragment`, `Mission`, `Round`, `omp` and `pinMission`. `Fractal.defineAgent`, `Fractal.defineMission`, `Fractal.defineSchedule` and `Fractal.defineObjective` are the low-level forms: `{ meta, intent }`, where `intent` may be a function of the loader's `context`.

### Harness startup messages

OMP, Codex and Claude harnesses accept an optional `message: { text, id }`:

```ts
Fractal.agent({
  id: 'garden/worker',
  harness: Fractal.omp({
    model: '@generalist_root',
    message: { text: 'Read the garden plan and begin.', id: 'garden-start-v1' },
  }),
})
```

This lowers to `harness "omp" { model "@generalist_root"; message "Read the garden plan and begin." id="garden-start-v1"; }`.
For Codex or Claude, supply the same `message` field on the harness record.
The native runtime delivers it as a one-shot first-turn message keyed by
`(subject, id)`, not as a recurring prompt. Changing either the text or the id
restarts the agent. Text must be nonblank, contain no NUL, and fit in 64 KiB of
UTF-8; the nonempty id must fit in 256 UTF-8 bytes, matching native admission.

`step()` and `gate.*()` validate eagerly, so a step handle's `assignedTo` must be an object with an `id`; a path-derived reference works in an inline step object, which the loader lowers.

## Loading and rendering

`loadSubjectTree({ root, context, bundleDir? })` imports every declaration module below `root`, lowers typed references to IDs without copying authored step and gate handles, assembles missions with `missionWire`, decodes the result with the strict schemas, and checks objectives with `Fractal.objective`. Program missions are bundled into `bundleDir` (see [Publishing a program](#publishing-a-program)). Render a loaded subject with the named exports:

```ts
import { agent, emit, mission, objectiveNode } from 'fractal'

const kdl = emit([subject.kind === 'agent' ? agent(intent) : mission(intent)])
const objectiveKdl = emit([objectiveNode({ ...intent, name: subject.id })])
```

Validate the output with `fractal validate` before publishing it with `fractal apply`.

### Source maps

`fractal apply --set … --source-map MAP.json` records where each published revision came from ([owned sets](../../docs/st3/owned-sets.md#source-maps)). `fractal/source-map` builds that document for rendered subjects. It imports nothing outside Node, so a loader can use it from a pinned checkout; the caller passes `parseSync` from `oxc-parser`.

```ts
import { parseSync } from 'oxc-parser'
import { sourceMap, traceSourceReads } from 'fractal/source-map'

// Before the tree is imported, in Bun: wrap the literal consts of the tree's modules.
const trace = traceSourceReads({ parse: parseSync, roots: [root] })
// …load and render the tree…
const { document, coverage } = sourceMap({
  parse: parseSync,
  subjects: rendered.map(({ subject, file, kdl }) => ({ subject, file, kdl })),
  reads: trace.reads(),
  locate: (file) => (file.startsWith(`${root}/`) ? { repository: 'acme/fleet', path: relative(root, file) } : undefined),
  texts: true,
})
```

Each KDL node with its own authoring site gets a `span` and `frames`, outermost first: the `declaration` call site in the subject module, `helper` call sites, and the `value` that was read; `constructor` and `fragment` frames name the call or the reused step object that authors it. Nodes are keyed by their KDL path (`mission[ID]/step[ID]/goal[0]`): agents, missions, steps and gates by their first string argument, other nodes by their index among siblings of the same name. Unlisted nodes inherit the nearest listed ancestor. Positions are 1-based lines and UTF-16 columns with exclusive ends.

The AST pass follows the default export through constructor calls (`Fractal.mission({…})`, `define*({ meta, intent })`), consts and static imports to the authoring object, its properties and array elements, and step objects by their `id`. The read trace covers values that helpers compute: it wraps literal object and array consts only, never frozen values, and nested values only where the initializer is itself a literal, so step and gate handles keep their identity. Values pass through unchanged, so the rendered KDL is byte-identical with and without the trace; `src/source-map.unit.test.ts` checks that.

## Relation to `@fractal/declarations`

[`@fractal/declarations`](../../clients/typescript/fractal-declarations/README.md) is the generated, dependency-free encoder for a subset of the grammar. This package covers more of the grammar (gates, loops, inputs and products, documents, observers, calendar schedules) and path-derived trees. For the shared subset both produce the same declarations; `src/parity.unit.test.ts` and `crates/fractal/tests/validate.rs` check that. [COVERAGE.md](./COVERAGE.md) records grammar coverage and conformance evidence; [docs/requirements.md](./docs/requirements.md) and [docs/spec.md](./docs/spec.md) hold the contract.

## Programs

`fractal/effect` holds the program form of a mission (design: [`docs/st3/program-engine.md`](../../docs/st3/program-engine.md)), built on Effect's `effect/workflow`. A program's body branches on step values. Its bundle runs in the daemon's program host, which journals every step and replays the journal after a restart; this package implements the bundle's side of that host contract.

```ts
import { Effect, Schema } from 'effect'
import { Fractal } from 'fractal/effect'

import factory from '../agent.ts'

export default Fractal.mission(
  {
    meta: import.meta,
    id: 'factory/land',
    goal: 'Land one change on main.',
    reportTo: factory,
    input: Schema.Struct({ change: Schema.String }),
    output: Schema.Struct({ commit: Schema.String }),
  },
  ({ change }) =>
    Effect.gen(function* () {
      const out = yield* Fractal.step(
        { id: 'merge', success: Schema.String },
        Effect.map(Fractal.command(['./merge', change]), (result) => result.stdout.trim()),
      )
      return { commit: out }
    }),
)
```

| Call | `effect/workflow` | Journal |
|---|---|---|
| `Fractal.mission(intent, program)` | a `Workflow` with `intent.input` as payload and `intent.output` as success | the success, encoded with `output`, or the typed failure fails the program step |
| `Fractal.step({ id, success?, error? }, effect)` | an `Activity` named by its key: `id`, then `id#2`… for repeats, so `Effect.retry` runs it again under a new key | its encoded value, or its typed failure with `_tag` and fields; defects and interrupts are not journaled |
| `Fractal.step({ id, success }).pipe(Effect.provide(Fractal.Worker.layer({ mission, input, product })))` | a `DurableDeferred` | a worker turn: a linked child run of `mission`; the product `resource/<child run>/<product>`, decoded with `success`, is the value |
| `Fractal.ask({ id, for, title, request, deadline?, default? })` | a `DurableDeferred` | a native person ask on the program step, answered `AskAnswer` or failed `AskCancelled` |
| `Fractal.run(mission, input, { id? })` | a `DurableDeferred` | a linked child run, `ChildRun` or `ChildFailed` |
| `Fractal.command(argv, { env?, cwd?, priority? })` | — | not journaled: run it inside a step; the host runs it as a process group it ends on cancel |

Every `input` field must be required and encode to a string, since each one is a native `input "name" kind="text"`; a payload that does not decode refuses the run with `InvalidInput`. Without `error`, a step or program journals any tagged error as `_tag` plus its fields and the step fails with that plain object (typed `Journaled<E>`), live and on replay, so `catchTag` matches; an untagged error is a defect. With `error`, the schema rebuilds the error as its class, which a program's `error` class needs. Only one fiber may use a step id: concurrent branches would take `id#n` keys in completion order, which replay does not reproduce, so a second branch using the id is a defect. A journal entry the daemon refuses ends the attempt with a defect that the body cannot catch.

A program mission's default export carries `[ProgramEntry]` (`Symbol.for('fractal.program.v1')`), the entry the program host calls with a `HostContextV1`. `fixtures/programs/factory/land-program` is the factory's landing program; `src/program-bundle.unit.test.ts` runs its bundle against an in-memory host.

### Publishing a program

`loadSubjectTree({ root, context, bundleDir })` bundles every program mission it loads with `bun build --target=bun --minify-whitespace` (`bun` on `PATH`, or the `bun` option) and writes the bundle to `<bundleDir>/<sha256>.mjs`; `bundleDir` must be absolute, and a tree with a program mission and no `bundleDir` is refused. Minified whitespace drops the comment Bun writes above each inlined module with its path from the build directory, so the same sources and lockfile give the same bytes wherever the tree and its dependencies are checked out. Before it pins the bundle the loader checks that its default export carries the entry. The mission then loads like any other, so `emit([mission(intent)])` renders it:

```kdl
mission "factory/land-program" report-to="agent/factory" state="ready" timeout="2h" {
  concurrent-runs max=8
  input "change" kind="text"
  input "reviewer" kind="text"
  goal "Land one change on main and prove it live, or leave main as it was."
  authority {
    publish "scratch/*"
    start "factory/step/review"
    write "resource/factory/*"
  }
  step "program" timeout="2h" {
    program "/bundles/<sha256>.mjs" sha256="<sha256>" stalled-after="10m"
  }
}
```

The intent's `timeout` bounds the mission and its program step, `stalledAfter` becomes the program's `stalled-after`, `concurrentRuns` becomes `concurrent-runs max=N`, and `authority` (`publish`, `start`, `revise` and `write` rules, rendered in canonical order) becomes the run authority block. `fractal validate` checks the KDL and that each bundle still has its pinned sha256; `fractal apply` publishes the mission and carries the bundle to the daemon. `fixtures/land-program.kdl` is the landing program's rendering; after an intended change to the program or to code its bundle inlines, `FRACTAL_UPDATE_GOLDENS=1 pnpm --filter fractal test` rewrites it.

A loader that walks its own tree renders one program mission the same way with `programMission({ root, file, definition, bundleDir, bun? })`: `file` is a `mission.ts` below the tree `root`, `definition` its default export, and the subject ID its real directory below `root`'s real path. The result is `{ kdl, bundle, sha256 }`, the bytes `loadSubjectTree` would render for that subject.

## Checks

```sh
nix develop .#web -c bash -c 'pnpm install --frozen-lockfile && pnpm --filter fractal typecheck && pnpm --filter fractal test'
cargo test -p fractal --test validate
```
