---
title: "Scripted demos"
description: "Rehearse a workflow in an isolated instance without model tokens."
order: 12
---

# Scripted demos

A demo seeds invented agents, missions, conversations, and documents, then plays visible beats in the terminal interface. Agents are scripted rather than model-backed, so the result is repeatable and consumes no model tokens.

From a source checkout:

```sh
fractal demo check examples/demos/release-day.kdl
fractal --instance demo demo play examples/demos/release-day.kdl
```

Use a dedicated instance, never your working fleet. Every play resets the demo-owned instance. Fractal refuses to erase pre-existing state that is not marked as demo-owned; choose a fresh instance rather than overriding the refusal for an important store.

Run the demo from your own terminal, outside a managed agent. The demo needs to act as its invented identities; a bound agent must not override its identity to play it. Its login shell must find `pty`.

## Controls

- **F8**, or the file's configured advance key, fires the next key-driven beat.
- During a timed wait, advance fires the next beat immediately.
- Normal interface actions still work; you can open conversations and answer alerts.
- **Ctrl+Q** ends the demo and stops its daemon and scripted agents.

After quitting, inspect any reported failed beats. A video that looks right does not prove every scripted action succeeded.

## Authoring

A file starts with `version 2`, then a `demo` block for the title, person, machine, and advance key; `seed` describes initial graph state and `beats` describes visible actions. Beats can wait for a key or a delay. Demo declarations cannot run arbitrary harnesses, commands, or gates.

Use the checked-in [demo examples](https://github.com/compoundingtech/fractal/blob/main/examples/demos) as complete authoring examples. `fractal demo check FILE` validates the file and lists its beats without playing it.
