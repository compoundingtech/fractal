---
title: "Your first mission"
description: "Publish a finite goal, start a run, and inspect the result."
order: 5
---

# Your first mission

KDL is the runnable format. Effect and TS are preview APIs that illustrate the design target, not an installed SDK. Use KDL for the runnable commands in these guides.

Use the worker from [Your first agent](first-agent.md). Run this in its workspace and substitute your own person name for `ada`.

## Declare a small result

```sh
cd "$HOME/fractal/garden"
mkdir -p missions
```

Save the KDL variant as `missions/first-note.kdl`.

```ts group=first-note variant=effect
import { Effect } from 'effect'
import { Fractal } from 'fractal/effect'
import { worker } from '../worker'

const ada = Fractal.person({ id: 'ada' })

export const mission = Fractal.mission(
  {
    id: 'garden/first-note',
    state: 'ready',
    timeout: '1h',
    reportTo: ada,
    goal: 'WELCOME.md introduces this workspace in one short paragraph.',
    constraints: [
      'Only create WELCOME.md. Do not modify other files or publish anything.',
    ],
  },
  () => Effect.gen(function* () {
    yield* Fractal.step({
      id: 'write',
      timeout: '15m',
      goal: 'WELCOME.md exists and contains a short introduction to this workspace.',
    }).pipe(Effect.provide(Fractal.Worker.layer(worker)))
  }),
)
```

```ts group=first-note variant=ts
import { Fractal } from 'fractal'
import { worker } from '../worker'

const ada = Fractal.person({ id: 'ada' })

export const mission = Fractal.mission(
  {
    id: 'garden/first-note',
    state: 'ready',
    timeout: '1h',
    reportTo: ada,
    goal: 'WELCOME.md introduces this workspace in one short paragraph.',
    constraints: [
      'Only create WELCOME.md. Do not modify other files or publish anything.',
    ],
  },
  async () => {
    await Fractal.Worker.provide(worker, async () => {
      await Fractal.step({
        id: 'write',
        timeout: '15m',
        goal: 'WELCOME.md exists and contains a short introduction to this workspace.',
      })
    })
  },
)
```

```kdl group=first-note variant=kdl
version 2
mission "garden/first-note" state="ready" {
  goal "WELCOME.md introduces this workspace in one short paragraph."
  constraint "Only create WELCOME.md. Do not modify other files or publish anything."
  step "write" timeout="15m" {
    assigned-to "agent/garden/worker"
    goal "WELCOME.md exists and contains a short introduction to this workspace."
  }
}
```

```sh
fractal apply missions/first-note.kdl --as person/ada --dry-run
fractal apply missions/first-note.kdl --as person/ada
fractal missions start garden/first-note --id garden/first-note/one \
  --workspace "$PWD" --as person/ada
fractal missions show mission-run/garden/first-note/one
```

Publication stores an immutable definition; starting creates one execution pinned to it. The complete run path is `garden/first-note/one`, not just `one`.

## Check the result

```sh
fractal work ls
fractal agents queue agent/garden/worker
fractal missions show mission-run/garden/first-note/one
cat WELCOME.md
```

Wait for the run to finish. Success means the run reports completion **and** the file contains the requested paragraph. A published mission, an idle worker, or an accepted start request alone is not success. Inspect the reported step result and check that only the authorized file changed.

If the agent is blocked, inspect its conversation and [troubleshoot](../guides/troubleshooting.md); do not start another run to disguise the first failure.

Next, add dependencies and a human review in [Missions in practice](../guides/missions-in-practice.md), or learn how [gates](../concepts/gates.md) make verification part of the plan.
