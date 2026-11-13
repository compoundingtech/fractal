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

## Availability

A person tells agents whether to expect an answer soon. Availability informs; it never grants authority or changes who may act.

```sh
fractal availability
fractal availability set --status away --interruptions urgent --deadline-clock hold-all --for 2h
fractal availability clear status
```

- **Status** is `available` or `away`. Without an override it follows your devices: `available` while one of them has had input in the last 10 minutes, `away` when every device reporting within the last 24 hours is idle, locked or asleep. When no device has reported for 24 hours, status is `available` and its source is `unreported`, so agents ask as usual.
- **Interruptions** are `all`, `urgent` or `none`.
- **Deadline clock** is `run`, `hold-routine` or `hold-all`. A hold pauses the deadlines of asks to you: routine asks only, or every ask.

`set` needs at least one of `--status`, `--interruptions` and `--deadline-clock`; `--for DURATION` or `--until RFC3339` applies to each one it names. `clear` with no axis clears all three: status follows presence again, interruptions return to `all` and the deadline clock to `run`. `--json` prints the client API response.

`fractal availability presence active|inactive|locked|sleeping [--last-input-at RFC3339] [--observed-at RFC3339]` reports a device's raw state the way a paired client does: once per transition, never as a heartbeat. `--observed-at` is when the device saw that state and defaults to now; the daemon refuses one older than 15 minutes or more than 2 minutes ahead of its clock. The daemon derives presence (`present`, `idle`, `locked`, or `unreported`) with one idle threshold for every device and prints when to report again if nothing changes.

Only you can change your availability; an agent may only read it. You see your own device presence. An agent sees the people it works for, and `--person person/NAME` shows one person, without their presence.
