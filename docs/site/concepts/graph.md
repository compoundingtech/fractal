---
title: "The graph"
description: "Durable claims preserve intent, observations, and decisions across process lifetimes."
order: 7
---

# The graph

The graph is Fractal's durable memory: an append-only history of immutable **claims**. A claim is a typed fact about a **subject**, such as `agent/example/worker`, `mission-run/example/first-note/one`, or `person/ada`.

Declarations, observed runtimes, claimed work, messages, and gate decisions all have durable identities and history.

## Current state is derived

`fractal missions ls` and `fractal work ls` show projections of recorded facts. An agent queue or run status can be rebuilt from history; it is not a fact held only in one process's memory.

```sh
fractal missions show mission-run/example/first-note/one
fractal subject show agent/example/worker
```

A repair records a replacement fact rather than rewriting the original history. Keep declarations and observations distinct: “should be running” is not evidence that a process is alive now.

## Across machines

Each daemon has its own local graph. Fleet members exchange authenticated claims and catch up after reconnecting. Replication moves graph state, not repository files or native provider sessions. A restarted daemon can recover the declared plan without pretending that a lost harness conversation survived.

## Two meanings of claim

A graph **claim** is a recorded fact. An agent **claiming a step** is taking a lease on work, which is itself recorded as facts. Context distinguishes them.

Results belong in step completion or a cited graph document. Messages coordinate people and agents; they do not replace evidence that a gate can check.

See the [schema](https://github.com/compoundingtech/fractal/blob/main/docs/st3/schema.md) for the engineering model.
