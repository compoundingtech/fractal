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
DSL renders them, one generated `program` step, and a mission-level `orchestrated-by` naming that
step. The program decides when each native step starts; the reconciler runs everything else as it
does for any mission.

```kdl
mission "docs/review" report-to="agent/docs/editor" state="ready" timeout="2h" {
  orchestrated-by "program"
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
  step "program" timeout="2h" {
    program "/bundles/<sha256>.mjs" sha256="<sha256>"
  }
}
```

Everything except `orchestrated-by` and `step "program"` is byte-identical to the native rendering
of the same mission, so the KDL a person reads, the step definitions and their hashes do not change.

| Owned by the program (orchestration) | Owned by the native graph (substrate) |
|---|---|
| when each normal-phase step or loop starts (its release) | the step lifecycle: baselines, selector, readiness, declarations, worker claim and report, nested and used missions, products, gates, retry attempts, timeouts |
| parallelism between released steps | the normal-phase transition: the first final failure ends normal work and cancels the rest ("another step failed") |
| constructs that run inside the body: local execs (§5), decisions over held values (§6) | mission products and gates, the completion frontier, the final phase, the run outcome |
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
| Engine work | a graph-read and wait API for gates | release and its admission rule (§2) | B plus a declaration read in the host contract |

B keeps native semantics wherever the program has nothing to decide, so parity holds by
construction for those parts and the parity tests concentrate on admission.

## Spec

### 1. Declaration

- `orchestrated-by "STEP"` on a mission names a normal-phase step that declares `program` and has
  no `depends-on`. The mission must have no other `program` step.
- Every other normal-phase node (step or loop) of an orchestrated mission is **released**: the
  reconciler admits it only after the program releases it. Final-phase steps (`finally`) are never
  released; the final phase runs natively after the normal phase, also when the run is cancelled,
  which a program cannot outlive.
- A released node's `depends-on` stays declared and keeps its meaning as a guard: the daemon refuses
  a release whose step dependencies do not hold, and the program fails with that defect. Graph
  predicate dependencies and baselines are evaluated natively after release, as in any mission.
- `orchestrated-by` is outside every step definition, so the step definition hashes of an
  orchestrated mission equal those of its native rendering.
- A member that has not advertised the orchestrated-mission capability refuses the declaration at
  apply, as `mission_program=1` fences `program` steps. This is also the protocol fence for §2: a
  bundle that releases steps never runs on an engine without `release`.

### 2. Release (host protocol and HostContext)

| Direction | Message | Fields | Meaning |
|---|---|---|---|
| host → daemon | `release` | `step`, `attempt`, `seq`, `key`, `path`, `env?` | admit the native node `path` of this run's generation; `env` adds string variables to its exec and work context |
| daemon → host | `resolved` | `step`, `attempt`, `seq`, `key`, `exit` | the released node ended; its result is journaled under `key` |

`HostContextV1` gains `release(request: { key, path, env? }): Promise<Exit>`. Success is
`{ step_run, status: 'completed', attempt, products?: Record<string, unknown> }`; failure is
`{ _tag: 'StepFailed', step_run, status: 'failed' | 'cancelled', attempt, reason }`, written when
the node is terminal and no native retry attempt remains.

- The journal key of a released node is `step:PATH`, its `program.step` entry names the step run in
  `child`. A release is idempotent per generation: releasing a node that already ran (a resumed
  attempt, a successor generation that carried it forward) resolves at once from its native state.
- The engine learns a node's end from its store transition, as for child runs, and requests an
  evaluation of the run on each release, so admission does not wait for a reconciler pass.
- In an orchestrated mission the program step never carries forward into a successor generation: a
  reopened or revised run starts the program again, which re-releases every node; carried-forward
  nodes resolve at once and only the rest run.
- When a released node fails finally, the reconciler ends the normal phase and cancels the program
  step with the other unfinished normal work ("another step failed"), exactly as for any normal
  step, so the reopen rules (`work retry`, revision of a failed run) see the same causes as for the
  native rendering.
