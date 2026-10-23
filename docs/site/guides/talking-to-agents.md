---
title: "Talking to agents"
description: "Send durable messages and distinguish conversation from tracked work and decisions."
order: 3
---

# Talking to agents

Messages are durable conversation. Missions track work and results; alerts track decisions that need a person. Use each for its own purpose.

## In the terminal interface

Run `fractal`, find an agent with **Ctrl+K**, and open its conversation. Type in the composer and send. **Ctrl+H** opens Now. When attached to a live agent terminal, detach before using application navigation shortcuts.

An agent's native terminal is useful for login, trust, and provider screens. It is not the only way to inspect its work: missions, messages, and results live in the graph.

## From the CLI

```sh
fractal conversations send agent/garden/worker --from person/ada \
  --subject 'README review' --body 'Explain the changes and point me to the result.'
fractal conversations ls person/ada
```

Use your configured person identity. A running agent uses its bound identity rather than impersonating a person. `fractal conversations --help` lists reading, following, replying, and attachment options.

Successful delivery is not proof that the agent read or completed the request. Check its reply and, for tracked work, the run's state and evidence.

## Ask for a human answer

Known decisions belong in a mission's human gates. When a new decision arises, the working agent uses `fractal work ask` with a structured request and the claimed step. The answer is stored with the work so it can resume with a clear decision.

The request should state the question, why a person is needed, relevant evidence, and consequences of each answer. Reuse the request's idempotency key when retrying the same question. Do not turn a message into an implicit approval.

A person reads and answers the resulting card in Now, or uses the exact actions displayed by `fractal alerts ls`. See the [mission runtime contract](https://github.com/compoundingtech/fractal/blob/main/docs/st3/mission-graph-runtime.md) for structured request details.

## On the phone

A paired [iOS client](ios-app.md) can show conversations and actionable requests. Confirm it is connected before sending an action. Use the paired-only gateway; do not expose the privileged daemon socket.
