# Missions as code: findings

Can an st mission be a program, an `Effect.gen` where each `yield*` is a step, gate, ask or
wait, instead of a KDL graph? Six experiments built enough to answer with running code and
measurements. The code is on branch `agent/missions-as-code` under
`experiments/missions-as-code/`, with per-experiment findings beside it. All names in it are
invented.

## The answer

**Not as a replacement for KDL. Yes as one programmed part inside a KDL mission, for control
flow KDL cannot hold.**

- **As an engine, it works.**
  - A ~300-line SDK on Effect.
  - A pure replay core.
  - A driver that runs as an ordinary st exec, records every yield in the graph, and survives
    SIGKILL.
  - Hot reload that is safe by construction.
  - An embedded QuickJS bridge that is deterministic and stoppable.
- **As a mission format, it loses.**
  - On a straight-line mission it is no shorter, it is checked later (at schedule time, not at
    publish), and the clients show it worse.
  - Its plan before the run is an outline, not a graph.
- **It wins in one place: a change's path to main.** Build, CI, fix on failure, intake GO or
  NO-GO, the curator's queue, merge or ejection, and back round again. Today that path lives
  in English in a constraint, because KDL cannot branch on a NO-GO or loop back to an earlier
  stage. As a program it is about 40 lines of code with a test for every way back.

**The shape to build**, if anything:

- A **`program` step** in a KDL mission, whose body is an Effect program and whose ops are
  child runs of that step.
- Seats, products, revisions, concurrency and cleanup stay declared in KDL.
- Run it **out of the daemon first, as an exec** (experiment 2 shows that works today).
- Move it into the daemon on a QuickJS bridge thread only once the gaps below are closed and
  it has earned its place.

A clear no for: replacing KDL wholesale, exact plans before a run, and demo scripts as programs.

## 1. What a mission looks like as a program

```ts
export default mission("orchard/ship-change", (input: ShipInput) =>
  Effect.gen(function* () {
    // parallel, with a retry whose backoff is a durable, named timer
    const [build, docs] = yield* Effect.all([
      step("build", { assignedTo: builder, goal: `Build ${input.change}` }).pipe(
        Effect.retry({ times: 2, schedule: Schedule.exponential("1 minute") }),
        timerName("build-backoff")),
      step("docs", { assignedTo: builder, goal: "Update the docs" })
    ], { concurrency: "unbounded" })

    // a deadline that escalates to a person, who decides what happens
    const review = yield* step("review", { assignedTo: reviewer, ... }).pipe(
      escalateAfter("review-deadline", "4 hours", () =>
        ask("review-overdue", { for: "person/wren", question: "Reassign or stop?",
          answers: [{ id: "reassign", label: "Reassign to the fixer" }, { id: "stop", label: "Stop the change" }] }).pipe(
          Effect.flatMap((a) => a.id === "reassign"
            ? step("review-reassigned", { assignedTo: fixer, ... })
            : Effect.fail({ _tag: "Stopped" })))))

    // a loop: CI, fix on failure, at most three rounds
    for (let round = 1; round <= 3; round++) {
      const ci = yield* gate("ci", { ciPassed: "orchard-ci", ... }).pipe(Effect.result, scope(`round-${round}`))
      if (ci._tag === "Success") return { shipped: true, rounds: round }
      yield* step("fix", { assignedTo: fixer, goal: "Fix the failing checks." }).pipe(scope(`round-${round}`))
    }
    return yield* Effect.fail({ _tag: "CiNeverPassed" })
  }).pipe(Effect.tapError(() => step("report-failure", { ... }).pipe(Effect.ignore))))
```

**What the SDK adds.** It adds very little, because Effect already has retries, timeouts,
races, loops and parallelism:

- four op kinds: step, gate, ask, wait;
- names for ops and timers;
- `patched()` for versioning.

A step's spec is the KDL step's fields, and seats stay declared in KDL.

**How the engine works.** `decide(program, history)` replays the run's history through the
program on a synchronous scheduler. Effect's Clock, Random and Scheduler are replaced, so time
and order come from the history. It returns the ops the program waits on and what to schedule
or cancel. `Effect.sleep`, `Effect.timeout` and every `Schedule` delay become durable timers
without the author doing anything.

## 2. Two real missions, beside their KDL

Ports of two real fleet missions, with names and text invented and the structure exact. A third
port (a queue of two nested missions with "until green" loops) and the full comparison are in
`exp6-port/COMPARISON.md`.

