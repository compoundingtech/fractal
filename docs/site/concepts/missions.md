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

`fractal apply mission.kdl --as person/ada` saves the definition. Initial publication does not start work. `fractal missions start` creates a run with its own identity, workspace, and history; later new-revision publication supersedes active stale runs by default, respecting existing revision policies as described below. Read [Your first mission](../getting-started/first-mission.md) for the full procedure.

Give goals observable end states. Keep operational instructions in constraints or documents named by the mission. A mission should say what must be true, not merely that an agent should try.

## Revising a running mission

For a mission without a revision policy, publishing a **new revision automatically supersedes already-active runs on older revisions**. Existing human-only review prevents automatic supersession; a person must approve the exact proposal before a fresh linked run is admitted. A when-idle policy waits for current work to drain in the existing `revision-draining` phase, rather than cancelling immediately. Before cancelling anything, cutover checks whether a fresh run can start with each source's exact resolved inputs, input declarations and kinds, workspace, capacity, authority, and interpolation. If admission fails, the source stays active and `supersession_fault` names both revisions and the incompatibility.

If admission succeeds, the source follows ordinary cancellation, finalization, and cleanup. Its linked fresh replacement waits in `supersession-waiting` until the source is terminal, then restarts every step and program from the top. No successful steps or program journals carry over. Effects already present in the workspace or external systems remain: work must be idempotent. Native JSON, public mission summaries, and `fractal missions show` expose `supersedes`, `superseded_by`, and any `supersession_fault`.

Republishing the same revision does nothing. Supersession responds to publication and declared policy boundaries, not a continuous stale-run scan: an explicit `retry --same-revision` started after a newer publication remains on the selected revision until another new revision is published. Unreviewed direct run revision remains a separate generation-adoption operation that may preserve compatible work; approved proposal cutover creates a fresh linked run. Every generation still pins its code and journal. No additional data-DSL syntax is required.

## Retrying a failed run

`fractal missions retry RUN --as ACTOR` creates a **new run**, using the latest published mission revision by default. Add `--same-revision` to use the source run's current revision instead. The source must be failed and terminal; a person or the owning agent uses the same authority as an ordinary run start.

The new run keeps the source's exact resolved inputs and workspace, not new defaults from the selected revision. If those inputs do not fit the latest revision, retry refuses with both revision IDs and the incompatibility reason. It never falls back to an older revision.

Every step and the program restart from the top. Make steps idempotent against effects already present in the shared workspace or external systems; retry does not undo those effects or carry completed work forward. The source's status, generation, and history stay unchanged. JSON and `fractal missions show` expose `retry_of` on the new run and `retried_by` on its source so either execution can be traced to the other.

## Finite and continuing work

A finite mission ends when its work and checks finish. For continuing event intake, use a deliberate retirement gate to keep subscriptions alive until you choose to stop them. Do not create an empty mission to keep an agent available: declare an independent agent instead.

See [Missions in practice](../guides/missions-in-practice.md) for dependencies, review, revisions, and queued runs.
