---
title: "Steps and work"
description: "Use dependencies and explicit ownership to coordinate units of work."
order: 5
---

# Steps and work

KDL is the runnable format. TS samples use the data form of the in-repo [`fractal` package](https://github.com/compoundingtech/fractal/tree/main/packages/fractal), which renders exactly the KDL shown; it is not published to npm yet. Effect samples show the program form, a roadmap API that does not run yet. Use KDL for the runnable commands in these guides.

A **step** is one unit of work in a mission. It states an outcome, who may do it, and what must happen first.

## Dependencies determine order

```ts group=steps variant=effect
// Roadmap: program-form missions do not run yet. The TS tab declares these steps as data.
import { Effect } from 'effect'
import { Fractal } from 'fractal/effect'
import { worker, reviewer } from './agents'

export const steps = Effect.gen(function* () {
  yield* Fractal.step({
    id: 'write',
    timeout: '30m',
    goal: 'The release notes describe the changes.',
  }).pipe(Effect.provide(Fractal.Worker.layer(worker)))

  yield* Fractal.step({
    id: 'review',
    timeout: '20m',
    goal: 'The release notes have an evidence-backed review.',
  }).pipe(Effect.provide(Fractal.Worker.layer(reviewer)))
})
```

```ts group=steps variant=ts
// The steps of a Fractal.mission({ ... }); worker and reviewer are imported from their agent.ts files.
steps: [
  {
    id: 'write',
    timeout: '30m',
    assignedTo: worker,
    goal: 'The release notes describe the changes.',
  },
  {
    id: 'review',
    timeout: '20m',
    assignedTo: reviewer,
    dependsOn: [{ step: 'write', state: 'completed' }],
    goal: 'The release notes have an evidence-backed review.',
  },
],
```

```kdl group=steps variant=kdl
step "write" timeout="30m" {
  assigned-to "agent/example/worker"
  goal "The release notes describe the changes."
}
step "review" timeout="20m" {
  assigned-to "agent/example/reviewer"
  depends-on {
    step "write" "completed"
  }
  goal "The release notes have an evidence-backed review."
}
```

In KDL, textual order is for presentation, not scheduling. The TS sample declares this dependency with `dependsOn`; the roadmap program form expresses it by running the write step before starting the review. Without a dependency a step can become ready immediately. Separate agents are required if independent work should run at the same time.

## Claims are leases

An agent claims ready work before doing it. A claim ties ownership to a runtime incarnation and is renewed while that incarnation remains live. This lets work recover after a failed runtime without treating two processes as the same owner.

Record progress and completion in the graph. Conversation is useful coordination, but a message saying “done” is not a step result or gate verdict.

## Small work and planned work

Use `fractal work start` for a small independent authorized job. Use a mission when you need dependencies, distinct owners, gates, or review. `fractal work ls` shows ready and active work; `fractal agents queue agent/example/worker` shows how a worker's runs are ordered.

An `agentless` step waits on its declared conditions or gates without consuming a worker. This is useful for [human review](gates.md) while agents continue with other work.
