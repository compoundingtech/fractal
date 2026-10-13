# Experiment 6: real missions ported to the Effect SDK

Three ports of real fleet missions. The names, commands and goal text are invented because this
repository is public. Each KDL file keeps its source's exact structure: steps, `depends-on`,
gates, retries, loops, queues, nested missions, `finally`, messages and attention. All three
pass `st apply --dry-run`. The only refusals are the seat references, because the invented seats
are not declared anywhere.

| Port | Source shape | KDL lines | TS lines | Tests |
|---|---|---|---|---|
| [change](change.kdl) / [.ts](change.ts) | simple: implement → verify → open PR → a person approves → merge | 40 | 40 | 2 |
| [refresh-then-answer](refresh-then-answer.kdl) / [.ts](refresh-then-answer.ts) | messy: a queue of two nested missions, exec gates, two "until green" fix loops with attention on exhaustion, messages, `finally` | 142 | 130 + 41 shared ([extras.ts](extras.ts)) | 3 |
| [ledger-changes](ledger-changes.kdl) / [.ts](ledger-changes.ts) | the path a change takes to main: build, CI, fix on failure, intake GO, the curator's queue, merge, ejection | 46, **with the path in prose** | 90 | 3 |

The line counts exclude blank lines and comments. All 8 tests pass:
`node --test exp6-port/*.test.ts`. They drive each port through `sdk/sim.ts` with scripted
outcomes: the happy path, red CI, a fix with feedback, an exhausted loop, a NO-GO, an ejection
from the queue, and an approval timeout.

## 1. The simple mission: a wash

```kdl
step "verify" timeout="3h" {
  assigned-to "agent/orchard/standing/steward"
  depends-on { step "implement" completed }
  goal "Run the focused tests, ..."
}
step "approve-merge" timeout="24h" {
  agentless
  depends-on { step "open-pull-request" completed }
  gate "Wren approves the exact pull request" type="human" {
    reviewer "person/wren"
    ...
```

```ts
yield* step("verify", { timeout: "3h", assignedTo: steward, goal: "Run the focused tests, ..." })

yield* gate("approve-merge", {
  human: { reviewer: "person/wren", question: "...", review: [pullRequest] }
}).pipe(Effect.timeout("24 hours"))
```

The two are the same length. Order is the source order, so the program has no `depends-on`. The
KDL is no harder to read, and st can check it statically: it found the missing seat at
`apply --dry-run`, before anything ran. The program finds it only when the driver schedules the
step. For a straight line, KDL loses nothing, and the program adds TypeScript, a bundle and a
driver.

Two features have no clean SDK form:

- `produces { resource ... }` becomes a field gate on the step. The product is no longer declared,
  so nothing downstream can find it by its declaration.
- `${ST_MISSION_RUN}` is not available, so the run ID arrives as an input. The SDK needs a run
  context (run, workspace, attempt).

## 2. The messy mission: shorter, and the duplication goes

The KDL repeats one 60-line nested mission twice with different numbers. The program writes it
once as `pullRequestJob(job)` and calls it twice:

```kdl
loop "green" timeout="8h" {
  depends-on { step "merge-main" completed }
  max-rounds 3
  until { gate "every check passes on 210" { exec "gh pr checks 210 ..." ... } }
  round {
    completion { when "all-steps-exhausted" }
    step "fix" timeout="4h" {
      assigned-to "agent/orchard/standing/gardener"
      goal "Read ${loop.feedback} when it is not empty: it holds the checks that failed."
      ...
  }
  on-exhausted { fail; attention "210 is not green ..." { reviewer "person/wren"; severity "error" } }
}
```

```ts
yield* loopUntil("green", {
  maxRounds: 3,
  round: (_round, feedback) => step(job.fix.id, { ..., context: { feedback } }),
  until: () => gate("checks", checks(job.pr, job.loop.checksLimit))
}).pipe(
  Effect.timeout(job.loop.timeout),
  Effect.tapError(() => attention("green-exhausted", { title: job.loop.exhausted, ... }))
)
```

What got clearer:
- The feedback is the gate's failure reason, passed as a value. In KDL it is a document reference
  the worker must read.
- The queue is two `yield*` lines in order. A failure in the first job stops the second, and a
  test shows it.
- `finally` is `Effect.ensuring`. The tests show it running after both success and an exhausted
  loop.

What got worse:
- The SDK has no `loop` with `max-rounds`, `until`, `on-exhausted`, `metric`, `keep-if` or stop
  rules. [extras.ts](extras.ts) rebuilds the plain form in 20 lines. Each program would write its
  own, and st would see neither rounds nor a loop, only a series of ops named `green/1/fix`,
  `green/2/fix` and so on. Today the UIs show a KDL loop as a loop with its round results, and
  this port loses that.
- `message`, `attention` and `stop` have no SDK op. The port invents them as agentless steps with
  extra spec fields, and the driver would need to know each one.
- A nested mission's constraints are shown once, on the mission. The program has to spread them
  into every step, so the reader of a step sees them repeated, and no mission-level goal exists
  to show.
