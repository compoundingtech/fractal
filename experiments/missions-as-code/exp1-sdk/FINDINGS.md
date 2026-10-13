# Experiment 1: a small Effect mission SDK

Question: can a mission be an `Effect.gen` program where each yield is a step, gate, ask or wait,
with retries, timeouts with escalation, on-failure handling, loops and parallel steps, while
agents stay declared as they are today?

**Yes.** The SDK is about 300 lines (`../sdk`), and almost all of the control flow is plain
Effect: the SDK adds the four kinds of op and a replay engine, not a second language.
[ship-change.ts](ship-change.ts) uses every feature, and
[ship-change.test.ts](ship-change.test.ts) drives it through each path in simulation (7 tests).

## The shape

```ts
export default mission("orchard/ship-change", (input) =>
  Effect.gen(function* () {
    const [build, docs] = yield* Effect.all([
      step("build", { assignedTo: builder, ... }).pipe(
        Effect.retry({ times: 2, schedule: Schedule.exponential("1 minute") }),
        timerName("build-backoff")),
      step("docs", { assignedTo: builder, ... })
    ], { concurrency: "unbounded" })
    const review = yield* step("review", { assignedTo: reviewer, ... }).pipe(
      escalateAfter("review-deadline", "4 hours", () => ask("review-overdue", { for: "person/wren", ... }) ...))
    for (let round = 1; round <= 3; round++) {
      const ci = yield* gate("ci", { ciPassed: "orchard-ci", ... }).pipe(Effect.result, scope(`round-${round}`))
      if (ci._tag === "Success") return { shipped: true, rounds: round }
      yield* step("fix", { assignedTo: fixer, ... }).pipe(scope(`round-${round}`))
    }
    return yield* Effect.fail({ _tag: "CiNeverPassed" })
  }).pipe(Effect.tapError((e) => step("report-failure", { ... }).pipe(Effect.ignore))))
```

| Feature | How | What it becomes in the history |
|---|---|---|
| step, gate, ask, wait | `step(id, spec)` and friends; the spec is the KDL step's fields | one op with that ID |
| agents | `assignedTo: "agent/..."`, exactly the KDL selector; seats stay declared in KDL | — |
| parallel | `Effect.all([...], { concurrency })` | several ops open together; the first failure cancels the others |
| retries | `Effect.retry` with any `Schedule`; or st's native `retry` on the step | `build`, `build.2`, `build.3`; delays are durable timers |
| timeouts, escalation | `Effect.timeout`, or `escalateAfter(name, after, fallback)` | a named timer racing the op; the loser is cancelled |
| on-failure | `catchTag("StepFailed")`, `tapError`, `onError`, `ensuring` | ordinary ops in the handler |
| loops | plain `for`/`while`, with `scope("round-N")` naming each round's ops | `round-1/ci`, `round-2/fix` |
| durable sleep | `sleep(id, duration)` or `Effect.sleep` | a timer op |
| versions | `patched(id)`, see experiment 4 | a marker op |

## How it works

`decide(program, history)` (`../sdk/engine.ts`) is pure. It runs the program on a synchronous
scheduler, replays the history one event at a time, and returns the ops the program waits on
plus the commands for a driver (schedule these, cancel those). Three Effect services are
replaced:

- **Scheduler:** a FIFO queue drained by hand, so the same history gives the same interleaving.
- **Clock:** "now" is the time of the history event being applied. `Clock.sleep` opens a
  durable timer op, which makes `Effect.sleep`, `Effect.timeout` and every `Schedule` delay
  durable without the author doing anything.
- **Random:** seeded per run, so jittered schedules replay.

## Where it falls short

1. **Results are untyped.** A step returns `{ summary, evidence }`. A program that branches on
   a step's outcome parses text: the experiment 6 port reads "GO" from a summary. st needs typed
   step output for programs to be honest about data.
2. **IDs are the author's job.** Ops must be named, unique per scope and stable across versions.
   Counted suffixes (`build.2`) and unnamed timers work, but they are fragile (experiments 3 and
   4).
3. **Escalation cancels the original.** `escalateAfter` interrupts the review when the deadline
   fires: the reviewer's step is cancelled. "Escalate but keep waiting" is a `race` the author
   writes by hand.
4. **Some KDL has no SDK form yet:**
   - `produces` and baselines;
   - run-owned agents;
   - `concurrent-runs`;
   - `revisions="human-only"`;
   - metric loops (`keep-if`, `plateau`);
   - messages and attention, for which experiment 6 wrote stand-ins.
5. **The program must stay pure.** Reading `Date.now()` or using I/O inside the program breaks
   replay. The Rust bridge (experiment 5) enforces this; Node does not.
6. **Effect is a big dependency for a mission author.** The control-flow vocabulary is Effect's:
   `Effect.result`, `scope` as a pipe. People who know Effect will find it natural; others face
   a learning curve that KDL does not have.

## Verdict

The SDK works and is small, because Effect already has the combinators missions need. The cost
is moved, not removed: the author now owns op names, versioning and purity, which KDL's
declarative shape gives for free.
