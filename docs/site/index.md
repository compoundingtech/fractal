---
title: "Introduction"
description: "Build your own software factory with Fractal."
order: 1
---

# Build your own software factory

A **software factory** is a system that produces software to meet a set of goals. **Fractal** is a software factory framework: the open building blocks for declaring work, coordinating coding agents, checking results, and bringing people in when a decision needs them.

Focus on what matters. The factory does the rest.

**Research preview.** Fractal is evolving quickly; expect rough edges and read release upgrade notes before changing a working installation.

## From goals to results

A terminal conversation can help with a task. A factory needs more: durable goals, clear ownership, dependencies, evidence, review, and a way to recover when a process or machine stops. Fractal records those in a graph rather than relying on an agent's terminal history.

1. Declare an [agent](concepts/agents.md): a harness, model, and workspace on your machine.
2. Give it a [mission](concepts/missions.md) with goals, steps, and constraints.
3. Make completion explicit with [products and gates](concepts/gates.md).
4. Follow the result and answer the decisions that need you in [Now](concepts/attention.md).

Fractal works with coding harnesses including Claude Code, Codex, OpenCode, Pi, and Omp. Your agents run on your machines using your provider accounts and quotas. The framework coordinates the work; it does not supply a model subscription.

## Choose your starting point

- **Let your coding agent set it up:** [Set up with your agent](getting-started/with-your-agent.md).
- **Try it yourself:** [Install Fractal](getting-started/install.md), then run a [first mission](getting-started/first-mission.md).
- **Understand it first:** [Understand Fractal](concepts/index.md), then adapt one workflow at a time.

You do not have to automate everything at once. Start with one agent, one finite goal, and one machine. Add [more machines](guides/two-machines.md) or [GitHub intake](guides/github-integration.md) when you need them.

## Community

Fractal is open source. Read the [code](https://github.com/compoundingtech/fractal), [join Discord](https://discord.com/invite/7PbaSUxcy), or follow [Compounding](https://x.com/compoundingtech).
