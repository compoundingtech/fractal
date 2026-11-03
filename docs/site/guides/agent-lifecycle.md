---
title: "Agent lifecycle"
description: "Choose restart, stop, or suspend without confusing durable work with native sessions."
order: 4
---

# Agent lifecycle

An agent's declaration, messages, and decisions live in the graph. Its harness has a native session too. Recovering durable work and preserving the exact native conversation are different guarantees.

Inspect first:

```sh
fractal agents show agent/garden/worker
fractal agents queue agent/garden/worker
```

## Restart

```sh
fractal agents restart agent/garden/worker --as person/ada
```

Use restart for a stuck harness or changed launch settings. A ready agent is asked for handoff before restart. Native continuation is used where possible; otherwise the new runtime recovers handoff and work from the graph. Repeated failed starts can park an agent. An explicit restart authorizes a new attempt, not an unlimited retry loop.

## Stop and start

```sh
fractal agents stop agent/garden/worker --as person/ada
fractal agents start garden/worker --as person/ada
```

Stop when you want the agent off. Reapplying its declaration can start it again; removing the file does not retire the graph declaration.

## Suspend and resume

Wait until the agent finishes its turn and holds no claimed work:

```sh
fractal agents suspend agent/garden/worker --as person/ada --reason 'Idle until tomorrow.'
fractal agents show agent/garden/worker
fractal agents resume agent/garden/worker --as person/ada
```

Suspend refuses active turns, claims, pending questions, unsent input, or running subagents. Resume verifies the saved native session on the same host. Mail can wait while suspended; it does not wake the agent.

Daemon restart normally adopts surviving harnesses. A process exit, stop request, and observed clean completion are not interchangeable. Read the actual end reason and diagnostics before deciding to restart.

For detailed continuity guarantees, see [native-session continuity](https://github.com/compoundingtech/fractal/blob/main/docs/st3/seat-continuity.md) and [suspend/resume](https://github.com/compoundingtech/fractal/blob/main/docs/st3/suspend.md).