### The simple one: a wash

```kdl
mission "orchard/change" state="ready" {
  input "request" kind="text"
  completion { when "all-steps-exhausted" }
  goal "Land one reviewed orchard change."

  step "implement" timeout="6h" {
    assigned-to "agent/orchard/standing/steward"
    goal "Implement this request in the steward's worktree: ${input.request}. Add focused tests."
  }
  step "verify" timeout="3h" {
    assigned-to "agent/orchard/standing/steward"
    depends-on { step "implement" completed }
    goal "Run the focused tests, the full unit suite and the model-free evals. Record exact evidence."
  }
  step "open-pull-request" timeout="1h" {
    assigned-to "agent/orchard/standing/steward"
    depends-on { step "verify" completed }
    goal "Push the branch and open a short pull request. Publish it as the declared pull-request resource."
    produces {
      resource "mission-run/${ST_MISSION_RUN}/pull-request" { kind "vcs.pull-request"; state "ready" }
    }
  }
  step "approve-merge" timeout="24h" {
    agentless
    depends-on { step "open-pull-request" completed }
    gate "Wren approves the exact pull request" type="human" {
      reviewer "person/wren"
      question "Is this orchard pull request ready to merge?"
      review "resource/mission-run/${ST_MISSION_RUN}/pull-request"
    }
  }
  step "merge" timeout="2h" {
    assigned-to "agent/orchard/standing/steward"
    depends-on { step "approve-merge" completed }
    goal "Read every review and CI result. Fix defects, get another approval if the pull request changes, and merge the approved change."
  }
}
```

```ts
export default mission("orchard/change", (input: ChangeInput) =>
  Effect.gen(function* () {
    const pullRequest = `resource/mission-run/${input.run}/pull-request` // no run context: the run is an input

    yield* step("implement", { timeout: "6h", assignedTo: steward,
      goal: `Implement this request in the steward's worktree: ${input.request}. Add focused tests.` })
    yield* step("verify", { timeout: "3h", assignedTo: steward,
      goal: "Run the focused tests, the full unit suite and the model-free evals. Record exact evidence." })
    // `produces` has no SDK form: the nearest is a field gate; the product is no longer declared
    yield* step("open-pull-request", { timeout: "1h", assignedTo: steward,
      goal: "Push the branch and open a short pull request. Publish it as the declared pull-request resource.",
      gates: [{ name: "the pull request is published", field: ["state", pullRequest, "is", "ready"] }] })
    yield* gate("approve-merge", { human: { reviewer: "person/wren",
      question: "Is this orchard pull request ready to merge?", review: [pullRequest] } })
      .pipe(Effect.timeout("24 hours"))
    return yield* step("merge", { timeout: "2h", assignedTo: steward,
      goal: "Read every review and CI result. Fix defects, get another approval if the pull request changes, and merge the approved change." })
  }))
```

Both are 40 lines. Order is source order, so the program has no `depends-on`. KDL loses nothing
here and keeps three things:

- **Checks at publish.** `apply --dry-run` caught a missing seat before anything ran; the
  program finds it when the step is scheduled.
- **A declared product** (`produces`).
- **A clearer view** in every client.

**Keep KDL.**

### The messy one: a change's path to main

The real mission's graph is three slices of work. The path each slice takes to main is a
sentence:

```kdl
mission "orchard/ledger-changes" state="ready" report-to="agent/orchard/factory" stalled-after="2h" {
  constraint "Builds and tests on grove only. Small pull requests with tests, the existing suites
  green, an intake GO from agent/orchard/intake, then send the pull request and the GO to
  agent/orchard/curator, which queues it; never arm auto-merge yourself. When CI fails, fix it and
  go back through intake. When the queue ejects a pull request, fix it and send it to the curator
  again."
  agent "builder" { identity "ledger-builder"; host "grove"; harness "codex" { ... }; restart "always" }

  step "design" timeout="8h" { assigned-to "agent/${ST_MISSION_RUN}/ledger-builder"; goal "..." }
  step "reads"  timeout="10h" { ...; depends-on { step "design" completed } }
  step "mirror" timeout="10h" { ...; depends-on { step "reads" completed } }
  finally { step "stop-builder" timeout="30m" { agentless; stop "agent/${ST_MISSION_RUN}/ledger-builder" } }
}
```

As a program, that sentence is code, and every way back is explicit:

```ts
// Check first, fix only what failed, at most three rounds.
const greenCi = (builder: string, branch: string) =>
  Effect.gen(function* () {
    for (let round = 1; ; round++) {
      const ci = yield* gate("ci", { ciPassed: "orchard-ci", repo, branch }).pipe(Effect.result, scope(`ci-${round}`))
      if (ci._tag === "Success") return round
      if (round === 3) return yield* Effect.fail(new CiNeverGreen({ branch, reason: ci.failure.reason }))
      yield* step("fix-ci", { assignedTo: builder, goal: `CI failed on ${branch}: ${ci.failure.reason}. Fix the cause and push.` })
        .pipe(scope(`ci-${round}`))
    }
  })

