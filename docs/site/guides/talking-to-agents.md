---
title: "Talking to agents"
description: "Send durable messages and distinguish conversation from tracked work and decisions."
order: 3
---

# Talking to agents

Messages are durable conversation. Missions track work and results; alerts track decisions that need a person. Use each for its own purpose. The CLI examples use the `garden/worker` seat from [Your first agent](../getting-started/first-agent.md).

## In the web app

Run `fractal web` to open the daemon's [local web app](web-and-ios.md), select an agent, and use its conversation composer. Read the agent's reply and inspect any work or request it refers to. The daemon serves installed web assets by default; the launcher opens a browser session rather than starting another server. Keep its one-time launch URL private.

An agent's native terminal remains useful for login, trust, and provider screens. It is not the only way to inspect its work: missions, messages, and results live in the graph.

In the terminal UI's Agents sidebar, a dim anchor after a seat's name marks an explicitly
declared `lifecycle "standing"` seat in both list and tree views. The anchor uses the MDI
anchor glyph (U+F0031), which requires a Nerd Font. Owner, bounded, unknown, and undeclared
lifecycles have no marker; runtime activity and mission ownership never infer one.
On a narrow sidebar, the name shortens before the anchor, preserving the right columns.

## From the CLI

```sh
fractal conversations send agent/garden/worker --from person/ada \
  --subject 'README review' --body 'Explain the changes and point me to the result.'
fractal conversations ls agent/garden/worker
```

Replies appear in the agent's conversation, not in a person mailbox.

Use your configured person identity. A running agent uses its bound identity rather than impersonating a person. `fractal conversations --help` lists reading, following, replying, and attachment options.

CLI sends reject an agent absent from this daemon's replicated agent view with
`unknown-recipient`, before recording a message. Remote and stopped agents remain valid
recipients when their declaration or runtime evidence is known. Check `fractal agents ls`
and fleet replication if a recipient is missing. If you know a seat exists on another host
but its evidence has not synced yet, explicitly pass `--allow-undeclared-recipient` on that
send; the override is never remembered. Replies resolve their recipient from the original
message and do not require a fresh agent lookup. People still do not have inboxes.
The check runs before attachment uploads, so a refusal does not store attachment bytes or
consume upload quota. An older daemon without the preflight endpoint still accepts sends;
the CLI prints one notice that recipient validation requires a daemon upgrade.

Successful delivery is not proof that the agent read or completed the request. Check its reply and, for tracked work, the run's state and evidence.

## Ask for a human answer

Known decisions belong in a mission's human gates. When a new decision arises, the working agent uses `fractal work ask` with a structured request and the claimed step. The answer is stored with the work so it can resume with a clear decision.

The request should state the question, why a person is needed, relevant evidence, and consequences of each answer. Reuse the request's idempotency key when retrying the same question. Do not turn a message into an implicit approval.

A person reads and answers the resulting card in Now, or uses the exact actions displayed by `fractal alerts ls`. See the [mission runtime contract](https://github.com/compoundingtech/fractal/blob/main/docs/st3/mission-graph-runtime.md) for structured request details.

## On the phone

A paired [iOS client](ios-app.md) can show conversations and actionable requests. Confirm it is connected before sending an action. Use the paired-only gateway; do not expose the privileged daemon socket.