- Nodes the program never releases are settled exactly as the reconciler settles nodes it never
  admits; the parity oracle (§9) pins that per dependency state.
- `missions show`, the run view and `missions waiting-on` fold `step:` keys into the native step
  rows they name, so a released step is shown once.

### 3. The generated program

The loader compiles each data-DSL mission into a module whose default export is
`Fractal.orchestrate(graph)`, bundled and pinned as any program mission
([§11 Publication](program-engine.md#11-program-api)). `graph` is the mission's normal-phase nodes
with their step dependencies and any in-body constructs (§5, §6); `Fractal.orchestrate` is the
interpreter in the `fractal` package.

```ts
// what Fractal.orchestrate runs for each normal-phase node, all under one Effect.all
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

- Its control flow reads only journaled resolutions and the run's inputs (no clock, randomness or
  I/O), so replay reproduces it (§5 of the engine).
- Keys derive from step paths, never from source order or generated code positions, and every path
  is unique, so concurrent releases never take `id#n` keys and a revision that adds or removes a
  step does not move another step's key.
- The program completes when every node is resolved or not released; it fails only on defects
  (bundle, host, refused journal entry, refused release). The run's outcome comes from the native
  rules, never from the program's value.

### 4. Construct mapping

| Data DSL | KDL | Compiled |
|---|---|---|
| `Fractal.step(...)`, worker (`assignedTo`, `availableTo`, run-scoped agents, `freshContext`, `keepOpen`, `capacity`) | `step` with `assigned-to`… | released native step |
| `after: [a, b]` / `dependsOn: [completed(a)]`, `Fractal.sequence(a, b, c)` | `depends-on { step "a" "completed" }` | the node awaits `a`'s resolution, then releases; the native guard holds by construction |
| siblings without mutual dependencies | roots or disjoint `depends-on` | released concurrently (`Effect.all`, unbounded) |
| `failed(a)` / `terminal(a)` dependencies | `step "a" "failed"` / `"terminal"` | released only when the state holds, as the native guard admits it |
| graph predicate dependency, `baseline` | `depends-on { field … }`, `baseline` | native on the released step |
| agentless `exec` step + `exit_code is 0` gate | `exec` + `gate { field "exit_code" … }` | released native step; local execs inline (§5) |
| gates on a step (field, exists, has/lacks, empty, document, merged, ci-passed, exec, LLM) | `gate "NAME" { … }` | native on the released step: they wait on graph state or a worker's report, which the program does not hold |
| human gate on a worker step (approve or feedback) | `gate type="human"` | native: it is part of the worker's report loop |
| human gate on an agentless step (an approval checkpoint) | agentless step + `gate type="human"` | released native step; an in-body `Fractal.ask` with deadline and default after Q3 (§6) |
| field gate over an earlier step's typed product | `gate { field … "resource/…" … }` | native; a TS predicate over the product (`gate.field(h.output, v => …)`) is an in-body decision (§6) |
| products (`produces`, `Fractal.product`) | `produces { … }` | native; the resolution carries them, decoded with the product's Effect Schema for in-body use |
| inputs (one Effect Schema Struct; `Fractal.input`) | `input "NAME" kind="text"\|"resource"` | native inputs unchanged; the program receives the payload as a Struct of strings (a resource input is its pinned subject) |
| `retry { attempts; backoff }` | `retry { … }` | native attempts on the released step; resolution waits for the last |
| step `timeout`, mission `timeout` | `timeout=` | native; the program step's timeout is the mission's |
| `onFailure: h` | `finally` step plus a guard | unchanged native lowering: after a final failure the normal phase is over, so compensation is final-phase work that also runs on cancel; for an inline step (§5) it is `Effect.onError` with the failed key in scope |
| `finally` steps | `finally { … }` | native final phase |
| `completion` frontier | `completion { … }` | native |
| `usesMission` / `waitFor` (revision or produced mission), `producesMission`, nested `mission` | `uses-mission`, `produces-mission`, nested `mission` | native on the released step (already a linked child run) |
| `loop` | `loop { … }` | released native node; rounds stay native child missions |
| `after-run`, `terminal`/`pty`, `document`, declarations in steps (resources, docs, observers, subscriptions) | as rendered | native on the released step |
| mission header: `concurrentRuns`, `lanes`, `authority`, `schedule`, report settings, mission gates and products | as rendered | native |
| run-data side channels | files in the run directory | unchanged; typed step values exist only in-body (§5, §6) and in hand-written programs |

### 5. Inline local execs

An agentless step whose only work is an `exec` on the run's own member with `restart "never"`, whose
only gate is `exit_code is 0` on that exec, and that declares no products, declarations, documents,
baselines or graph predicate dependencies compiles to
`Fractal.step({ id: 'exec:ID' }, Fractal.command(argv, { env, cwd }))` failing typed on a non-zero
exit. `retry { attempts n; backoff }` becomes `Effect.retry` with `Schedule.recurs(n - 1)` and the
backoff (keys `exec:ID#2`…), and `timeout` a typed `StepTimedOut` failure that ends the command's
process group.

The command acts as `mission-run/RUN` under the mission's `authority` block (R07), which may differ
from the actor of a native exec. A mission without an `authority` block keeps its execs native, so the
compile never changes who writes. Inline steps run again in a successor generation (T03); whether a
same-revision reopen carries their journal entries forward is Q4.

### 6. Decisions over held values

The capabilities the native graph cannot express, available only because the mission is a program:

- **Predicates over typed products:** `Fractal.gate.field(review.output, (v) => v.verdict === 'GO')`
  on a step is evaluated in-body on the resolved product after the step completes; false fails the
  program with `GateFailed { step, gate }`, journaled under `gate:STEP/NAME`, which fails the run.
- **Approval checkpoints as asks:** an agentless step whose only gate is a human approve gate
  compiles to `Fractal.ask({ id: 'ask:STEP/NAME', for: reviewer, title, request: { review } })`,
  which adds deadlines and defaults (Q3).
- **Values into steps:** `release` carries `env`, so a compensation or follow-up step receives the
  failed step's path and reason, or an inline step's value, without a run-directory side channel.

### 7. Latency

| Hop | Native graph | Released native step | In-body (inline exec, decision, ask) |
|---|---|---|---|
| step end → next step admitted | ~3.3 s measured (2 s pass spacing × 2 passes) | engine hop 0.01–0.02 s (spike) plus one run evaluation; target under 0.1 s, measured in slice 4 | 0.01–0.02 s (spike); R13 under 50 ms |
| run start → first step | one reconciler pass | one pass for the program step, then a release | as released |
| worker step | the agent's wake and work dominate | same | — |

A routine landing measured 11.4 s on the native graph and 4.0 s as a program: the gain is the
in-body path. A graph of released worker steps gains the admission hop only, and a native
reconciler woken by events instead of pass spacing would gain most of that too.

### 8. Published missions and state

| State | Class | What happens |
|---|---|---|
| Mission revisions published from KDL by hand | authoritative | unchanged; native orchestration keeps running them (DQ7) |
| Revisions published from the data DSL before the cutover | authoritative | keep running natively until republished; a schedule pinned to `mission@revision` keeps starting that revision until re-pinned |
| Revisions published from the data DSL after the cutover | authoritative, additive | a new revision of the same mission ID (`orchestrated-by` plus the program step); new runs use the engine |
| Runs in flight at the cutover | authoritative | finish on their revision (R05); `missions revise RUN` cuts one over, carrying completed steps forward because the step definitions are identical |
| `program.step` journal entries (`step:`, `exec:`, `gate:`, `ask:` keys) | authoritative, additive | new keys under the existing claim kind; no reader arm |
| Run view, `waiting-on` and `missions show` projections | rebuildable from claims | rebuilt; no migration |
| Bundles | generated, content-addressed | a new hash is a new blob |
| Rendered KDL fixtures and goldens | generated | regenerated |

No old record changes meaning, so no converter is needed: native orchestration is the reader for
revisions without `orchestrated-by`. Its deletion trigger is a query, not a date: no ready mission
and no active run whose revision lacks `orchestrated-by` on any member (DQ7).

Rollback is the previous release of the `fractal` package: its rendering republishes the native
form as the next revision, and runs started on the orchestrated revision finish on it or are
revised back, again carrying completed steps forward.

Upgrading the `fractal` package changes the interpreter inlined in every bundle, so it revises
every compiled mission once (§2 of the engine: the bundle pins the runtime library). In-flight runs
are unaffected; the cost is one revision per mission per package upgrade (Q5).

### 9. Parity

Parity means the same mission, rendered natively and compiled, produces the same **observable
run**:

- per node: terminal status, attempt count, claimant, products (subject and fields), gate results,
  person answers, child runs (mission, inputs, outcome);
- the happens-before order between node ends and starts;
- the run's phases, outcome and reasons, final-phase steps and faults;
- the effects of reopen (`work retry`), revision and cancellation.

The trace excludes the program step and its `program.step` claims, timestamps (only order counts),
and run, generation and step-run identifiers (canonically renamed). Each in-body construct adds one
named normalization row (an inline exec's key for its native step and `exec/` subject, an ask's
answer for a human gate result); parity is checked after it.

**Tier 1: oracle and properties (every PR, in `pnpm --filter fractal test`).** A TypeScript model of
the native admission and normal-phase rules, written from the reconciler
(`step_dependencies_hold`, `mission_completion_selected` and the normal-phase transition in
`crates/fractal-reconcile/src/reconcile.rs`), serves as the native substrate for the compiled
program running on the in-memory host (`src/program-host.fixture.ts`). Property tests generate
graphs from Effect Schema arbitraries: up to 12 nodes, random acyclic dependencies with
`completed`/`failed`/`terminal` states, worker, agentless and exec kinds, retry attempts, final
steps, completion frontiers and scripted outcomes per attempt. They assert that every release
satisfies its guard, that the released set and resolutions equal the oracle's, that the trace
equals the oracle's native trace, and that killing the host at a random journal entry and replaying
reproduces the same trace.

**Tier 2: golden runs on a scratch daemon (fixtures per PR, seeded sample scheduled).** Each fixture
(`daily-review`, `pr-landing`, `upstream-repin`) and a fixed-seed sample of tier-1 graphs is
published twice to isolated scratch instances (`fractal/testing`'s `scratchDaemonLayer`): the
native rendering and the compiled one. A scripted worker claims and reports with scripted products
and a scripted person answers gates and asks. The normalized traces must be equal and match the
stored golden trace (`FRACTAL_UPDATE_GOLDENS=1` rewrites it). Scenario cases: a final failure and
`work retry` reopening the run; cancellation during a step with a final phase; a mission gate
failure; step and mission timeouts; a loop; daemon SIGKILL and host kill mid-run; an in-flight
native run revised to the compiled revision. The same lane validates the oracle: a tier-2 trace that
differs from the oracle's is an oracle bug before it is a compiler bug.

**Rendering parity (every PR).** For every fixture and for a corpus of published data-DSL trees, the
compiled KDL equals the native KDL plus exactly the `orchestrated-by` node and the program step.

## Slices

Stacked pull requests of about 300 changed lines; each updates this document and the affected docs
to describe what it shipped.

| # | Slice | Size | Base |
|---|---|---|---|
| 1 | **Declaration:** `orchestrated-by` grammar (parse, render, validation rules of §1), capability advertisement and refusal, step definition hashes independent of it, `schema.md` | ~250 | M1.1a on `main` |
| 2a | **Admission:** released nodes and the guard, `step:` journal entries with `child`, resolution from store transitions and at once for terminal or carried-forward nodes, run evaluation on release, no carry-forward of the program step, settling unreleased nodes, view folding | ~350 | 1 and M1.2 |
| 2b | **Protocol:** `release`/`resolved` in the host, `HostContextV1.release`, `Fractal.release` in `fractal/effect` | ~200 | 2a |
| 3a | **Interpreter and oracle:** `Fractal.orchestrate`, the tier-1 oracle and property tests on the in-memory host | ~350 | M1.5; parallel with 1–2 |
| 3b | **Compiler:** the loader compiles data-DSL missions (opt-in per tree) to orchestrated KDL plus bundle; rendering-parity test | ~250 | 1, 3a and M2 |
| 4 | **Golden harness:** tier 2 on scratch daemons, scenario cases, admission-hop and worker-memory measurements | ~400 | 2b, 3b |
| 5 | **Cutover:** the loader always compiles data-DSL missions (the opt-in and the native-only rendering path removed); docs; consumers republish a canary mission, then every tree | ~150 | 4 green and the owner's go |
| 6 | **Inline local execs** (§5) with their normalization row | ~250 | 5 |
| 7 | **Decisions over held values** (§6): typed product predicates, approval asks (after Q3), `env` into released steps | ~300 | 5 |

Slices 1 and 3a start at once; 2a/2b and 3b follow; 4 joins both lines; 6 and 7 are independent
after 5.

## Risks

- **The host becomes the orchestration path for every data-DSL run.** A host outage stops admission
  for all of them (T02); running native steps go on, and DQ8 replaces a silent host. Native-only
  missions never depended on the host.
- **A Bun worker per run for the run's whole life.** Data-DSL runs wait hours on agents, and a
  waiting program stays loaded (§11 of the engine). Slice 4 measures worker memory per run; if
  concurrent runs exceed the member's budget, evicting waiting programs and replaying them on
  `resolved` becomes a slice before 5 (Q2).
- **Native rules the oracle misreads.** Unreleased nodes, `failed`/`terminal` dependencies under the
  first-failure-ends-normal-work rule, loops and reopen are edges the tier-2 lane must cover before
  the cutover; a mismatch blocks slice 5.
- **Two orchestrators until DQ7.** The reconciler keeps admitting hand-written KDL missions, so
  admission rules change in two places until native orchestration retires.
- **Revision churn on package upgrades** (§8, Q5).
- **Authority enforcement and inline execs.** Under enforced run authority an inline exec that writes
  outside the mission's `authority` block is refused where its native form was allowed; §5's rule and
  the golden harness catch it before it ships.

## Open questions

- **Q1 Released native steps.** This document refines §9 of the engine: worker steps stay native
  steps the program releases, instead of worker turns as child runs. Accept?
- **Q2 Waiting programs.** Evict a program whose fibers all wait on releases, asks or child runs, and
  replay it on `resolved`; build it only if slice 4's measurement requires it, or before the
  cutover regardless?
- **Q3 Approval checkpoints.** Compile an agentless human approve gate to `Fractal.ask` (deadlines,
  defaults, one person-facing mechanism; the person sees an ask instead of a gate review), or keep
  the native gate and offer `Fractal.ask` only as a new construct?
- **Q4 Inline journals across a reopen.** For a same-revision successor generation, carry the
  completed entries of inline keys forward (the native graph carries completed execs forward), for
  compiled programs only or for every program (which changes §5 of the engine)?
- **Q5 Interpreter versioning.** Accept one revision per compiled mission per `fractal` package
  upgrade, or ship the interpreter with the daemon and give up R04 for orchestration code?
- **Q6 DQ7.** Retire native admission once no ready mission or active run lacks `orchestrated-by`,
  converting hand-written KDL missions by publishing them through the data DSL?
- **Q7 Releases in hand-written programs.** Expose `Fractal.release` to authors, so a program can use
  native worker steps (shared run-scoped agents, native gates) without a child run per worker turn?
