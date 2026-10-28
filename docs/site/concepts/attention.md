---
title: "Attention"
description: "Find the decisions and requests that need you without watching every agent."
order: 8
---

# Attention

Fractal brings work that needs a named person into **Now**. The goal is a short list of decisions, not a stream of every diagnostic from every agent.

```sh
fractal now
fractal alerts ls --as person/ada
```

## Alerts are actionable

Human gates, explicit asks, launch approvals, and protected revision approvals can wait on your answer. Each alert carries its context and available actions. Use the displayed subject IDs and current actions rather than guessing which run needs approval.

Messages are conversation, not alerts. Read them with `fractal conversations`. Faults belong to their affected subjects and the agents that own them; an owner can ask a person when it cannot resolve one.

## Resolve, do not monitor

Answered or superseded requests no longer need the person's attention. Opening a card does not approve it. Read the evidence and choose the explicit action.

Use planned human gates for known review points. For a decision discovered during work, an agent uses `fractal work ask` to save the question and its eventual answer in the graph.

This makes the human's role explicit: set direction, supply missing judgment, and review meaningful results rather than continuously reading terminal output.
