---
title: "Your first mission"
description: "Publish a finite goal, start a run, and inspect the result."
order: 5
---

# Your first mission

KDL is the runnable format. TS samples declare the same file with the in-repo [`fractal` package](https://github.com/compoundingtech/fractal/tree/main/packages/fractal), which renders exactly the KDL shown; it is not published to npm yet. Effect mission samples show the program form, `Fractal.mission(intent, program)`, a roadmap API that does not run yet. Use KDL for the runnable commands in these guides.

Use the worker from [Your first agent](first-agent.md). Run this in its workspace and substitute your own person name for `ada`.

## Declare a small result

```sh
cd "$HOME/fractal/garden"
mkdir -p missions
```

Save the KDL variant as `missions/first-note.kdl`.

```ts group=first-note variant=effect
// Roadmap: program-form missions do not run yet. The TS tab declares this mission as data.
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
// garden/first-note/mission.ts
import { Fractal } from 'fractal'

import worker from '../worker/agent.ts'

export default Fractal.mission({
  meta: import.meta,
  state: 'ready',
  reportTo: worker,
  goal: 'WELCOME.md introduces this workspace in one short paragraph.',
  constraints: ['Only create WELCOME.md. Do not modify other files or publish anything.'],
  steps: [
    {
      id: 'write',
      timeout: '15m',
      assignedTo: worker,
      goal: 'WELCOME.md exists and contains a short introduction to this workspace.',
    },
  ],
})
```

```kdl group=first-note variant=kdl
version 2
mission "garden/first-note" report-to="agent/garden/worker" state="ready" {
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

The run workspace is a path on its execution members, not necessarily the computer
where you type the command. For missions whose declared runtimes or executable
gates run on another member, or whose placement cannot be determined before
starting (including referenced missions), pass an absolute `--workspace` on those members;
the CLI publishes it as written without checking the caller's filesystem.
Omitting it or passing a relative path is refused rather than publishing the
caller's current directory. Provably local execution still defaults to the current
directory and canonicalizes local paths. Placement uses actual step paths and
supplied inputs, never fabricated interpolation values; unresolved host expressions
require an explicit workspace. The same rule applies to `launch run`
and `launch approve-and-launch`; `launch start` creates a local planning agent.
Approved launches keep using their pinned candidate even if the mission is renamed.

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
