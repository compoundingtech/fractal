# Data-DSL missions as programs

Status: proposed design for milestone M3 of the [program engine](program-engine.md#build-plan),
awaiting the owner's answers to the [open questions](#open-questions). Behaviour this document
describes is released only as far as the slices that shipped it.

The program engine runs `Fractal.mission(intent, program)`. Decision D6 makes it the one execution
model: a mission authored in the `fractal` package's data DSL compiles into a program and runs on
the same engine, while the native step graph stays the substrate for worker claims, gates and
products (R12). This document specifies that compilation, what it changes for published missions,
how parity with the native graph is proved, and the slices that ship it.

## The shape

A data-DSL mission compiles into an **orchestrated mission**: its native steps exactly as the data
DSL renders them, one generated `program` step named `orchestrator`, and a mission-level
`orchestrated-by` naming that step. The program decides when each native step starts; the
reconciler runs everything else as it does for any mission.

```kdl
mission "docs/review" report-to="agent/docs/editor" state="ready" timeout="2h" {
  orchestrated-by "orchestrator"
  input "change" kind="text"
  goal "Review one documentation change."
  completion {
    when "all-steps-exhausted"
  }
  step "draft" {
    assigned-to "agent/docs/writer"
    goal "Draft the correction for ${input.change}."
    produces {
      resource "mission-run/${ST_MISSION_RUN}/draft" {
        kind "custom.docs.draft"
      }
    }
  }
  step "review" {
    assigned-to "agent/docs/editor"
    depends-on {
      step "draft" "completed"
    }
    goal "Review the draft."
    gate "approved" {
      field "state" "resource/mission-run/${ST_MISSION_RUN}/draft" "is" "approved"
    }
  }
  step "orchestrator" timeout="2h" {
    program "/bundles/<sha256>.mjs" sha256="<sha256>"
  }
}
```

Apart from `orchestrated-by` and `step "orchestrator"`, the compiled KDL is byte-identical to the
native rendering of the same mission, so the KDL a person reads, the step definitions and their
hashes do not change. The only exceptions are the in-body constructs of §5 and §6. Each keeps its
node and step ID and changes only its own body, in the one way that section names.

| Owned by the program (orchestration) | Owned by the native graph (substrate) |
|---|---|
| when each released node starts (§1) | the step lifecycle: baselines, selector, readiness, declarations, worker claim and report, nested and used missions, products, gates, retry attempts, timeouts |
| parallelism between released nodes | the normal-phase transition: the first final failure ends normal work and cancels the rest ("another step failed") |
| the outcome of settled steps (§5, §6) and the result of program gates (§6) | mission products and gates, the completion frontier, the final phase, the run outcome |
| | reopening a failed run, revisions, cancellation |

## Options considered

| | A. Every step inside the program; worker steps as worker turns (child runs, §6 of the engine) | **B. Program releases native steps (chosen)** | C. One shared interpreter bundle, graph read from the declaration |
|---|---|---|---|
| Interpolation (`${ST_MISSION_RUN}`) in products, gates and goals | names the child run, so a later step's gate on `resource/mission-run/${ST_MISSION_RUN}/…` reads the wrong subject | unchanged | unchanged |
| Run-scoped agents (`agent/${ST_MISSION_RUN}/…`) | one agent per child run: a worker loses its session between steps | one per run, as in the native graph | as B |
| Gates that wait on graph state (field, exists, merged, ci-passed, exec, human feedback) | need a graph-read and wait API in the host contract | native, unchanged | as B |
| `missions show`, step-run subjects, work claims | child runs instead of steps | unchanged plus one program step | as B |
| In-flight run cut over by revision | impossible (no shared step definitions) | completed steps carry forward (identical definitions) | as B |
| Bundle per mission revision | yes | yes | no, but the interpreter then changes with the daemon, against R04 |
| Engine work | a graph-read and wait API for gates | release and its admission rules (§1, §2) | B plus a declaration read in the host contract |

B keeps native semantics wherever the program has nothing to decide, so parity holds by
construction for those parts and the parity tests concentrate on admission.

## Spec

### 1. Declaration and the released set

- `orchestrated-by "STEP"` on a mission names a top-level normal-phase step that declares `program`
  and no authored `depends-on`. The mission must have no other `program` step. A run started with
  `--after RUN` adds its `after-run` dependency to the orchestrator as to every root step, so the
  program starts once that run completes.
- The loader always names the step `orchestrator`. It refuses a data-DSL mission that declares a
  step with that ID (`orchestrator-step-exists`), as a run started with `--after` refuses a mission
  that declares its own `after-run` step.
- **Released nodes** are exactly the top-level normal-phase nodes the mission declares in `steps`
  (steps and loops), except the orchestrator. The reconciler admits a released node only after the
  program releases it.
- **Admitted natively** in an orchestrated mission, as in any mission:
  - the orchestrator;
  - final-phase (`finally`) steps;
  - the synthetic `after-run` step of a run started with `--after RUN`;
  - steps of a nested `mission`, admitted under their released parent step;
  - the steps of loop rounds, which run in each round's child mission under the released loop.
- A released node's `depends-on` stays declared and keeps its meaning as a guard: the daemon refuses
  a release whose step dependencies do not hold, and the program fails with that defect. Graph
  predicate dependencies and baselines are evaluated natively after release, as in any mission.
- `orchestrated-by` is outside every step definition. Every released node's definition hash is
  therefore its native one, except for the settled steps and program gates of §5 and §6.
- A member advertises the orchestrated-mission capability only once it runs both the admission rules
  (§2) and the release protocol (§2). Until then every member refuses `orchestrated-by` at apply,
  as `mission_program=1` fences `program` steps, so a bundle that releases steps never runs on an
  engine without `release`. The in-body constructs raise the capability level: `settled-by` (§5)
  needs level 2 and a program gate (§6) needs level 3. A member below the level refuses the
  declaration.
- `settled-by "program"` and `gate { program }` are allowed only on released nodes, because only
  there does an orchestrator attempt exist to answer them. The loader and `apply` both refuse
  them on the following, where they would wait until the step or mission timed out:
  - final-phase steps;
  - steps of nested missions and loop rounds;
  - a loop's `until` gates;
  - mission-level gates.

### 2. Release, resolution and the run outcome

| Direction | Message | Fields | Meaning |
|---|---|---|---|
| host → daemon | `release` | `step`, `attempt`, `seq`, `key`, `path`, `env?` | admit the released node `path` of this run's generation; `env` adds string variables to its exec and work context |
| daemon → host | `resolved` | `step`, `attempt`, `seq`, `key`, `exit` | the released node ended; its result is journaled under `key` |

`HostContextV1` gains `release(request: { key, path, env? }): Promise<Exit>`. Success is
`{ step_run, status: 'completed', attempt, products?: Record<string, unknown> }`; failure is
`{ _tag: 'StepFailed', step_run, status: 'failed' | 'cancelled', attempt, reason }`, written when
the node is terminal and no native retry attempt remains.

**Release.**
- A released node's journal key is `step:PATH`, and its `program.step` entry names the step run in
  `child`.
- A release is idempotent per generation. Releasing a node that already ran (in a resumed attempt,
  or carried forward into a successor generation) resolves at once from its native state.
- The engine learns that a node ended from its store transition, as for child runs. It requests an
  evaluation of the run on each release, so admission does not wait for a reconciler pass.
- The orchestrator never carries forward into a successor generation. A reopened or revised run
  starts the program again, which re-releases every node; carried-forward nodes resolve at once and
  only the rest run.
- A node the program never releases is settled exactly as the reconciler settles a node it never
  admits, by the same rules for each dependency state. The parity oracle (§9) pins this down.

**The orchestrator is not normal work for the run's outcome.**
- `all-steps-exhausted` and a `completion { depends-on … }` frontier are evaluated without the
  orchestrator.
- When the normal phase ends, the reconciler settles the orchestrator and the engine cancels its
  attempt on the host:
  - `completed` with "the normal phase completed" when normal work completed;
  - `cancelled` with "another step failed" when it did not.
- A cancelled orchestrator never makes a run failed. A mission whose frontier holds while the
  orchestrator is still running therefore completes, also when it has `finally` steps, exactly as
  its native rendering does.
- Only the orchestrator's own failure fails the run, as a failed normal step does. That failure is
  always a defect: a bundle, host or protocol fault, a refused journal entry or a refused release.
- When a released node fails finally, the reconciler ends the normal phase as it does for any normal
  step. The reopen rules (`work retry`, revision of a failed run) therefore see the same causes as
  for the native rendering.

**Visibility.** `missions show`, the run view and `missions waiting-on` fold `step:`, `exec:`,
`ask:` and `gate:` keys into the native step rows they name, so a released step is shown once.

### 3. The generated program

The loader compiles each data-DSL mission into a module whose default export is
`Fractal.orchestrate(graph)`, bundled and pinned like any program mission
([§11 Publication](program-engine.md#11-program-api)). `graph` holds the released nodes with their
step dependencies, the work of each settled step (§5, §6) and each program gate's predicate (§6).
`Fractal.orchestrate` is the interpreter in the `fractal` package.

```ts
// what Fractal.orchestrate runs for each released node, all under one Effect.all
const node = (n: Node) =>
  Effect.gen(function* () {
    const deps = yield* Effect.all(n.dependsOn.map((d) => Deferred.await(done[d.step])))
    const exit = n.dependsOn.every((d, i) => holds(d.state, deps[i]))
      ? yield* Effect.exit(Fractal.release({ id: `step:${n.path}`, path: n.path }))
      : notReleased // satisfies no dependency, as a never-admitted step stays pending
    yield* Deferred.succeed(done[n.path], exit)
  })
yield* Effect.all(graph.nodes.map(node), { concurrency: 'unbounded', discard: true })
```

A settled step's fiber releases the node, runs its work, settles it (§5) and then awaits its
resolution. Program gates are answered by a handler that the interpreter installs for the whole
attempt (§6).

- Its control flow reads only journaled resolutions, journaled in-body results and the run's
  inputs (no clock, randomness or I/O), so replay reproduces it (§5 of the engine).
- Keys derive from step paths and gate names, never from source order or generated code
  positions. Every path is unique, so concurrent fibers never take `id#n` keys, and a revision
  that adds or removes a step does not move another step's key. Only the retry of an inline exec
  repeats a key (`exec:PATH#2`…), inside one fiber.
- The program ends when every node is resolved or not released, or when the reconciler settles it
  at the end of the normal phase (§2). The run's outcome comes from the native rules, never from
  the program's value.

### 4. Construct mapping

| Data DSL | KDL | Compiled |
|---|---|---|
| `Fractal.step(...)`, worker (`assignedTo`, `availableTo`, run-scoped agents, `freshContext`, `keepOpen`, `capacity`) | `step` with `assigned-to`… | released native step |
| `after: [a, b]` / `dependsOn: [completed(a)]`, `Fractal.sequence(a, b, c)` | `depends-on { step "a" "completed" }` | the node awaits `a`'s resolution, then releases; the native guard holds by construction |
| siblings without mutual dependencies | roots or disjoint `depends-on` | released concurrently (`Effect.all`, unbounded) |
| `failed(a)` / `terminal(a)` dependencies | `step "a" "failed"` / `"terminal"` | released only when the state holds, as the native guard admits it |
| graph predicate dependency, `baseline` | `depends-on { field … }`, `baseline` | native on the released step |
| agentless `exec` step + `exit_code is 0` gate | `exec` + `gate { field "exit_code" … }` | released native step; a local exec settled by the program (§5) |
| gates on a step (field, exists, has/lacks, empty, document, merged, ci-passed, exec, LLM) | `gate "NAME" { … }` | native on the released step: they wait on graph state or a worker's report, which the program does not hold |
| human gate on a worker step (approve or feedback) | `gate type="human"` | native: it is part of the worker's report loop |
| human gate on an agentless step (an approval checkpoint) | agentless step + `gate type="human"` | released native step; with Q3, a step settled by `Fractal.ask` (§6) |
| field gate over an earlier step's product | `gate { field … "resource/…" … }` | native |
| TS predicate over a step's typed product (`Fractal.gate.field(h.output, v => …)`) | `gate "NAME" { program }` | a program gate on the step (§6); it has no native form |
| products (`produces`, `Fractal.product`) | `produces { … }` | native; the resolution carries them, decoded with the product's Effect Schema for in-body use |
| inputs (one Effect Schema Struct; `Fractal.input`) | `input "NAME" kind="text"\|"resource"` | native inputs unchanged; the program receives the payload as a Struct of strings (a resource input is its pinned subject) |
| `retry { attempts; backoff }` | `retry { … }` | native attempts on the released step, and resolution waits for the last; in-body for a settled exec (§5) |
| step `timeout`, mission `timeout` | `timeout=` | native, settled steps included; the orchestrator's timeout is the mission's |
| `onFailure: h` | `finally` step plus a guard | unchanged native lowering, also on a settled step, which keeps its node: compensation is final-phase work that also runs on cancel |
| `finally` steps | `finally { … }` | native final phase |
| `completion` (all-steps-exhausted or a frontier) | `completion { … }` | native, evaluated without the orchestrator (§2) |
| `usesMission` / `waitFor` (revision or produced mission), `producesMission`, nested `mission` | `uses-mission`, `produces-mission`, nested `mission` | native on the released step (already a linked child run); nested steps admitted natively (§1) |
| `loop` | `loop { … }` | released native node; rounds stay native child missions |
| `after-run`, `terminal`/`pty`, `document`, declarations in steps (resources, docs, observers, subscriptions) | as rendered | native on the released step |
| a run started with `--after RUN` | synthetic `after-run` step | admitted natively; the orchestrator waits for it (§1) |
| mission header: `concurrentRuns`, `lanes`, `authority`, `schedule`, report settings, mission gates and products | as rendered | native |
| run-data side channels | files in the run directory | unchanged; `env` on release (§6) replaces them where a step needs a value |

### 5. Settled local execs

An agentless step is settled by the program when all of these hold:

- its only work is an `exec` on the run's own member with `restart "never"`;
- its only gate is `exit_code is 0` on that exec;
- it declares no products, declarations, documents, baselines or graph-predicate dependencies;
- its mission declares an `authority` block.

The step keeps its node, its ID, its `depends-on`, its `timeout` and its place in the KDL. Its
`exec`, its gate and its `retry` are replaced by one `settled-by "program"`:

```kdl
step "check" timeout="10m" {
  depends-on {
    step "classify" "completed"
  }
  settled-by "program"
}
```

| Direction | Message | Fields | Meaning |
|---|---|---|---|
| host → daemon | `settle` | `step`, `attempt`, `seq`, `key`, `path`, `exit` | set the released settled step `path` to completed (Success) or failed (Failure, with its reason) |

- A released settled step is working until it is settled. The program runs
  `Fractal.step({ id: 'exec:PATH' }, Fractal.command(argv, { env, cwd }))` and fails it typed on a
  non-zero exit. `retry { attempts n; backoff }` becomes `Effect.retry` with
  `Schedule.recurs(n - 1)` and the backoff (keys `exec:PATH#2`…). The program then sends `settle`,
  and the daemon writes the step's state and acknowledges.
- The native step's `timeout` stays authoritative. When it fires, the step fails, the normal phase
  ends, and the cancelled orchestrator ends the command's process group.
- Dependents, completion frontiers and the `finally` guard an `onFailure` lowers to all keep naming
  the step, so no reference dangles. Their definitions are unchanged.
- The settled step's own definition differs from its native form. Revising a native run onto the
  compiled revision therefore restarts the settled step and every transitive dependent without
  prior completion ([mission revisions](mission-graph-runtime.md#immutable-generations): a changed
  step and every transitive dependent start again). In a landing-style mission whose settled
  `check` precedes worker steps and `merge`, all of them run again; only steps that do not depend
  on a settled step carry forward. Revise only runs whose settled steps have not completed, or
  accept the rerun (idempotent by T01).
- `work retry` on a failed settled step reopens the run as for any step. The new generation's
  program releases it again and runs the command. A completed settled step carries forward like any
  completed step, and its release resolves at once.
- The command acts as `mission-run/RUN` under the mission's `authority` block (R07), which may
  differ from the actor of a native exec. A mission without an `authority` block keeps its execs
  native, so the compile never changes who writes.

### 6. Decisions over held values

The capabilities the native graph cannot express, available only because the mission is a program:

- **Program gates.** `Fractal.gate.field(review.output, (v) => v.verdict === 'GO')` renders as
  `gate "NAME" { program }` on the step. The reconciler evaluates it like any gate, after the
  step's products hold: it asks the engine, which sends the host
  `evaluate { step, attempt, seq, key, path, gate, step_attempt, products }`. The interpreter
  decodes the products with the step's Effect Schemas, runs the predicate and journals the result
  under `gate:PATH/NAME@STEP_ATTEMPT`. The reconciler reads that entry as the gate's result for
  that attempt. A false predicate therefore fails the attempt like any failed gate: native `retry`
  gives the worker another attempt, and `work retry` redoes the step. A replayed evaluation answers
  from the journal. A program gate is allowed only on a released node (§1).
- **Approval checkpoints as asks** (with Q3). An agentless step whose only gate is a human approve
  gate keeps its node, which becomes `settled-by "program"`. The program asks
  `Fractal.ask({ id: 'ask:PATH', for: reviewer, title, request: { review }, deadline, default })`
  and settles the step completed on approval and failed on rejection.
- **Values into steps.** `release` carries `env`, so a compensation or follow-up step receives the
  failed step's path and reason, or a settled step's value, without a run-directory side channel.

### 7. Latency

| Hop | Native graph | Released native step | In-body (settled exec, program gate, ask) |
|---|---|---|---|
| agentless step end → next agentless step started | ~0.2 s (p50 0.21 s, p95 0.32 s), because a run is evaluated again at once from an exec's exit | engine hop 0.01–0.02 s (spike) plus the run evaluation that admits the node; about the native figure, measured in slice 4 | 0.01–0.02 s (spike); R13 under 50 ms |
| step end → worker step claimed | admission as above, then the seat's wake, which dominates; not measured separately | same | — |
| run start → first step | one reconciler pass | one pass for the orchestrator, then a release | as released |

Releasing native steps is about as fast as native admission, not faster. It is not what makes the
model fast. The gain is the in-body path: a settled exec skips exec declaration, convergence and
observation, and a program gate skips a reconciler round. Each still pays one settle or evaluate
hop. The landing comparison in the [engine design](program-engine.md#why) (11.4 s native, 4.0 s as
a program) was measured before a run was evaluated again from an exec's exit. With that change, the
native side of that comparison shrinks to about 0.2 s per agentless hop.

### 8. Published missions and state

| State | Class | What happens |
|---|---|---|
| Mission revisions published from KDL by hand | authoritative | unchanged; native orchestration keeps running them (DQ7) |
| Revisions published from the data DSL before the cutover | authoritative | keep running natively until republished; a schedule pinned to `mission@revision` keeps starting that revision until re-pinned |
| Revisions published from the data DSL after the cutover | authoritative, additive | a new revision of the same mission ID (`orchestrated-by` plus the orchestrator); new runs use the engine |
| Runs in flight at the cutover | authoritative | finish on their revision (R05); `missions revise RUN` cuts one over, carrying completed steps forward because their definitions are identical, except settled steps and their transitive dependents, which start again (§5) |
| `program.step` journal entries (`step:`, `exec:`, `gate:`, `ask:` keys) | authoritative, additive | new keys under the existing claim kind; no reader arm |
| Run view, `waiting-on` and `missions show` projections | rebuildable from claims | rebuilt; no migration |
| Bundles | generated, content-addressed | a new hash is a new blob |
| Rendered KDL fixtures and goldens | generated | regenerated |

No old record changes meaning, so no converter is needed: native orchestration is the reader for
revisions without `orchestrated-by`. Its deletion trigger is a query, not a date: no ready mission
and no active run whose revision lacks `orchestrated-by` on any member (DQ7).

Rollback is the previous release of the `fractal` package. Its rendering republishes the native
form as the next revision. Runs started on the orchestrated revision either finish on it or are
revised back, again carrying completed steps forward.

Upgrading the `fractal` package changes the interpreter inlined in every bundle, so it revises
every compiled mission once (§2 of the engine: the bundle pins the runtime library). In-flight runs
are unaffected; the cost is one revision per mission per package upgrade (Q4).

### 9. Parity

Parity means the same mission, rendered natively and compiled, produces the same **observable
run**:

- per node: terminal status, attempt count, claimant, products (subject and fields), gate results,
  person answers, child runs (mission, inputs, outcome);
- the happens-before order between node ends and starts;
- the run's phases, outcome and reasons, final-phase steps and faults;
- the effects of reopen (`work retry`), revision and cancellation.

The trace excludes the orchestrator and its `program.step` claims, and it excludes timestamps (only
order counts). Run, generation and step-run identifiers are renamed canonically. Each in-body
construct has one named normalization row, and parity is checked after normalization:

| Construct | Native trace | Compiled trace | Normalized as |
|---|---|---|---|
| settled exec (§5) | an `exec/` subject, the `exit_code` gate result, native attempts | `exec:PATH`, `exec:PATH#n` entries, one settle | the step's status and attempt count (n) |
| approval ask (§6) | a human gate review on the step | an `ask:PATH` answer and one settle | the step's status and the person's decision |
| program gate (§6) | none (no native form) | `gate:PATH/NAME@n` entries | golden trace only |

**Tier 1: oracle and properties (every PR, in `pnpm --filter fractal test`).**
- **Oracle.** A TypeScript model of the native admission and normal-phase rules, written from the
  reconciler, serves as the native substrate. It covers `step_dependencies_hold`,
  `mission_completion_selected` and the normal-phase transition in
  `crates/fractal-reconcile/src/reconcile.rs`, plus the orchestrator rules of §2. The compiled
  program runs against it on the in-memory host (`src/program-host.fixture.ts`).
- **Generated graphs.** Property tests generate graphs from Effect Schema arbitraries:
  - up to 12 nodes, with random acyclic dependencies in `completed`, `failed` and `terminal` states;
  - worker, agentless, exec and settled kinds;
  - retry attempts, final steps and completion frontiers;
  - scripted outcomes per attempt.
- **Assertions.**
  - Every release satisfies its guard.
  - The released set and the resolutions equal the oracle's.
  - The trace equals the oracle's native trace.
  - Killing the host at a random journal entry and replaying reproduces the same trace.
- **Limit.** The oracle is also the substrate, so tier 1 proves only that the interpreter agrees
  with the model. Bugs in how the program interacts with the reconciler show up only in tier 2.

**Tier 2: golden runs on a scratch daemon (fixtures per PR, seeded sample scheduled).**
- **Setup.** Each fixture (`daily-review`, `pr-landing`, `upstream-repin`) and a fixed-seed sample
  of tier-1 graphs is published twice to isolated scratch instances (`fractal/testing`'s
  `scratchDaemonLayer`): once as the native rendering, once compiled.
- **Drivers.** A scripted worker claims and reports with scripted products, and a scripted person
  answers gates and asks.
- **Check.** The normalized traces must be equal and match the stored golden trace
  (`FRACTAL_UPDATE_GOLDENS=1` rewrites it). A tier-2 trace that differs from the oracle's is treated
  as an oracle bug before it is treated as a compiler bug.
- **Scenarios.** Each scenario runs both forms:
  - a final failure, then `work retry` reopening the run;
  - a completion frontier that holds while later released nodes are unreleased, with a `finally`
    step (the run completes, §2);
  - a run started with `--after RUN`;
  - a released step with a nested mission;
  - a loop of several rounds;
  - cancellation during a step with a final phase;
  - a mission gate failure;
  - step and mission timeouts, including the timeout of a settled step;
  - a failed settled exec with a dependent step and an `onFailure` compensation;
  - `work retry` of a settled step;
  - a false program gate under native `retry`;
  - daemon SIGKILL and host kill mid-run;
  - an in-flight native run revised onto the compiled revision, in two variants. Without settled
    steps, every completed step carries forward. With a completed settled `check` before a
    completed worker step and a pending `merge`, the expected trace is: `check`, the worker step
    and `merge` start again in the successor generation; steps independent of `check` carry
    forward; the outcome is the native rendering's for that order.
- **Refusals.** `settled-by` and `gate { program }` on a final, nested or round step, in a loop's
  `until`, or as a mission gate are refused by the loader and by `apply` (§1).

**Rendering parity (every PR).** The test runs over every fixture and a corpus of published
data-DSL trees. The compiled KDL must equal the native KDL plus exactly two nodes, the
`orchestrated-by` node and the orchestrator. The only other allowed difference is each settled step
whose body is replaced by `settled-by "program"`. A mission with a program gate has no native
rendering, so its parity is golden-only.

## Slices

Stacked pull requests of about 300 changed lines; each updates this document and the affected docs
to describe what it shipped.

| # | Slice | Size | Base |
|---|---|---|---|
| 1 | **Declaration:** `orchestrated-by` grammar (parse, render, the rules of §1), the reserved orchestrator ID, refusal at apply on every member, step definition hashes independent of it, `schema.md` | ~250 | M1.1a on `main` |
| 2a | **Admission:** the released set and the guard, the orchestrator outcome rules, `step:` entries with `child`, resolution from store transitions (at once for terminal or carried-forward nodes), run evaluation on release, settling unreleased nodes, view folding | ~350 | 1 and M1.2 |
| 2b | **Protocol and capability:** `release`/`resolved` in the host, `HostContextV1.release`, `Fractal.release` in `fractal/effect`; the member advertises the capability, so `orchestrated-by` is accepted from here on | ~200 | 2a |
| 3a | **Interpreter and oracle:** `Fractal.orchestrate`, the tier-1 oracle and property tests on the in-memory host | ~350 | M1.5; parallel with 1–2 |
| 3b | **Compiler:** the loader compiles data-DSL missions (opt-in per tree) to orchestrated KDL plus bundle, refusing the reserved ID; rendering-parity test | ~250 | 2b, 3a and M2 |
| 4a | **Golden harness:** tier 2 on scratch daemons, the three fixtures and the seeded sample | ~250 | 3b |
| 4b | **Scenarios and measurements:** the scenario cases of §9 for released steps, admission-hop and worker-memory measurements | ~200 | 4a |
| 5 | **Cutover:** the loader always compiles data-DSL missions (the opt-in and the native-only rendering path removed); docs; consumers republish a canary mission, then every tree | ~150 | 4b green and the owner's go |
| 6 | **Settled local execs** (§5): `settled-by` grammar and admission, `settle`, capability level 2, compile, refusal outside released nodes, normalization row and their scenarios | ~300 | 5 |
| 7 | **Decisions over held values** (§6): program gates and `evaluate`, capability level 3, refusal outside released nodes (final, nested and round steps, loop `until`, mission gates) in the loader and at `apply`, approval asks (with Q3), `env` into released steps, and their scenarios | ~300 | 5 |

Slices 1 and 3a start at once. 2a and 2b follow 1, and 3b joins them. 4a and 4b follow, then the
cutover. 6 and 7 are independent after 5.

## Risks

- **The host becomes the orchestration path for every data-DSL run.** A host outage stops admission
  for all of them (T02). Running native steps go on, and DQ8 replaces a silent host. Native-only
  missions never depended on the host.
- **A Bun worker per run for the run's whole life.** Data-DSL runs wait hours on agents, and a
  waiting program stays loaded (§11 of the engine). Slice 4b measures worker memory per run. If
  concurrent runs exceed the member's budget, evicting waiting programs and replaying them on
  `resolved` becomes a slice before 5 (Q2).
- **Native rules the oracle misreads.** These edges are listed in tier 2's scenarios, and a mismatch
  blocks slice 5:
  - unreleased nodes;
  - `failed`/`terminal` dependencies under the rule that the first failure ends normal work;
  - frontiers;
  - `--after` runs;
  - nested missions and loops;
  - reopen.
- **Two orchestrators until DQ7.** The reconciler keeps admitting hand-written KDL missions, so
  admission rules change in two places until native orchestration retires.
- **Revision churn on package upgrades** (§8, Q4).
- **Authority enforcement and settled execs.** Under enforced run authority, a settled exec that
  writes outside the mission's `authority` block is refused where its native form was allowed.
  §5's rule and the golden harness catch this before it ships.

## Open questions

| # | Question | Needed before |
|---|---|---|
| Q1 | **Released native steps.** This document refines §9 of the engine: worker steps stay native steps the program releases, instead of worker turns as child runs. Accept? Once answered, it is recorded as decided here and in the engine's decision table. | merging this design |
| Q2 | **Waiting programs.** Evict a program whose fibers all wait on releases, asks or child runs, and replay it on `resolved`. Build it only if slice 4b's measurement requires it, or before the cutover regardless? | slice 5 |
| Q3 | **Approval checkpoints.** Compile an agentless human approve gate to `Fractal.ask` (deadlines, defaults, one person-facing mechanism, but the person sees an ask instead of a gate review), or keep the native gate and offer `Fractal.ask` only as a new construct? | slice 7 |
| Q4 | **Interpreter versioning.** Accept one revision per compiled mission per `fractal` package upgrade, or ship the interpreter with the daemon and give up R04 for orchestration code? | slice 3b |
| Q5 | **DQ7.** Retire native admission once no ready mission or active run lacks `orchestrated-by`, converting hand-written KDL missions by publishing them through the data DSL? | after slice 5 |
| Q6 | **Releases in hand-written programs.** Expose `Fractal.release` to authors, so a program can use native worker steps (shared run-scoped agents, native gates) without a child run per worker turn? | after slice 5 |
