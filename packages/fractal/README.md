# fractal

Typed authoring for Fractal declarations: agents, missions and schedules kept as TypeScript modules in a directory tree, validated with Effect schemas and rendered to KDL v2. This is the private workspace package `fractal`; it is not published to npm.

| Entrypoint | Runtime | Contents |
|---|---|---|
| `fractal` | isomorphic | The `Fractal` authoring namespace; declaration schemas and the KDL lowering (`agent`, `mission`, `schedule` nodes, `emit`, `fractalKdl`) as named exports |
| `fractal/tree` | Node | `loadSubjectTree` plus the define forms and reference types |
| `fractal/testing` | Node | `scratchDaemonLayer`, an isolated daemon for conformance tests |

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

## Roadmap

Program-form missions, `Fractal.mission(intent, program)` with `Fractal.step(...)` effects and `Fractal.ask`, are a design target shown in the documentation's Effect tab. They have no runtime yet: missions are declared as data.

## Checks

```sh
nix develop .#web -c bash -c 'pnpm install --frozen-lockfile && pnpm --filter fractal typecheck && pnpm --filter fractal test'
cargo test -p fractal --test validate
```
