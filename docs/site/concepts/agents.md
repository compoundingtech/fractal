---
title: "Agents"
description: "A durable harness, model, and workspace that can serve many missions."
order: 3
---

# Agents

KDL is the runnable format. Effect and TS are preview APIs that illustrate the design target, not an installed SDK. Use KDL for the runnable commands in these guides.

An **agent** is a durable identity backed by a coding harness, a model, and a workspace. No single mission owns an independently declared agent, so finishing a mission does not stop it.

```ts group=worker variant=effect
import { Fractal } from 'fractal/effect'

export const worker = Fractal.agent({
  id: 'example/worker',
  lifecycle: 'standing',
  host: 'local',
  workspace: '/work/example',
  harness: { kind: 'claude', model: 'sonnet' },
})
```

```ts group=worker variant=ts
import { Fractal } from 'fractal'

export const worker = Fractal.agent({
  id: 'example/worker',
  lifecycle: 'standing',
  host: 'local',
  workspace: '/work/example',
  harness: { kind: 'claude', model: 'sonnet' },
})
```

```kdl group=worker variant=kdl
version 2
agent "example/worker" {
  workspace "/work/example"
  restart "always"
  harness "claude" {
    model "sonnet"
  }
}
```

Keep the declaration in version control and publish it with `fractal apply worker.kdl --as person/ada`. Use your own person identity and an existing absolute workspace path.

## Identity is not a process

The graph keeps the agent's declaration, messages, work, and decisions. A runtime incarnation is one execution of its harness. Restarting an incarnation does not delete its identity or queued work, but exact native-session continuation depends on the harness and saved session.

Each agent holds one claimed step at a time and has an ordered queue of runs. Independent steps assigned to the same agent still execute serially. Use separate agents and workspaces when work should truly run in parallel.

## Access and readiness

Agents use the daemon user's filesystem and provider accounts. Provider permission modes are not an isolation boundary. Install and authenticate the harness before expecting it to become ready. Put instructions in mission goals, constraints, linked documents, or messages, not a harness prompt field.

Follow [Your first agent](../getting-started/first-agent.md) for a runnable example and [Agent lifecycle](../guides/agent-lifecycle.md) for stop, restart, and suspend.
