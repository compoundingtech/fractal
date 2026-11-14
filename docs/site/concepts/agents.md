---
title: "Agents"
description: "A durable harness, model, and workspace that can serve many missions."
order: 3
---

# Agents

KDL is the runnable format. TS and Effect samples declare the same file with the in-repo [`fractal` package](https://github.com/compoundingtech/fractal/tree/main/packages/fractal), which renders exactly the KDL shown; it is not published to npm yet. Use KDL for the runnable commands in these guides.

An **agent** is a durable identity backed by a coding harness, a model, and a workspace. No single mission owns an independently declared agent, so finishing a mission does not stop it.

```ts group=worker variant=effect
// example/worker/agent.ts
import { Fractal } from 'fractal'

export default Fractal.agent({
  meta: import.meta,
  workspace: '/work/example',
  restart: 'always',
  harness: { kind: 'claude', model: 'sonnet' },
})
```

```ts group=worker variant=ts
// example/worker/agent.ts
import { Fractal } from 'fractal'

export default Fractal.agent({
  meta: import.meta,
  workspace: '/work/example',
  restart: 'always',
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

## Charter

An agent's `description` is its **charter**: the lasting responsibility it carries across missions, such as what it owns, what it watches, and when it acts without being asked. Mission goals, steps, and messages carry finite tasks; the charter says what the agent is for between them, so keep it a summary rather than a task brief.

```kdl
version 2
agent "example/release-keeper" {
  description """
    Keep main releasable and own its release notes.
    Watch CI on main; fix or report a red build before other work.
    """
  workspace "/work/example"
  harness "claude" {}
}
```

A charter is UTF-8 text of at most 8 KiB (8,192 bytes) and may span several lines. Publishing a charter over 1,000 bytes requires every active fleet member to support it; see [agent charter publication](https://github.com/compoundingtech/fractal/blob/main/docs/st3/compatibility.md#agent-charter-publication).

`fractal agents show ID` prints the charter of the agent's current declaration under **Charter**. A running agent reads its own with `fractal agents show "$ST_AGENT"`. When a restarted seat is told where its previous session was, that message opens with the charter. The charter is never a launch input: editing it publishes the declaration without restarting the agent.

## Access and readiness

Agents use the daemon user's filesystem and provider accounts. Provider permission modes are not an isolation boundary. Install and authenticate the harness before expecting it to become ready. Put instructions in mission goals, constraints, linked documents, or messages, not a harness prompt field.

Follow [Your first agent](../getting-started/first-agent.md) for a runnable example and [Agent lifecycle](../guides/agent-lifecycle.md) for stop, restart, and suspend.
