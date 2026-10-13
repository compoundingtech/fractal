// Port of change.kdl. A straight line: five yields, one per KDL step.

import { Effect } from "effect"
import { gate, mission, step } from "../sdk/mission.ts"

const steward = "agent/orchard/standing/steward"

// The SDK gives a program no run context, so the run ID arrives as input. KDL has
// ${ST_MISSION_RUN}; the SDK should expose the same values (run, workspace, attempt).
export interface ChangeInput {
  readonly request: string
  readonly run: string
}

export default mission("orchard/change", (input: ChangeInput) =>
  Effect.gen(function* () {
    const pullRequest = `resource/mission-run/${input.run}/pull-request`

    yield* step("implement", {
      timeout: "6h",
      assignedTo: steward,
      goal: `Implement this request in the steward's worktree: ${input.request}. Add focused tests.`
    })

    yield* step("verify", {
      timeout: "3h",
      assignedTo: steward,
      goal: "Run the focused tests, the full unit suite and the model-free evals. Record exact evidence."
    })

    // KDL `produces { resource ... }` has no SDK form. The nearest is a gate on the step that
    // checks the resource, which st already runs; the product itself is not declared.
    yield* step("open-pull-request", {
      timeout: "1h",
      assignedTo: steward,
      goal: "Push the branch and open a short pull request. Publish it as the declared pull-request resource.",
      gates: [{ name: "the pull request is published", field: ["state", pullRequest, "is", "ready"] }]
    })

    // An agentless step whose only content is a human gate is just a gate. The step's 24h budget
    // becomes a durable timeout on the program side; KDL fails the agentless step the same way.
    yield* gate("approve-merge", {
      human: {
        reviewer: "person/wren",
        question: "Is this orchard pull request ready to merge?",
        review: [pullRequest]
      }
    }).pipe(Effect.timeout("24 hours"))

    return yield* step("merge", {
      timeout: "2h",
      assignedTo: steward,
      goal: "Read every review and CI result. Fix defects, get another approval if the pull request changes, and merge the approved change."
    })
  })
)
