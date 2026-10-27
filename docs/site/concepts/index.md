---
title: "Understand Fractal"
description: "The building blocks that turn goals into durable, verifiable agent work."
order: 1
---

# Understand Fractal

Fractal is a **software factory framework**. It turns agent work from a collection of terminal sessions into declared, durable, inspectable work on your machines.

A harness crash should not erase the goal. A worker's claim that it finished should not be the only evidence. A person should not need to read every terminal to find the decision that needs them.

New to the idea? [Why a software factory](why-software-factory.md) explains the problem these building blocks solve.

## The building blocks

| Concept | What it contributes |
| --- | --- |
| [Software factory](software-factory.md) | The system around agents that produces software to meet goals. |
| [Agents](agents.md) | Durable identities with a harness, model, and workspace. |
| [Missions and runs](missions.md) | A declared plan and one execution of that plan. |
| [Steps and work](steps.md) | Ownership, dependencies, and leased units of execution. |
| [Products and gates](gates.md) | Explicit conditions for accepting a result. |
| [The graph](graph.md) | Durable facts, history, and derived current state. |
| [Attention](attention.md) | Decisions and requests that need a named person. |
| [Launch](launch.md) | A reviewed path from plain-language intent to a mission. |

## Declarations and observations

A declaration says what should be true: which agent should run, what goal to achieve, or which checks must pass. An observation says what has been seen: a running process, a produced artifact, a test result. Fractal keeps those distinct so an intended state is never mistaken for evidence that it happened.

The daemon reconciles desired state with observed state. The graph keeps the plan and result durable; provider-native harnesses do the actual coding inside persistent terminal sessions.

Start with one machine and a [first mission](../getting-started/first-mission.md). Add infrastructure only when the workflow needs it.
