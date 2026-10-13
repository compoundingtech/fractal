// Port of ledger-changes.kdl, with the path to main that the KDL leaves in prose written as code:
// CI (fix on failure, three rounds), an intake GO (fix what intake asks and go round again), the
// curator's queue, and the merge (an ejected pull request is fixed and queued again).

import { Data, Effect } from "effect"
import { gate, mission, scope, step, waitFor, type StepResult } from "../sdk/mission.ts"
import { stopAgent } from "./extras.ts"

const intake = "agent/orchard/intake"
const curator = "agent/orchard/curator"
const repo = "orchard/app"

export class CiNeverGreen extends Data.TaggedError("CiNeverGreen")<{ readonly branch: string; readonly reason: string }> {}
export class NeverLanded extends Data.TaggedError("NeverLanded")<{ readonly slice: string; readonly passes: number }> {}

// The run-owned seat is still declared in KDL (agent "builder"); only its subject is needed here.
export interface LedgerInput {
  readonly run: string
}

const pullRequestOf = (opened: StepResult): string =>
  opened.evidence.find((e) => e.startsWith(`${repo}#`)) ?? `${repo}#unknown`

// Check first, fix only what failed, at most three rounds.
const greenCi = (builder: string, branch: string) =>
  Effect.gen(function* () {
    for (let round = 1; ; round++) {
      const ci = yield* gate("ci", { ciPassed: "orchard-ci", repo, branch }).pipe(Effect.result, scope(`ci-${round}`))
      if (ci._tag === "Success") return round
      if (round === 3) return yield* Effect.fail(new CiNeverGreen({ branch, reason: ci.failure.reason }))
      yield* step("fix-ci", {
        assignedTo: builder,
        goal: `CI failed on ${branch}: ${ci.failure.reason}. Fix the cause and push.`
      }).pipe(scope(`ci-${round}`))
    }
  })

// One slice from a pushed branch to main. Every way back to the start is explicit.
const land = (builder: string, slice: string) =>
  Effect.gen(function* () {
    const branch = `ledger/${slice}`
    const opened = yield* step("open-pull-request", {
      assignedTo: builder,
      goal: `Open a small pull request for ${branch} with its tests.`
    })
    const pr = pullRequestOf(opened)
    for (let pass = 1; pass <= 3; pass++) {
      const landed = yield* Effect.gen(function* () {
        yield* greenCi(builder, branch)
        const verdict = yield* step("intake", {
          assignedTo: intake,
          goal: `Review ${pr} and answer GO or NO-GO with reasons.`
        })
        // A step returns a summary string; the SDK has no typed output yet, so GO is parsed.
        if (!verdict.summary.startsWith("GO")) {
          yield* step("address-intake", { assignedTo: builder, goal: `Intake said: ${verdict.summary}. Address it and push.` })
          return false
        }
        yield* step("queue", {
          assignedTo: curator,
          goal: `Queue ${pr} with intake's GO (${verdict.evidence.join(", ")}). Do not arm auto-merge.`
        })
        const merged = yield* waitFor("merged", { merged: pr }).pipe(Effect.result)
        if (merged._tag === "Success") return true
        yield* step("fix-ejection", { assignedTo: builder, goal: `The queue ejected ${pr}: ${merged.failure.reason}. Fix it.` })
        return false
      }).pipe(scope(`pass-${pass}`))
      if (landed) return pr
    }
    return yield* Effect.fail(new NeverLanded({ slice, passes: 3 }))
  }).pipe(scope(`land-${slice}`))

export default mission("orchard/ledger-changes", (input: LedgerInput) => {
  const builder = `agent/${input.run}/ledger-builder`
  return Effect.gen(function* () {
    yield* step("design", {
      title: "Change model verified on the real host",
      timeout: "8h",
      assignedTo: builder,
      goal: "The change model is stored as document orchard/ledger-changes/design, verified on a scratch repository."
    })
    // The design's intake GO is a document review, not a pull request: ask until GO, at most three times.
    for (let round = 1; round <= 3; round++) {
      const verdict = yield* step("design-intake", {
        assignedTo: intake,
        goal: "Review document orchard/ledger-changes/design and answer GO or NO-GO."
      }).pipe(scope(`design-${round}`))
      if (verdict.summary.startsWith("GO")) break
      if (round === 3) return yield* Effect.fail(new NeverLanded({ slice: "design", passes: 3 }))
      yield* step("revise-design", { assignedTo: builder, goal: `Intake said: ${verdict.summary}.` }).pipe(scope(`design-${round}`))
    }

    const shipped: Array<string> = []
    for (const slice of ["reads", "mirror"] as const) {
      yield* step(slice, {
        title: slice === "reads" ? "list and show from the ledger" : "Pull requests as a mirror of changes",
        timeout: "10h",
        assignedTo: builder,
        goal: slice === "reads"
          ? "Implement list, show and claim from the ledger, with tests for concurrent claims."
          : "Implement mirroring a change to a pull request; a pull request's state never sets a change's state."
      })
      shipped.push(yield* land(builder, slice))
    }
    return { shipped }
  }).pipe(Effect.ensuring(stopAgent("stop-builder", builder).pipe(Effect.ignore)))
})
