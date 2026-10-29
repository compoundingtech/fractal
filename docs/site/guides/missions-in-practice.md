---
title: "Missions in practice"
description: "Add dependencies, human review, queued runs, and explicit revisions."
order: 2
---

# Missions in practice

KDL is the runnable format. Effect and TS are preview APIs that illustrate the design target, not an installed SDK. Use KDL for the runnable commands in these guides.

Use the worker from [Your first agent](../getting-started/first-agent.md). The examples use `person/ada`; substitute your configured person.

## Plan the result and the review

Save this as `missions/readme.kdl` in the worker workspace:

```ts group=mission variant=effect
import { Effect } from 'effect'
import { Fractal } from 'fractal/effect'
import { worker } from '../worker'

const ada = Fractal.person({ id: 'ada' })

export const mission = Fractal.mission(
  {
    id: 'garden/readme',
    state: 'ready',
    timeout: '1h',
    reportTo: ada,
    goal: 'README.md introduces this workspace and has a human approval.',
    constraints: [
      'Only edit README.md. Do not commit or publish changes.',
    ],
  },
  () => Effect.gen(function* () {
    yield* Fractal.step({
      id: 'prepare',
      timeout: '30m',
      goal: 'README.md contains a clear one-paragraph workspace introduction.',
    }).pipe(Effect.provide(Fractal.Worker.layer(worker)))

    yield* Fractal.step(
      { id: 'approve' },
      Effect.gen(function* () {
        const answer = yield* Fractal.ask({
          name: 'Ada approves the README',
          reviewer: ada,
          question: 'Is the README ready to use?',
        })
        if (answer._tag === 'Rejected') {
          yield* Effect.fail(new Error('README rejected'))
        }
      }),
    )
  }),
)
```

```ts group=mission variant=ts
import { Fractal } from 'fractal'
import { worker } from '../worker'

const ada = Fractal.person({ id: 'ada' })

export const mission = Fractal.mission(
  {
    id: 'garden/readme',
    state: 'ready',
    timeout: '1h',
    reportTo: ada,
    goal: 'README.md introduces this workspace and has a human approval.',
    constraints: [
      'Only edit README.md. Do not commit or publish changes.',
    ],
  },
  async () => {
    await Fractal.Worker.provide(worker, async () => {
      await Fractal.step({
        id: 'prepare',
        timeout: '30m',
        goal: 'README.md contains a clear one-paragraph workspace introduction.',
      })
    })

    await Fractal.step(
      { id: 'approve' },
      async () => {
        const answer = await Fractal.ask({
          name: 'Ada approves the README',
          reviewer: ada,
          question: 'Is the README ready to use?',
        })
        if (answer._tag === 'Rejected') {
          throw new Error('README rejected')
        }
      },
    )
  },
)
```

```kdl group=mission variant=kdl
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

A rejected gate fails its condition. Use a feedback gate when changes should route back to the worker rather than end as rejection; see the [worked examples](https://github.com/compoundingtech/fractal/blob/main/examples/fractal/README.md).

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

## Repair current work after a person identity change

After changing a person identity, repair its current inbox once. Upgrade every fleet member
to a build that understands person repair first, and send the repair through one member.
The command refuses until every configured or active fleet member has advertised the matching
registry digest; an unavailable or mismatched peer also blocks retries.
During rollout, only the operator designated by release approval or that person's explicitly
named delegate may run this one-shot repair. This is an operational restriction, not an
additional authorization rule in the product.
OLD and NEW must be distinct complete `person/NAME` subjects. NEW must already be known
from stored identity or routing evidence; naming it in this request does not create it.
Only OLD or NEW may act: agents and unrelated people are refused.

```sh
fractal persons rename person/ada person/robin --as person/robin \
  --idempotency-key ada-person-repair
```

This atomically reassigns eligible open person steps and pending human reviews on that
member without changing their episodes, original requests, saved specifications, run
requesters, or historical actors. It creates no alias and does not rewrite published
missions. Work created later still follows its declarations.

Use `--json` to keep the complete report:

| Report field | Meaning and next action |
| --- | --- |
| `reassigned_steps`, `reassigned_reviews` | Current steps and review owners moved to NEW. Read NEW's inbox to continue them. |
| `unchanged_attention_requests`, `unchanged_fault_cards` | Current attention requests and fault cards still routed to OLD; these were not moved. Inspect each ID and resolve it through its normal workflow. |
| `unchanged_revision_approvals`, `unchanged_planning_launch_approvals` | Revision-proposal and planning-launch approvals still routed to OLD; these were not moved either. Inspect and handle the existing approval rather than assuming it reached NEW. |
| `live_run_references` | Live run IDs mapped to JSON-pointer paths that still name OLD in their routing or retained specifications. Runtime step assignees use `/step_runs/<escaped-step-subject>/assignee`; saved-spec routes and `/requester` are listed separately. Inspect the run and use an authorized run revision where appropriate; republishing alone does not change a live run. |
| `published_mission_references` | Published mission IDs mapped to JSON-pointer paths still naming OLD, including step assignees and reviewer routes. Update the source declarations and republish before starting future work. |
| `run_requesters`, `missions_to_republish` | Summary IDs for unchanged live-run requesters and published missions with OLD references. Requesters remain historical attribution, not renamed actors. |
| `namespace_notice` | Client-derived custom/person namespaces, including private glasses and terminals, are not automatically moved. No namespace migration or alias is created. |

The report is a snapshot, not a promise that OLD will receive no future work. Review every
unchanged category and routing path. Saved specifications and historical attribution remain
unchanged even when current runtime routing has moved.

After a timeout, retry the **same key and identical request**, including the actor, through
the same upgraded member. The winning durable receipt returns its original report, including
after reopen or replication; it does not sweep work created since that receipt. Reusing a key
for another request returns `idempotency-conflict`. A completed repair fences OLD against a
different destination with `person-rename-conflict` before any routing writes, so competing
requests on that member cannot split the repair.

Independent disconnected members can each initially accept a different destination.
After their receipts replicate, the earliest completed receipt in canonical claim order
(including envelope ordering) selects one destination consistently for both work and review
routing. Losing routes are removed by rebuilding the affected run projections; retrying a
losing receipt's exact request returns `person-rename-conflict` instead of its old report.
The winner's exact retry remains write-free. This is eventual convergence, not global
admission exclusion before replication. Use one member for repair rather than deliberately
issuing competing requests.

Do not try to undo a completed repair by swapping an older binary back in or sending OLD to
another destination. Keep the receipt and claim history, finish updating declarations and
handling the reported unchanged items, and roll forward on compatible builds. Restoring an
older backup loses subsequent claims and requires coordinated fleet recovery, not just a
local binary rollback.
