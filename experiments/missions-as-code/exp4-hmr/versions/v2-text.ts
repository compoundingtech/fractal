// Experiment 4 version: the fix step asks for a smaller fix (definition change only).
// Generated from exp1-sdk/ship-change.ts by exp4-hmr/make-versions.py; edit that script, not this file.

// Experiment 1: one mission that uses every control-flow feature the SDK claims.
// All names are invented. The seats are declared in KDL today (see ../exp6-port/seats.kdl).

import { Effect, Schedule } from "effect"
import { ask, escalateAfter, gate, mission, scope, step, timerName, type StepFailed, type StepResult } from "../../sdk/mission.ts"

interface Stopped {
  readonly _tag: "Stopped"
  readonly reason: string
}

const builder = "agent/orchard/builder"
const reviewer = "agent/orchard/reviewer"
const fixer = "agent/orchard/fixer"

export default mission("orchard/ship-change", (input: { change: string; repo: string }) =>
  Effect.gen(function* () {
    // Parallel steps: both open at once; the first failure cancels the other.
    const [build, docs] = yield* Effect.all(
      [
        // Retry: a transient build failure gets two more attempts, 1m then 2m apart.
        // Each attempt is its own op (build, build.2, build.3) and the waits are durable timers
        // named build-backoff, build-backoff.2.
        step("build", { title: "Build the change", goal: `Build ${input.change}.`, assignedTo: builder }).pipe(
          Effect.retry({ times: 2, schedule: Schedule.exponential("1 minute") }),
          timerName("build-backoff")
        ),
        step("docs", { title: "Update the docs", goal: `Document ${input.change}.`, assignedTo: builder })
      ],
      { concurrency: "unbounded" }
    )

    // Timeout with escalation: after 4h without a review, ask a person what to do.
    const review = yield* step("review", {
      title: "Review the change",
      goal: "Review the diff and the build evidence.",
      assignedTo: reviewer,
      context: { build: build.evidence as any, docs: docs.summary }
    }).pipe(
      escalateAfter("review-deadline", "4 hours", () =>
        ask("review-overdue", {
          for: "person/wren",
          title: "The review is overdue",
          question: "The reviewer has not answered in four hours. Reassign or stop?",
          answers: [
            { id: "reassign", label: "Reassign to the fixer" },
            { id: "stop", label: "Stop the change" }
          ]
        }).pipe(
          Effect.flatMap((answer): Effect.Effect<StepResult, StepFailed | Stopped> =>
            answer.id === "reassign"
              ? step("review-reassigned", { title: "Review the change", goal: "Review it.", assignedTo: fixer })
              : Effect.fail({ _tag: "Stopped", reason: "a person stopped the change" })
          )
        )
      )
    )

    // A loop: up to three rounds of CI, fixing what fails. Each round's ops are named round-N/...
    for (let round = 1; round <= 3; round++) {
      const ci = yield* gate("ci", { ciPassed: "orchard-ci", repo: input.repo, branch: input.change }).pipe(
        Effect.result,
        scope(`round-${round}`)
      )
      if (ci._tag === "Success") return { shipped: true, rounds: round, review: review.summary }
      yield* step("fix", { title: `Fix CI, round ${round}`, goal: "Fix the failing checks with the smallest change that passes.", assignedTo: fixer }).pipe(
        scope(`round-${round}`)
      )
    }
    return yield* Effect.fail({ _tag: "CiNeverPassed" as const, rounds: 3 })
  }).pipe(
    // On-failure handling: any failure first files a report, then the mission still fails.
    Effect.tapError((error) =>
      step("report-failure", {
        title: "Report why the change stopped",
        goal: "Write up the failure for the requester.",
        assignedTo: builder,
        context: error as any
      }).pipe(Effect.ignore)
    )
  )
)