// One slice from a pushed branch to main.
const land = (builder: string, slice: string) =>
  Effect.gen(function* () {
    const opened = yield* step("open-pull-request", { assignedTo: builder, goal: `Open a small pull request for ledger/${slice}.` })
    const pr = pullRequestOf(opened)
    for (let pass = 1; pass <= 3; pass++) {
      const landed = yield* Effect.gen(function* () {
        yield* greenCi(builder, `ledger/${slice}`)
        const verdict = yield* step("intake", { assignedTo: intake, goal: `Review ${pr} and answer GO or NO-GO with reasons.` })
        if (!verdict.summary.startsWith("GO")) {          // no typed output yet: GO is parsed
          yield* step("address-intake", { assignedTo: builder, goal: `Intake said: ${verdict.summary}. Address it and push.` })
          return false                                      // back through CI and intake
        }
        yield* step("queue", { assignedTo: curator, goal: `Queue ${pr} with intake's GO. Do not arm auto-merge.` })
        const merged = yield* waitFor("merged", { merged: pr }).pipe(Effect.result)
        if (merged._tag === "Success") return true
        yield* step("fix-ejection", { assignedTo: builder, goal: `The queue ejected ${pr}: ${merged.failure.reason}. Fix it.` })
        return false                                        // back through CI, intake and the queue
      }).pipe(scope(`pass-${pass}`))
      if (landed) return pr
    }
    return yield* Effect.fail(new NeverLanded({ slice, passes: 3 }))
  }).pipe(scope(`land-${slice}`))

