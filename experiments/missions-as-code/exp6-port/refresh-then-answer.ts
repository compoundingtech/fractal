// Port of refresh-then-answer.kdl. The two nested missions share one shape, so the program
// writes it once as a function and calls it twice, in order, which is all the KDL queue does.

import { Duration, Effect } from "effect"
import { mission, scope, step, gate, type GateSpec, type StepSpec } from "../sdk/mission.ts"
import { attention, loopUntil, send } from "./extras.ts"

const gardener = "agent/orchard/standing/gardener"
const chief = "agent/orchard/standing/chief"
const checks = (pr: number, timeLimit: string): GateSpec => ({
  exec: `gh pr checks ${pr} --repo orchard/app --watch --fail-fast --interval 60`,
  host: "grove",
  timeLimit
})

interface Job {
  readonly id: string
  readonly pr: number
  // Nested-mission constraints apply to every step of the job; KDL states them once.
  readonly constraints: ReadonlyArray<string>
  readonly first: { readonly id: string } & StepSpec
  readonly fix: { readonly id: string; readonly timeout: string; readonly goal: ReadonlyArray<string> }
  readonly loop: { readonly timeout: Duration.Input; readonly checksLimit: string; readonly exhausted: string }
  readonly done: { readonly id: string; readonly title: string; readonly content: string }
}

const pullRequestJob = (job: Job) =>
  Effect.gen(function* () {
    const constraint = job.constraints
    yield* step(job.first.id, { ...job.first, constraint, assignedTo: gardener })

    // KDL loop "green": max-rounds 3, a timeout over every round, attention when exhausted.
    yield* loopUntil("green", {
      maxRounds: 3,
      round: (_round, feedback) =>
        step(job.fix.id, {
          timeout: job.fix.timeout,
          assignedTo: gardener,
          constraint,
          goal: job.fix.goal,
          // ${loop.feedback} was a document reference; here it is the last gate failure itself.
          context: { feedback }
        }),
      until: () => gate("checks", checks(job.pr, job.loop.checksLimit))
    }).pipe(
      Effect.timeout(job.loop.timeout),
      Effect.tapError(() =>
        attention("green-exhausted", { title: job.loop.exhausted, reviewer: "person/wren", severity: "error" }).pipe(
          Effect.ignore
        )
      )
    )

    yield* send(job.done.id, { to: chief, title: job.done.title, content: job.done.content })
  }).pipe(
    // KDL finally: runs after success or failure. Cancellation is the gap (see COMPARISON.md).
    Effect.ensuring(
      send("tell-chief-finished", {
        to: chief,
        title: `orchard/${job.id} finished`,
        content: `The ${job.id} run reached its final phase.`
      }).pipe(Effect.ignore)
    ),
    scope(job.id)
  )

export default mission("orchard/refresh-then-answer", (_input: {}) =>
  Effect.gen(function* () {
    // queue "jobs": each item starts only after the one before it completed.
    yield* pullRequestJob({
      id: "refresh-210",
      pr: 210,
      constraints: [
        "210 is Linden's draft and conflicts with main. Merge main in, fix the conflicts, and leave it closer to ready.",
        "Preserve the intent of every conflicted hunk. Resolve conflicts; do not redesign or tidy.",
        "Merge main into the branch. Do not rebase and do not force-push: the branch is shared.",
        "Do not mark it ready, do not merge it, and do not answer any review thread on it.",
        "Comment on 210 when done: what was merged, how each conflict was decided, and what still fails."
      ],
      first: {
        id: "merge-main",
        timeout: "4h",
        goal: [
          "Merge origin/main into the branch in a worktree of its own, resolve every conflict so both sides survive, and push.",
          "Decide each conflicting hunk from what the code is for, not from which side is newer."
        ],
        gates: [
          {
            name: "210 is no longer conflicting",
            exec: `for i in $(seq 30); do M=$(gh pr view 210 --repo orchard/app --json mergeable --jq .mergeable); [ "$M" != UNKNOWN ] && break; sleep 10; done; test "$M" = MERGEABLE`,
            host: "grove",
            timeLimit: "20m"
          }
        ]
      },
      fix: {
        id: "fix",
        timeout: "4h",
        goal: [
          "Make the checks pass: type errors, lint and tests the merge broke.",
          "Fix the code that is wrong, not the test that reports it, unless main deliberately changed what the test asserts."
        ]
      },
      loop: { timeout: "8 hours", checksLimit: "90m", exhausted: "210 is not green after three rounds" },
      done: {
        id: "tell-chief",
        title: "210 has main merged in and its checks pass",
        content: "Linden's branch is up to date with main and green. Nothing was merged and no thread was touched."
      }
    })

    yield* pullRequestJob({
      id: "answer-233",
      pr: 233,
      constraints: [
        "A finding is valid only when it names a real defect or gap in this diff. Never change code to satisfy a finding you judge wrong.",
        "Answer every thread with the fix and its commit, or why it will not change. Then resolve it.",
        "Do not merge and leave every other pull request alone."
      ],
      first: {
        id: "answer",
        timeout: "4h",
        goal: [
          "Read each finding against the diff, decide fix or no fix, make the fixes with tests, and push.",
          "Answer and resolve all three threads."
        ],
        gates: [
          {
            name: "no review-bot thread is left unresolved on 233",
            exec: `test "$(./scripts/unresolved-bot-threads 233)" -eq 0`,
            host: "grove",
            timeLimit: "20m"
          }
        ]
      },
      fix: { id: "make-green", timeout: "2h", goal: ["Fix whatever the checks report and push again."] },
      loop: { timeout: "6 hours", checksLimit: "60m", exhausted: "233 is not green after answering the review bot" },
      done: {
        id: "tell-chief",
        title: "The review bot's findings on 233 are answered",
        content: "Every thread on 233 is answered and resolved and the checks pass. Nothing is merged."
      }
    })
  })
)
