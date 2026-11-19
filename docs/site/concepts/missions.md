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

Publishing a new revision automatically supersedes older active runs only when they have no completed work or person responses and their pinned revision allows it. Completed work, answered asks, and human-only revision policy wait for explicit proposal approval; publication cannot remove that protection. A when-idle policy drains active work before cutover, but is not permission to discard completed work. Admission checks the source's resolved inputs, workspace, capacity, authority, and interpolation before cancellation; refusal leaves the source active with a visible `supersession_fault`.

An admitted replacement waits in `supersession-waiting` for its predecessor's ordinary cancellation and confirmed cleanup. Compatible completed steps retain original evidence through `step-run.carried` references; changed contracts and their dependents run again. Bound person answers are reused only with matching ask contracts, resolved inputs, and authority. Historical unbound answers require explicit recovery confirmation. Program journals and unresolved effects are not fabricated or rolled back. Native JSON and `fractal missions show` expose the lineage and any refusal.

Republishing the same revision does nothing. Supersession responds to publication and declared policy boundaries, not a continuous stale-run scan: an explicit `retry --same-revision` started after a newer publication remains on the selected revision until another new revision is published. Unreviewed direct run revision remains a separate generation-adoption operation that may preserve compatible work; approved proposal cutover creates a fresh linked run. Every generation still pins its code and journal. No additional data-DSL syntax is required.

## Retrying a failed run

`fractal missions retry RUN --as ACTOR` creates a **new run**, using the latest published mission revision by default. Add `--same-revision` to use the source run's current revision instead. The source must be failed and terminal; a person or the owning agent uses the same authority as an ordinary run start.

The new run keeps the source's exact resolved inputs. Schedule and subscription launches resolve their workspace from the current launching declaration, preserving the occurrence suffix; explicit API/CLI workspaces and older runs without launch provenance retain their explicit workspace. Incompatible inputs refuse with both revisions and the reason, without selecting an older revision.

Matching completed work carries by reference rather than restarting unchanged work or replaying person answers. Changed step/dependency contracts, heads, inputs, authority, or withdrawn answers refuse adoption. The source's status, generation, and authentic history stay unchanged; `retry_of` and `retried_by` connect the executions.

## Finite and continuing work

A finite mission ends when its work and checks finish. For continuing event intake, use a deliberate retirement gate to keep subscriptions alive until you choose to stop them. Do not create an empty mission to keep an agent available: declare an independent agent instead.

See [Missions in practice](../guides/missions-in-practice.md) for dependencies, review, revisions, and queued runs.