export default mission("orchard/ledger-changes", (input: LedgerInput) => {
  const builder = `agent/${input.run}/ledger-builder`     // the seat itself stays declared in KDL
  return Effect.gen(function* () {
    yield* step("design", { timeout: "8h", assignedTo: builder, goal: "..." })
    // ... design intake until GO, at most three rounds ...
    const shipped: Array<string> = []
    for (const slice of ["reads", "mirror"] as const) {
      yield* step(slice, { timeout: "10h", assignedTo: builder, goal: "..." })
      shipped.push(yield* land(builder, slice))
    }
    return { shipped }
  }).pipe(Effect.ensuring(stopAgent("stop-builder", builder).pipe(Effect.ignore)))   // finally
})
```

**Why KDL cannot hold it.** A test drives "red CI is fixed, a NO-GO goes round again, an
ejection is fixed and requeued": 15 ops across three passes. KDL could only do it as prose, as
today, or as a hand-unrolled graph. Even unrolled it fails, because a gate can only pass or wait,
and `depends-on ... failed` cannot route a NO-GO back to an earlier step.

**What it still needs:**

- **Typed step output.** GO is parsed from a summary string, and the PR number comes out of
  evidence.
- **Intake and the curator as ordinary steps.** Today their GO and queue are messages between
  seats.

**The program wins here.** This is the case for a programmed step.

### And a demo script

A follow-up asked for one of the demo-scripts mission's example demos as a program, so both of
its examples were written as Effect programs (`exp6-port/demo/`).

- **A beat is one more op kind.** `after("1.5s")` is a durable sleep, so a whole demo plays
  out in virtual time.
- **A compiler writes the beats file the program means.** Both programs compile back to their
  original files line for line, and a test checks it.
- **The plain file wins as the thing that plays.** It is shorter, needs no build, and anyone
  can edit it.
- **The program helps in two places:**
  - repetition, through helpers;
  - branching on the person's answer to an alert. One program covers both answers and compiles
    one file per path.
- **One small format addition would cover most of that:** a beat that waits for the alert's
  answer.

## 3. Running it through st, and what the clients see

The program runs as a **declared st exec** inside an agentless step of a host mission. Its
state lives in the graph:

- **Each history event** is one custom claim on `custom/mac/journal/<run>`. The idempotency
  key `<run>/<seq>` makes an append safe to retry, and it fences off a second driver.
- **Each op** is an ordinary mission run, `<run>/op/<op>`, from a mission
  `<program>/ops/<op>`. It is started at an exact revision with a deterministic ID, so enacting
  a command twice returns the same run.
- **Timers** stay in the driver. Their fire time is part of the history, so a replay knows when
  they are due.

| Run | Result |
|---|---|
| Full ship-change runs: build retried after a backoff, a missed 4h review deadline escalated to a person, reassigned, CI red once, fixed, shipped | completed |
| **SIGKILL** to the driver while the review and its deadline were open | st restarted the exec, the driver replayed 11 events, the deadline fired at its original program time; 23 contiguous journal events; no op started twice |
| **Crash between starting a run and journaling it** | on restart the same command returned the same run: one `mission-run.created` |

**What web, tui and iOS show:**

- each op as an ordinary run with a readable name, its seat, status and summary;
- the host run as one working step.

**What they do not show:**

- the ops nested under the program;
- what the program waits on;
- its pending deadlines;
- the journal itself.

## 4. Changing a running program

A version is one content-addressed bundle that carries its own engine and its own Effect, about
290 KB. It is stored as an st document, `doc/mac/programs/<name>@<sha256>`.

**How a reload works:**

1. A reload is a request claim.
2. The driver replays the run's history through the new version.
3. The run moves only if that replay succeeds. Otherwise it is refused and the run finishes on
   its own version, which stays replayable forever from the graph.

| change | building | in review | fixing round 1 | last CI round |
|---|---|---|---|---|
| a step's goal text | moves | moves | waits (open op changed) | moves, change accepted |
| insert a step after review | moves, runs it | moves, runs it | **refused** | **refused** |
| the same behind `patched()` | moves, runs it | moves, runs it | moves, skips it | moves, skips it |
| remove a step | **refused** | **refused** | **refused** | **refused** |
| 5 CI rounds instead of 3 | moves | moves | moves | moves |

All of it ran live through st, including a SIGKILL after a move: the restarted driver loaded
the new version from the document store and finished on it.

**The catch:**

- **Op names and their order are the compatibility contract.**
- **The most common change, "add a step after X", is refused for every run past X** unless the
  author guards it with `patched()`.

## 5. Seeing the plan before it runs

- **Static extraction** with the TypeScript type checker, about 0.5 s per file, at publish.
  On ship-change it finds every op, the parallel group, the retry, the escalation, the CI loop
  and its bound, and the failure handler.
- **A probe** runs the real program against made-up histories.

**Both fail in ordinary ways:**

- computed op IDs;
- choice by data;
- loops without a literal bound;
- branches on a result's content.

The probe reported "complete" on the messy port while missing the whole path to main, because
its made-up intake answer never said GO. Every new Effect idiom needed a fix in the extractor,
and that will continue.

**An outline yes, an exact plan no.**

- Show the outline with unknowns marked, and never a "plan complete" badge.
- Treat the run's history as the truth.
- Anything that needs the complete set of ops before the run, such as authorizing every seat a
  run may wake, has to check each op as it opens, against rules.

## 6. Runtime measurements

**Through st, out of the daemon** (lab instance):

| | |
|---|---|
| per op, for a 3 s stand-in worker | 12–14 s, the same as starting that one-step mission directly: st's step lifecycle, not the driver |
| driver overhead per op | its poll interval, 0.5 s |
| st CLI calls per 11-op run | about 230–300, nearly all polling; 3–15 s inside st |
| restart after SIGKILL | about a second (st's exec restart); the replay itself is under a millisecond |

**Embedded in Rust**, as a pure function from history to next actions, with no host functions.
The host was a shared build machine under heavy load, so take p50s only.

| | QuickJS | Boa | V8 (deno_core) | Node `vm` |
|---|---|---|---|---|
| start, bundle loaded | 9–20 ms | 195 ms | 19–41 ms | 17 ms |
| RSS per engine | 1.7 MB | 4.7 MB | 4.3 MB | |
| full replay, ship-change (11 events) | 0.65 ms | 2.6 ms | 0.10 ms | 0.32 ms |
| full replay, 2000 events | 86–95 ms | 330–500 ms | 6–15 ms | 35–49 ms |
| live session, per event | 14–72 µs | 85–280 µs | 3–8 µs | 6–44 µs |
| a live 2000-event session | 1.6 MB | 4.4 MB | 1.3 MB | |
| byte-identical to Node, repeatable | yes | yes | yes | |
| `for (;;) {}` stopped at a 50 ms budget, engine still usable | yes | yes, by loop count only | yes | |

**Against the daemon's rules:**

- **A full replay per decision is too slow for the reconciler,** and it grows with the square
  of a run's length.
- **A live session is cheap, so the shape is:**
  - the reconciler only enqueues "run R has new events";
  - a bridge thread owns the engines and live sessions;
  - each call has a budget, and a call that overruns faults the run, not the daemon;
  - the commands go back as a write job through the one writer;
  - clients read the journal and op runs, never the engine;
  - idle sessions are evicted and rebuilt by replay when the run's next event arrives.

**QuickJS is the engine.** It is small, starts fast, has a real time budget and memory limit,
and exposes nothing by default.

## 7. Open risks

1. **Clients see a program as loose runs.** Op runs are not children of the program's run.
   The agentless host step cannot report what it waits on (`work progress` needs an agent).
   Timers are invisible to st. These are three st changes: a parent link on run creation, a
   status line for agentless steps, and a waitable timer.
2. **Untyped step output.** Branching on a result means parsing text ("GO"). Steps need a
   schema the worker fills.
3. **Op names are a contract nobody enforces.** Renames, reorders and retries in front of a
   counted name break replay of running runs. A publish-time diff of op sequences, built on the
   static extractor, could warn; it does not exist.
4. **Errors move from publish to run time.** A missing seat or a broken gate shows up when the
   op is scheduled, mid-run, not at `apply`.
5. **KDL features without a program form:**
   - `produces` and baselines;
   - `loop` with rounds shown in the UI;
   - `message`, `attention` and `stop`;
   - `finally` on cancellation, because the history has no cancel event;
   - `revisions="human-only"`;
   - mission-level constraints.

   A programmed step inside a KDL mission keeps most of these declared.
6. **Purity is a promise in Node.** A program that reads the clock or does I/O breaks replay.
   The QuickJS bridge enforces purity; the Node driver does not.
7. **Effect is the authoring language.** People who know Effect find it natural. Everyone else
   faces a learning curve that KDL does not have.
8. **Polling and identity.** The exec driver polls st (about 300 calls per run) and acts as the
   run's requester. It needs a watch primitive and an identity of its own before it runs at
   fleet scale.
9. **Retention and size.** Each version is about 290 KB and must outlive every run that names
   it.
10. **Bridge gaps.**
    - The V8 startup snapshot crashes once Effect has run; a minimal repro is on the branch.
    - deno_core's default runtime is not I/O-free.
    - Live sessions in one engine share `Math.random`, which only Effect's identity hashes
      read.
    - Experiment 2's driver still decides in Node, not through the bridge.

## 8. A demo you can run

```sh
git fetch origin agent/missions-as-code
git worktree add /tmp/missions-as-code origin/agent/missions-as-code
/tmp/missions-as-code/experiments/missions-as-code/demo.sh        # about 2–3 minutes
```

It needs Node 24 and st on the PATH, and it touches nothing of yours. It creates its own
throwaway st instance (`~/.st-instance/mac-demo-<time>`), then:

1. **Prints the plan of `ship-change`** read from its source before anything runs.
2. **Publishes the program as a mission** whose one step is the driver exec, and starts a run.
   Agents and CI are stand-in scripts, and the 4-hour review deadline passes in 24 seconds.
3. **SIGKILLs the driver** while the review and its deadline are open. st restarts it, and it
   resumes by replaying the journal. The deadline escalates, a scripted person answers
   "reassign", CI fails once, is fixed, and the change ships.
4. **Prints the evidence:**
   - the driver's log across the kill;
   - the 23 journal events, contiguous, with no op started twice;
   - every op's own st run and its state: the failed first build, the cancelled review, the
     red first CI round.

`KEEP=1` leaves the instance running so you can look at it in `st ui` with
`ST_INSTANCE=<name>`. The unit tests for everything else:

```sh
npm install && exp5-bridge/build.sh
node --test sdk/*.test.ts exp1-sdk/*.test.ts exp3-plan/*.test.ts exp4-hmr/*.test.ts exp6-port/*.test.ts exp6-port/demo/*.test.ts
```
