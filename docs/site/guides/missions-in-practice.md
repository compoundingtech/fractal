---
title: "Missions in practice"
description: "Add dependencies, human review, queued runs, and explicit revisions."
order: 2
---

# Missions in practice

Use the worker from [Your first agent](../getting-started/first-agent.md). The examples use `person/ada`; substitute your configured person.

## Plan the result and the review

Save this as `missions/readme.kdl` in the worker workspace:

```kdl
version 2
mission "garden/readme" state="ready" {
  goal "README.md introduces this workspace and has a human approval."
  constraint "Only edit README.md. Do not commit or publish changes."
  step "prepare" timeout="30m" {
    assigned-to "agent/garden/worker"
    goal "README.md contains a clear one-paragraph workspace introduction."
  }
  step "approve" {
    agentless
    depends-on { step "prepare" completed }
    gate "Ada approves the README" type="human" {
      reviewer "person/ada"
      question "Is the README ready to use?"
    }
  }
}
```

```sh
fractal apply missions/readme.kdl --as person/ada --dry-run
fractal apply missions/readme.kdl --as person/ada
fractal missions start garden/readme --id garden/readme/one --workspace "$PWD" --as person/ada
fractal missions show mission-run/garden/readme/one
```

The review waits without occupying the worker. Read the file and run result before answering. Copy the exact approval step ID from `fractal alerts ls --as person/ada`, then:

```sh
fractal alerts approve STEP_RUN_ID --as person/ada --reason 'The README is ready.'
```

A rejected gate fails its condition. Use a feedback gate when changes should route back to the worker rather than end as rejection; see the [worked examples](https://github.com/compoundingtech/fractal/blob/main/examples/st3/README.md).

## Revise a live run explicitly

Write the complete replacement mission, including unchanged steps, to `missions/readme-v2.kdl`. Add a `check-links` step depending on `prepare`, and make `approve` depend on `check-links`.

```sh
fractal work revise mission-run/garden/readme/one missions/readme-v2.kdl \
  --as person/ada --reason 'Check every relative link before review.'
fractal work revision generations mission-run/garden/readme/one
```

The run keeps its identity and gets a new generation. Publishing a changed mission by itself does not revise a running instance. Changed work and its dependents may need to run again.

## Queue follow-on work

To start a separately published summary mission only after this run succeeds:

```sh
fractal missions start garden/summary --id garden/summary/one \
  --after mission-run/garden/readme/one --workspace "$PWD" --as person/ada
fractal agents queue agent/garden/worker
```

Publish `garden/summary` first with its own finite goal and assigned step. A failed or cancelled prerequisite does not count as successful completion. One agent holds one step at a time; give concurrent work separate agents and workspaces.

## Keep unattended work actionable

Write known decisions into constraints and referenced documents before starting. Use explicit dependencies instead of relying on file order. For an unexpected decision, use `fractal work ask` rather than waiting for a person to notice a message. Record progress and evidence in the graph, not only in chat.

See [Configuration and KDL](../reference/configuration.md) for publication semantics and [Talking to agents](talking-to-agents.md) for conversation.
