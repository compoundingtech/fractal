---
title: "The software factory"
description: "How goals become coordinated work, verified results, and useful human decisions."
order: 2
---

# The software factory

A **software factory** is a system that produces software to meet a set of goals. It is not just a prompt or a model. It includes the system that decides what to do, keeps execution running, checks the output, and brings the right decision to a person.

Fractal provides an open framework for building your own factory rather than prescribing a single workflow or coding harness.

## How a mission becomes running work

1. Declare an [agent](agents.md) and publish a [mission](missions.md), or use [launch](launch.md) to draft one from intent.
2. Start a run. Fractal records the plan in [the graph](graph.md).
3. The daemon reconciles desired state with actual processes. Harnesses run their real interactive interfaces inside persistent `pty` sessions.
4. When a step is ready, the assigned agent receives a durable wake through its native integration and claims the work.
5. The agent does the work and records its result. [Products and gates](gates.md) decide whether it passes.
6. Run-owned processes stop when their owner ends. Independently declared agents stay available for other missions.

The graph, not a terminal or process, is the durable source of truth. A daemon restart can adopt surviving terminal sessions and recover the current work.

## Runtime and transport

[pty](https://compounding.tech/docs/pty/) keeps programs alive and lets you attach to them. [Fabric](https://compounding.tech/docs/fabric/) connects trusted machines. They are separate tools and useful independently of Fractal.

Each Fractal daemon is a complete local system. A [fleet](../guides/two-machines.md) connects your machines and replicates authenticated graph claims. Workspaces and repository checkouts are not copied by graph replication.

## Adopt one workflow at a time

Choose a bounded goal, state the constraints you already know, and name the evidence you will accept. Start with a single agent and a finite mission. Add independent reviewers, mechanical gates, event intake, or more machines only as the work calls for them.
