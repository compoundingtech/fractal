---
title: "Missions and runs"
description: "Separate the declared definition of work from each execution."
order: 4
---

# Missions and runs

KDL is the runnable format. TS samples declare the same file with the in-repo [`fractal` package](https://github.com/compoundingtech/fractal/tree/main/packages/fractal), which renders exactly the KDL shown; it is not published to npm yet. Effect mission samples show the program form, `Fractal.mission(intent, program)`, a roadmap API that does not run yet. Use KDL for the runnable commands in these guides.

A **mission** declares goals, constraints, inputs, and the steps and checks needed to finish. A **run** is one execution pinned to an immutable mission revision.

```ts group=mission variant=effect
// Roadmap: program-form missions do not run yet. The TS tab declares this mission as data.
import { Effect } from 'effect'
import { Fractal } from 'fractal/effect'
import { worker } from '../worker'

const ada = Fractal.person({ id: 'ada' })

export const mission = Fractal.mission(
  {
    id: 'example/first-note',
    state: 'ready',
    timeout: '1h',
    reportTo: ada,
    goal: 'A short project introduction exists in WELCOME.md.',
  },
  () => Effect.gen(function* () {
    yield* Fractal.step({
      id: 'write',
      timeout: '15m',
      goal: 'WELCOME.md contains the project introduction.',
    }).pipe(Effect.provide(Fractal.Worker.layer(worker)))
  }),
)
```

```ts group=mission variant=ts
// example/first-note/mission.ts
import { Fractal } from 'fractal'

import worker from '../worker/agent.ts'

export default Fractal.mission({
  meta: import.meta,
  state: 'ready',
  reportTo: worker,
  goal: 'A short project introduction exists in WELCOME.md.',
  steps: [
    {
      id: 'write',
      timeout: '15m',
      assignedTo: worker,
      goal: 'WELCOME.md contains the project introduction.',
    },
  ],
})
```

```kdl group=mission variant=kdl
version 2
mission "example/first-note" report-to="agent/example/worker" state="ready" {
  goal "A short project introduction exists in WELCOME.md."
  step "write" timeout="15m" {
    assigned-to "agent/example/worker"
    goal "WELCOME.md contains the project introduction."
  }
}
```

## Publish, then start

`fractal apply mission.kdl --as person/ada` saves the definition. It does not start work. `fractal missions start` creates a run with its own identity, workspace, and history. Read [Your first mission](../getting-started/first-mission.md) for the full procedure.

Give goals observable end states. Keep operational instructions in constraints or documents named by the mission. A mission should say what must be true, not merely that an agent should try.

## Revising a running mission

Publishing a new definition does not silently change existing runs. An explicit run revision creates a new generation with preserved history. Finished work carries forward only where it remains compatible with the new plan; changed work and its dependents may need to run again.

## Finite and continuing work

A finite mission ends when its work and checks finish. For continuing event intake, use a deliberate retirement gate to keep subscriptions alive until you choose to stop them. Do not create an empty mission to keep an agent available: declare an independent agent instead.

See [Missions in practice](../guides/missions-in-practice.md) for dependencies, review, revisions, and queued runs.
