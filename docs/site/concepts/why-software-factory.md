---
title: "Why a software factory"
description: "Coding agents got good. Driving them by hand doesn’t scale. What an ideal factory needs, and why it needs structure."
order: 1
---

Coding agents can do real engineering work now. Running a few of them is easy. Running dozens, for weeks, toward goals you care about, is a different problem. This page explains that problem, what a system that solves it has to provide, and why it needs structure. The rest of this section shows how Fractal answers it.

## You were the factory

A **software factory** is a system that produces software to meet a set of goals. It runs a loop: decide what should exist, do the work, look at the result, and let that feedback shape the next intent.

For most of your career, you have been that factory. You held the intent, wrote the code, ran the tests, read the reviews and decided what came next. Every round of the loop went through you.

```figure human-factory
The human software factory. You run every station of the loop:

1. Intent (you): decide what should exist.
2. Work (you): write and change the code.
3. Feedback (you): test, review, ship, listen.

Feedback returns to intent, and you close the loop every round.
```

## Agents do the work now

Coding agents changed the middle of that loop first. An agent writes the code while you watch. Then agents started reviewing, testing and reporting back, so feedback moved to agents too.

What stays with you is intent: the goals, the boundaries and the decisions only a person can make.

```figure agentic-factory
The agentic software factory. Agents run work and feedback; you own the intent:

1. Intent (you): set goals and boundaries.
2. Work (agents): build, in parallel.
3. Feedback (agents): test, review, report.

Feedback returns to intent, and the loop runs on its own.
```

## What breaks at scale

Most setups stop halfway. The agents do the work, but you still start each one, steer it, read its chat and carry its context in your head. Thirty-nine open terminals isn’t a factory. It’s you, doing the factory’s job.

```figure what-breaks
What breaks when you drive agents by hand:

- Babysitting agents: starting, steering and context switching between agents
  is draining. Your attention is the bottleneck.
- Noisy questions: constant inconsequential questions, or work built on wrong
  assumptions.
- Stops when you stop: close the laptop and the work stops with it.
- Waits for prompts: agents only act when you prompt them.
- Sessions die: sessions die and lose their progress.
- Invisible work: you can’t see what your agents are actually doing.
```

More agents make each of these worse. The fix isn’t a better terminal. It’s a system around the agents.

## What an ideal factory needs

Each problem points at a requirement. Together they fall into three groups: the factory has to run on its own, it can’t lock you in, and it needs a runtime with real structure.

```figure ideal-factory
Requirements for an ideal software factory, each answering a problem.

Autonomy
- Babysitting agents → Goal-driven: you set intent and goals; you don’t steer
  every agent.
- Noisy questions → Signal over noise: only what needs you reaches you.
- Stops when you stop → Long-horizon: work continues for days and weeks
  without you.
- Waits for prompts → Proactive: agents act on events and goals, not only on
  prompts.

Flexibility & no lock-in
- Tied to your desk → Run & use anywhere: on your own computers and servers,
  from any device.
- Vendor lock-in → Any harness, model, client: no single coding harness, model
  provider or client.
- Work lives elsewhere → Deep integrations: GitHub, issue trackers, chat and
  the rest of your world.
- One size fits none → Extensible via plugins: open, so it fits how you work.

Structured runtime
- Imperative mess → Declarative: the factory as code, not clicks.
- Sessions die → Durable: work survives crashes and restarts.
- Bottlenecks run out → Budgets: spend, compute and any other limited
  resource.
- Invisible work → Observable: you can see what every agent is doing and why.
```

These are the bar, not a feature list.

## Why structure

The last group is the least obvious and the most important. Agent work today is organized around the pieces the harnesses give you: agents, sessions, transcripts and turns. None of them compose. A session can’t contain a loop, a turn can’t depend on another agent’s turn, and a transcript can’t say when the work is done.

