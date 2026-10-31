---
title: "Products and gates"
description: "Declare the evidence and decisions that make work complete."
order: 6
---

# Products and gates

KDL is the runnable format. Effect and TS are preview APIs that illustrate the design target, not an installed SDK. Use KDL for the runnable commands in these guides.

A worker's completion report is a report, not a verdict. **Products** describe promised graph state; **gates** decide whether finished work can pass. All declared products must hold and all gates must pass.

## Products

`produces` names state the work promises to create, such as a resource in a particular state. Fractal checks the promise against the graph. Until it holds, the step remains in verification with the unmet conditions visible.

## Gates

- **Graph predicates** check recorded facts.
- **Mechanical gates** run a command with a time limit.
- **LLM gates** ask a model to judge with explicit limits and tools.
- **Human gates** ask a named person to decide.

A graph or human gate can remain pending until there is enough evidence. A failed gate is not success just because the worker reported completion.

## Human review

Inside a mission, place this after a worker's `prepare` step:

```ts group=approval variant=effect
import { Effect } from 'effect'
import { Fractal } from 'fractal/effect'

const ada = Fractal.person({ id: 'ada' })

// Sequence after the prepare step inside the mission.
export const approve = Fractal.step(
  { id: 'approve' },
  Effect.gen(function* () {
    const answer = yield* Fractal.ask({
      name: 'The operator approves',
      reviewer: ada,
      question: 'Is the result ready to use?',
    })
    if (answer._tag === 'Rejected') {
      yield* Effect.fail(new Error('Result rejected'))
    }
  }),
)
```

```ts group=approval variant=ts
import { Fractal } from 'fractal'

const ada = Fractal.person({ id: 'ada' })

// Await after the prepare step inside the mission.
export const approve = () => Fractal.step(
  { id: 'approve' },
  async () => {
    const answer = await Fractal.ask({
      name: 'The operator approves',
      reviewer: ada,
      question: 'Is the result ready to use?',
    })
    if (answer._tag === 'Rejected') {
      throw new Error('Result rejected')
    }
  },
)
```

```kdl group=approval variant=kdl
step "approve" {
  agentless
  depends-on { step "prepare" completed }
  gate "The operator approves" type="human" {
    reviewer "person/ada"
    question "Is the result ready to use?"
  }
}
```

The request appears in the reviewer's [Now](attention.md) view with its available actions. The agent remains free for other ready work while the review waits.

Declare checks before execution so the acceptance standard is visible to everyone. Read the [mission language reference](https://github.com/compoundingtech/fractal/blob/main/docs/st3/mission-graph-runtime.md) for exact gate syntax and [Missions in practice](../guides/missions-in-practice.md) for a complete review flow.