- The sharing that makes the program shorter is invisible until it runs: `pullRequestJob` is a
  function, so a static reader sees one call with unknown op IDs (experiment 3).

## 3. The path to main: the program wins, because KDL never held it

The real mission's steps are three slices of work. The path every change takes to main is a
constraint in English:

```kdl
constraint "... Small pull requests with tests, the existing suites green, an intake GO from
agent/orchard/intake, then send the pull request and the GO to agent/orchard/curator, which
queues it; never arm auto-merge yourself. When CI fails, fix it and go back through intake.
When the queue ejects a pull request, fix it and send it to the curator again."
```

In the program, the same path is code, every way back is explicit, and a test checks each one:

```ts
for (let pass = 1; pass <= 3; pass++) {
  const landed = yield* Effect.gen(function* () {
    yield* greenCi(builder, branch)                       // check, fix-ci, check ... at most 3
    const verdict = yield* step("intake", { assignedTo: intake, ... })
    if (!verdict.summary.startsWith("GO")) { yield* step("address-intake", ...); return false }
    yield* step("queue", { assignedTo: curator, ... })
    const merged = yield* waitFor("merged", { merged: pr }).pipe(Effect.result)
    if (merged._tag === "Success") return true
    yield* step("fix-ejection", ...); return false
  }).pipe(scope(`pass-${pass}`))
  if (landed) return pr
}
```

The test "red CI is fixed, a NO-GO goes round again, an ejection is fixed and requeued" runs
15 ops across three passes. In KDL this needs either prose, as today, or a hand-unrolled graph:
three passes × (CI loop + intake + queue + merge wait), with no way to branch on a GO. Gates can
only pass or wait, and `depends-on ... failed` cannot route a NO-GO back to an earlier step.

Some of this is still weak:

- The pull request number comes from a worker's evidence at run time, and GO is parsed from a
  summary string. Steps need typed output, a schema the worker fills, before branching on it is
  safe.
- Every hop is a separate op, and an intake or curator seat has to treat each as ordinary work.
  Today the intake GO and the curator queue are messages between seats. The port turns them into
  steps assigned to those seats, which changes how those seats work.

## What KDL has that the program form loses

| KDL | In the program |
|---|---|
| Static checks at publish: references, cycles, seats, gates (`missions check`) | Only when an op is scheduled. A broken gate or a missing seat shows up mid-run. |
| `produces`, `baseline`, mission `gate`s, `completion` frontiers | No form. A field gate on a step is the nearest. |
| `loop` with rounds, metrics, `keep-if`, stop rules, round results in the UI | Hand-written loops that st sees as flat ops. |
| Run-owned `agent` declarations (`agent "builder" { ... }`) | Must stay in a KDL host mission. The program can only name the subject. |
| `concurrent-runs`, `revisions="human-only"`, `revision-cutover`, report-to | Properties of the host mission, not of the program. |
| `finally` that runs on **cancellation** | `Effect.ensuring` runs on success and failure. The history has no "run cancelled" event yet, so a cancelled program never schedules its finalizer ops. |
| A diff of a revision is a graph diff | A diff of a revision is a code diff, and compatibility is decided by replay (experiment 4). |
| `${loop.feedback}` as an immutable document | Feedback is a plain value. It is easier to use, but nobody else can audit it. |

## What the program has that KDL cannot express

- Branching on a result: GO or NO-GO, merged or ejected.
- A loop back to an earlier stage: fix, then CI again, then intake again.
- Abstraction: one job shape reused.
- Check-first loops: fix only when red. A KDL loop always runs its round before `until`.
- A deadline over a block of steps (`Effect.timeout` on the loop or the job) with typed recovery.

## Verdict

- **Simple missions:** no gain. KDL is as short, statically checked, and shown better. Keep KDL.
- **Repeated structure (refresh-then-answer):** the program is a little shorter and easier to
  change, but it hides the loop from st and its UIs. Without a first-class `loop` in the SDK,
  with rounds recorded in the graph, the port is a step backwards in visibility.
- **The change path (ledger-changes):** this is the only place the program clearly wins, and it
  wins because the fleet's real control flow is in prose today. Even here it needs typed step
  output and a native place for asks and handoffs between seats before it is safe.
- **Overall:** a program does not replace KDL for whole missions. It earns its place for the one
  part KDL cannot hold: a change's loop through CI, intake and the queue. That fits a single step
  or a nested mission driven by a program inside an otherwise declarative KDL mission. Seats,
  products, revisions, concurrency and cleanup stay declared.

## What the SDK should add (from these ports)

1. A run context (run, workspace, attempt, requester), instead of passing the run as an input.
2. Typed step output, with a schema the worker fills, for branching on GO or NO-GO and for
   carrying a PR number.
3. `message`, `attention` and `stop` ops, written here in [extras.ts](extras.ts) as stand-ins.
4. A first-class `loop` that records rounds the way KDL loops do, so the UIs keep showing them.
5. A cancellation event in the history, so `Effect.ensuring` matches `finally`.
6. Mission-level constraints that apply to every step in a block, without repeating them.