UI development had the same problem. Before React, an app was loose DOM nodes and imperative updates. The component gave UI a boundary that takes inputs, nests and composes. Agent work needs the same kind of unit.

```figure why-structure
UI before React: loose DOM nodes and imperative calls (el.appendChild,
node.textContent). React gave UI a composable boundary: the component (App
contains List contains Item; props in, nests, composes).

Agents today: agent, session, transcript and turn, none of which compose. The
open questions:

- What is a useful unit of composition?
- How do you automate things?
- How do you express a long-running plan?
```

### A PLAN.md is not the plan

The usual answer to long-horizon work is a markdown plan the agents keep editing. It works for an afternoon. After a few weeks and a dozen sessions it is thousands of lines that nobody reads, its structure is implied by headings, and “done” is a checklist that nothing enforces.

The same work declared as a mission is small enough to read. Its structure is explicit: steps, the agent assigned to each, and what each depends on. And it says what done means: gates decide when a step passes, and a human gate puts the final decision with a person.

```figure plan-vs-mission
The same work, “Comments on every platform”, two ways.

As a PLAN.md (4,812 lines):
1. Can’t be read: rewritten by every session.
2. Structure isn’t explicit: phases, owners and dependencies live in prose.
3. Goals aren’t explicit: “Done when” is a checklist nothing enforces.

As a mission:
1. Readable: the whole plan is a graph that fits on a screen. Goal: discuss
   any document on web, iOS, Android, desktop and the CLI.
2. Explicit structure: a Comments API step, then Web, iOS, Android, Desktop
   and CLI steps in parallel, each assigned to an agent.
3. Explicit done: every client step has gates, a parity step runs 12 checks
   on 5 platforms, and you approve the release at a human gate.
```

## What it feels like

A factory with that structure changes your day more than your tooling.

### Running dozens of agents stops feeling crazy

Our own factory has 83 declared agents on 4 machines. Almost none of that reaches us. Agents work, test and review each other; the few decisions that need a person arrive as [alerts](attention.md), a short list instead of a stream of chats.

```figure attention
83 declared agents on 4 machines in our own factory. Nearly all of them are
working, testing or reviewing, with nothing for you to read. 3 alerts need a
person (illustrative):

- Review: Is the comments release ready?
- Request: Should guests see resolved threads?
- Launch: Draft mission: offline mode.
```

### Your role changes

You stop being the bottleneck and start being the owner. You still step in, just far less often.

```figure human-role
With a software factory, you no longer have to … / instead, you …

- Start & steer every agent → Provide intent
- Read agent chats all day → Be in the flow more often
- Be the bottleneck for work getting done → Set goals
- Read and review all code → Observe feedback
- Get paged in the middle of the night → Set the bar the factory must meet
```

### Stepping away

Pairing with an agent is still the fastest way to shape something. But it’s optional. Tell the agent you’re stepping away; the declared mission keeps the rest of the team working.

```figure afk
A conversation with the Web builder agent in the Fractal web app:

- You: Let’s pair on the mention picker for comments.
- Web builder: The picker filters people as you type and inserts @name.
- You: Arrows and Enter too, then wire it to the comments API.
- Web builder: Done. The composer posts to the API; 14 tests pass.
- You: I’m stepping away for the rest of the day.
- Web builder: Got it. I’ll finish web comments with the team and hand the
  build to the parity checker. Anything that needs you waits in your alerts.

Before you leave, the Web builder works in short rounds with you. After, the
Web, iOS and Android builders and the parity checker keep working on their
own.
```

## Next: what Fractal provides

Fractal is our work toward that factory: an open framework for building your own, on your machines, with the harnesses and models you choose. It is a research preview and doesn’t meet every requirement above yet. The rest of [Understand Fractal](index.md) covers what exists today: the [software factory](software-factory.md) itself, [agents](agents.md), [missions and runs](missions.md), [steps and work](steps.md), [products and gates](gates.md), [the graph](graph.md), [attention](attention.md) and [launch](launch.md).
