# fractal

Typed authoring for Fractal declarations: agents, missions, schedules and objectives kept as TypeScript modules in a directory tree, validated with Effect schemas or the native readers' rules and rendered to KDL v2. This is the private workspace package `fractal`; it is not published to npm.

| Entrypoint | Runtime | Contents |
|---|---|---|
| `fractal` | isomorphic | The `Fractal` authoring namespace; declaration schemas and the KDL lowering (`agent`, `mission`, `schedule`, `objectiveNode` nodes, `emit`, `fractalKdl`) as named exports |
| `fractal/tree` | Node | `loadSubjectTree` plus the define forms and reference types |
| `fractal/testing` | Node | `scratchDaemonLayer`, an isolated daemon for conformance tests |

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

An objective (`docs/st3/objectives.md`) is a target over something Fractal measures. A fleet declares intervals over claims, named `NAMESPACE/NAME` by their directory; `objective.ts` needs no `meta` because nothing references an objective, and a `name`, if given, must equal the path ID.

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

`Fractal.objective` checks the rules the native reader enforces and throws on the first broken one. The other indicators (`latency`, `share`, `statement`, `transaction`, `writerHold`, `cpu`, `database`, `divergence`, `ciDuration`, `ciCount`) are native telemetry: Fractal ships them in [`slo/objectives.kdl`](../../slo/objectives.kdl), rendered by [`slo/objectives.kdl.genie.ts`](../../slo/objectives.kdl.genie.ts), and the tree loader refuses them. `src/objective.ts` and `src/kdl.ts` import nothing outside the package, so Genie bundles that file without installing dependencies.

The namespace also holds `step`, `gate`, `person`, `input`, `product`, `t`, `doc`, `pr`, `childMission`, `runId`, `completed`/`failed`/`terminal`, `Timing`, `Launch`, `Review`, `Fragment`, `Mission`, `Round`, `omp` and `pinMission`. `Fractal.defineAgent`, `Fractal.defineMission`, `Fractal.defineSchedule` and `Fractal.defineObjective` are the low-level forms: `{ meta, intent }`, where `intent` may be a function of the loader's `context`.

`step()` and `gate.*()` validate eagerly, so a step handle's `assignedTo` must be an object with an `id`; a path-derived reference works in an inline step object, which the loader lowers.

## Loading and rendering

`loadSubjectTree({ root, context })` imports every declaration module below `root`, lowers typed references to IDs without copying authored step and gate handles, assembles missions with `missionWire`, decodes the result with the strict schemas, and checks objectives with `Fractal.objective`. Render a loaded subject with the named exports:

```ts
import { agent, emit, mission, objectiveNode } from 'fractal'

const kdl = emit([subject.kind === 'agent' ? agent(intent) : mission(intent)])
const objectiveKdl = emit([objectiveNode({ ...intent, name: subject.id })])
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
