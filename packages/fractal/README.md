# fractal

Typed authoring for Fractal declarations: agents, missions and schedules kept as TypeScript modules in a directory tree, validated with Effect schemas and rendered to KDL v2. This is the private workspace package `fractal`; it is not published to npm.

| Entrypoint | Runtime | Contents |
|---|---|---|
| `fractal` | isomorphic | The `Fractal` authoring namespace; declaration schemas and the KDL lowering (`agent`, `mission`, `schedule` nodes, `emit`, `fractalKdl`) as named exports |
| `fractal/tree` | Node | `loadSubjectTree` plus the define forms and reference types |
| `fractal/testing` | Node | `scratchDaemonLayer`, an isolated daemon for conformance tests |
| `fractal/effect` | Bun program host, isomorphic | The `Fractal` namespace with the program forms of `mission` and `step`, plus `ask`, `run`, `command`, `env`, `cwd` and `Worker`; the host contract (`ProgramEntry`, `HostContextV1`) as named exports |

## Authoring

Each declaration is the default export of `<id>/agent.ts`, `<id>/mission.ts` or `<id>/schedule.ts` below a tree root. `meta: import.meta` ties the declaration to its module; the loader derives the ID from the module's directory, so declarations never repeat it. Imported declarations are typed references.

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

The namespace also holds `step`, `gate`, `person`, `input`, `product`, `t`, `doc`, `pr`, `childMission`, `runId`, `completed`/`failed`/`terminal`, `Timing`, `Launch`, `Review`, `Fragment`, `Mission`, `Round`, `omp` and `pinMission`. `Fractal.defineAgent`, `Fractal.defineMission` and `Fractal.defineSchedule` are the low-level forms: `{ meta, intent }`, where `intent` may be a function of the loader's `context`.

`step()` and `gate.*()` validate eagerly, so a step handle's `assignedTo` must be an object with an `id`; a path-derived reference works in an inline step object, which the loader lowers.

## Loading and rendering

`loadSubjectTree({ root, context })` imports every declaration module below `root`, lowers typed references to IDs without copying authored step and gate handles, assembles missions with `missionWire`, and decodes the result with the strict schemas. Render a loaded subject with the named exports:

```ts
import { agent, emit, mission } from 'fractal'

const kdl = emit([subject.kind === 'agent' ? agent(intent) : mission(intent)])
```

Validate the output with `fractal validate` before publishing it with `fractal apply`.

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
| `Fractal.step({ id, success }).pipe(Effect.provide(Fractal.Worker.layer({ mission, input, product })))` | a `DurableDeferred` | a worker turn: a linked child run of `mission`; the product `<child run>/<product>`, decoded with `success`, is the value |
| `Fractal.ask({ id, for, title, request, deadline?, default? })` | a `DurableDeferred` | a native person ask on the program step, answered `AskAnswer` or failed `AskCancelled` |
| `Fractal.run(mission, input, { id? })` | a `DurableDeferred` | a linked child run, `ChildRun` or `ChildFailed` |
| `Fractal.command(argv, { env?, cwd?, priority? })` | — | not journaled: run it inside a step; the host runs it as a process group it ends on cancel |

Every `input` field must be required and encode to a string, since each one is a native `input "name" kind="text"`; a payload that does not decode refuses the run with `InvalidInput`. Without `error`, a step or program journals any tagged error as `_tag` plus its fields and replays it as that plain object, so `catchTag` matches; an untagged error is a defect. With `error`, the schema rebuilds the error on replay. Two concurrent steps with one id are a defect, because their keys would depend on scheduling.

A program mission's default export carries `[ProgramEntry]` (`Symbol.for('fractal.program.v1')`), the entry the program host calls with a `HostContextV1`; `fixtures/programs/factory/land-program` is the factory's landing program, and `src/program-bundle.unit.test.ts` runs its `bun build --target=bun` bundle. Bundling a program and rendering its `program` step declaration are not part of `loadSubjectTree` yet.

## Checks

```sh
nix develop .#web -c bash -c 'pnpm install --frozen-lockfile && pnpm --filter fractal typecheck && pnpm --filter fractal test'
cargo test -p fractal --test validate
```
