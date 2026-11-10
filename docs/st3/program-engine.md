# Program engine

Status: accepted design (owner, 2026-11-10). The decisions below are taken; the [build
plan](#build-plan) says which milestone ships each part. Behavior this document describes is
released only as far as the milestones that shipped it.

This document specifies how the daemon runs `Fractal.mission(intent, program)`: a mission whose
body is code that branches on step results. It builds on the [mission graph
runtime](mission-graph-runtime.md) (runs, generations, steps, revisions) and the [schema
registry](schema.md). The preview API it executes is in `docs/site/concepts/steps.md` and
`docs/site/guides/missions-in-practice.md` (Effect variant).

## Why

A static mission graph cannot branch on a step's value, pass values between steps, or know which
step failed when it compensates. Factory's landing mission needs all three: it routes a change by
its classes, reviews the merged commit, and reverts exactly what a refused publication brought in.
As data it took five per-class missions, a publish mission per main head, a subscription intake
and run-data side channels. As a program it is one 80-line body.

Two earlier ways to run it were measured on scratch instances:

| Run model | Routine change live (median) | Cost |
|---|---|---|
| Data DSL, native graph | 11.4 s | ~3.3 s reconciler hop per step handoff, 1.6–9.6 s subscription hop to publish |
| Per-run engine agent (program in a run-scoped seat) | 6.9 s | ~4.2 s agent launch and first claim per run; the seat needs the union of every step's authority |
| **Daemon program engine (this design, spike)** | **4.0 s** | handoff 0.01–0.02 s; start → first step one reconciler pass |

## Decisions

Taken by the owner on 2026-11-10:

| # | Decision | Section |
|---|---|---|
| D1 | Program JavaScript runs in one daemon-supervised Bun host per member; the engine (dispatch, journal, replay, identity, outcome, supervision) is in the daemon. | §1, §2 |
| D2 | Steps act as `mission-run/RUN` with the authority the revision declares, intersected with the starter's authority. | §3 |
| D3 | A program revision is its bundle's sha256. | §4 |
| D4 | The journal is the registered `program.step` claim on the program's step run. | §5 |
| D5 | A serialized section is a child run of a mission with `concurrent-runs max=1`, not a native lane. | §7 |
| D6 | The data DSL compiles into programs: one execution model. | §9 |
| D7 | The program API is the `Fractal` namespace of the `fractal` package, built on Effect's `effect/workflow`: `Fractal.step` is an `Activity`, `Fractal.ask` a `DurableDeferred`, the daemon is the `WorkflowEngine` and its journal is native claims. Mission inputs and outputs are Effect Schemas. | §11 |

## Requirements

### Durable

- **R01 Journal before continuing:** a program continues past a step only after the step's result
  is durable in the graph.
- **R02 Resume:** a program interrupted by a daemon restart, a host crash or a member restart
  continues from its journal without repeating a journaled step.
- **R03 Fenced effects:** a step key has at most one journaled result per attempt. A second,
  different result for the same key and attempt is refused.

### Versioned

- **R04 Pinned code:** a run executes exactly the bytes its mission revision pins.
- **R05 Old runs finish on old code:** publishing new program code never changes a running
  generation; a run moves to new code only by a new generation.
- **R06 No divergent replay:** replay never feeds a journal to different code.

### Accountable

- **R07 Steps act as the run:** a program's effects act as the mission run under authority the
  mission revision declares, not as an engine agent and not with the union of all steps' authority.

### Composable

- **R08 Linked children:** a child run a program starts is linked to the program's step;
  cancelling the parent cancels the child, and a failed child is a typed failure in the parent.
- **R09 Serialized sections:** a program can run a section that at most one run executes at a time.

### Observable

- **R10 Inner steps are visible:** the run view, `missions show` and `missions waiting-on` show
  each inner step's status, timing and failure.
- **R11 Stalls are detected:** a program step that makes no journal progress, or whose host is
  gone, raises the same run fault as other stalled work.

### One model

- **R12 One execution model:** a mission authored in the data DSL runs on the same engine as a
  program; the native step graph stays the substrate for worker claims, gates and products.

### Fast

- **R13 Handoff:** the median handoff between two inner steps is under 50 ms on an idle member.
- **R14 First step:** the first inner step starts within one reconciler pass of the run start.

## Acceptable tradeoffs

- **T01 At-least-once effects:** an effect interrupted between running and journaling runs again.
  Every step effect must be idempotent (merge by `Factory-Change`, revert by `Factory-Revert-Of`).
- **T02 Shared crash domain per member:** one host process runs every program on a member; a host
  crash interrupts all of them, and each resumes from its journal.
- **T03 Fresh start on a new generation:** a revised run restarts its program from the top under
  the new code; only native steps carry forward.

## Spec

### 1. Where program bodies execute

```
fractal daemon (Rust)                          program host (Bun, one per member)
┌──────────────────────────────────────┐ stdin  ┌──────────────────────────────────┐
│ reconciler: program step ready →     │──────▶ │ loads bundle@sha256 (once)        │
│ ProgramEngine.dispatch(step, journal)│        │ runs body; Fractal.step → effect  │
│ journal writes (program.step claims) │◀────── │ journal entry → waits for ack     │
│ step outcome (completed/failed)      │ stdout │ effects: process groups it owns   │
│ supervision: restart host, resume    │        └──────────────────────────────────┘
└──────────────────────────────────────┘
```

The engine (dispatch, journal, replay, identity, outcome, supervision) lives in the daemon. Only
evaluation of the program's JavaScript is delegated, to one daemon-supervised host process per
member. Options considered:

| | Embedded JS runtime in the daemon (deno_core/V8 or rquickjs) | **Supervised host per member (chosen)** | Per-run engine agent (prototype) |
|---|---|---|---|
| Start of a run | none | none (host stays warm) | 2.2–3.6 s launch + ~2 s first claim |
| Handoff between steps | in-process | one pipe round trip, 0.01–0.02 s | 0.02–0.05 s, CLI calls |
| A program bug (loop, OOM, crash) | stalls or kills the daemon and every seat | a crash kills the host and the daemon restarts it, programs resume; a busy loop stalls that host's programs, never the daemon (its writes to the host are queued) | kills one run's agent |
| Effect 4 + Node/Bun APIs the step libraries use | V8 needs Node shims; QuickJS lacks them | native Bun | native Bun |
| Daemon build and binary | +V8 (tens of MB, longer builds) | +1 embedded 5 KB script | none |
| Authority | daemon-held | daemon-held; host has no credential | agent identity = union of steps |
| Process count | 0 | 1 per member | 1 per run |

The host contract is a narrow protocol (§2), so a later embedded runtime can replace the host
without changing a bundle.

### 2. Host protocol

One JSON object per line. The pipe is the host's only write path: a host orphaned by a dead daemon
can run an effect to its end but can never journal it.

| Direction | Message | Fields | Meaning |
|---|---|---|---|
| daemon → host | `start` | `step`, `attempt`, `module`, `sha256`, `payload`, `env`, `cwd`, `journal[]` | start or resume one program step |
| host → daemon | `journal` | `step`, `attempt`, `seq`, `key`, `status` (`started`/`completed`/`failed`), `value`/`error`, `ms` | an inner step began or ended |
| daemon → host | `ack` | `step`, `attempt`, `seq` | the completed/failed entry is durable (R01) |
| daemon → host | `nack` | `step`, `attempt`, `seq`, `error` | the entry could not be written; the program fails with a defect |
| daemon → host | `cancel` | `step`, `attempt`, `reason` | the step ended or moved to another attempt; stop the program and its effects |
| host → daemon | `done` | `step`, `attempt`, `exit` (`{_tag: Success, value}` or `{_tag: Failure, error}`) | the program ended |

Both sides drop a message for another attempt than the one running, and the daemon drops messages
from a host generation it no longer runs. A host that exits takes exactly its own programs with it;
the next host resumes each from its journal.

A bundle is one self-contained ES module whose default export carries the entry
`Symbol.for('fractal.program.v1')`. The program API (`Fractal.mission/step/command`) and Effect
are inside the bundle, so the host is version-free and the bundle hash pins the runtime library
too. The host verifies the hash before loading, runs each step effect in its own process group,
and kills those groups on `cancel` and when its stdin closes. The second does not yet hold in every
case (DQ2).

### 3. Declaration and authority

```kdl
mission "factory/land-program" state="ready" timeout="2h" {
  concurrent-runs max=8
  input "change" kind="text"
  goal "Land one change on main and prove it live, or leave main as it was."
  authority { publish "scratch/*"; write "resource/factory/*" }
  step "program" timeout="1h" {
    program "/abs/bundles/land-aff9555cbaf1.mjs" sha256="aff9555c…48f09"
  }
}
```

`program` makes the step agentless and gives the engine sole ownership of its outcome: the step
may not declare an assignee, gates, products, nested or used missions, or `after-run`, and it may not
be a `finally` step: cancellation cleanup ends final agentless work that has no runtime process, so
a program compensates inside its body.

Steps act as the run (R07). The engine starts every effect with a run credential scoped to
`(run, step run, attempt)`; the daemon binds a request carrying it to the actor
`mission-run/RUN`. That actor holds what the revision's `authority` block grants, intersected with
the authority of whoever started the run, and nothing else. The daemon writes journal claims as
that actor. Changing `authority` is a mission revision, so factory classifies it as an
authority-class change.

### 4. Versioning

A program's revision is its bundle hash: `sha256` is part of the step definition, so it enters the
step's `definition_hash` and the mission revision. A run generation binds one revision, hence one
bundle.

1. `fractal apply` of a mission with a `program` reads the declared path, checks its sha256 and
   stores the bytes as a blob keyed by that hash. The step definition names the hash in a
   `bundle_hash` field, so the graph replicates the blob with the mission declaration, the way it
   replicates every blob a claim's `bundle_hash` names ([attachments](attachments.md#what-travels-and-what-does-not)).
   The member that runs the step reads the blob, never the author's path, which stays as provenance.
2. Every dispatch and every resume reads the blob of the revision's hash, whatever the source path
   holds now (R05).
3. A declaration whose source no longer matches its sha256 is refused at `apply`; a step whose blob
   is missing fails with `program bundle … is not in this member's store` (R06).
4. Revising a running run creates a new generation with a new step run, hence an empty journal: the
   program restarts under the new code (T03). There is no Temporal-style patching.

### 5. Journal

Each inner step writes registered `program.step` claims on the program's step run subject
(`step-run/GENERATION/PATH`), system-only, append, durable:

| Field | Type | Meaning |
|---|---|---|
| `key` | string | step identity in this execution: `id`, then `id#2`, `id#3`… for repeats |
| `status` | `started` / `completed` / `failed` | `started` is for visibility only and never replayed |
| `attempt` | integer | the program step's attempt that wrote it |
| `program` | string | the bundle sha256 that wrote it |
| `value` / `error` | any | the encoded success value, or the typed error with `_tag` and fields |
| `ms` | integer | effect duration |

Idempotency keys: `program:STEP:ATTEMPT:KEY:completed` and `program:STEP:ATTEMPT:KEY:failed`. A
double execution that produces a different result is refused by the store (R03); the daemon then
sends `nack` and the program fails instead of continuing past a result that is not durable.

Replay reads the subject's claims in pages of 500 inside the daemon (no CLI trace limit) and returns
the `completed` and `failed` entries of the current attempt. A resumed attempt continues where it
stopped. A retried attempt starts from the top, because an earlier attempt's compensation (a revert)
may have undone what its completed steps did. A replayed failure comes back with its `_tag` and
fields, so `catchTag` matches as it did live.

Step identity in loops: the n-th occurrence of an id in one execution has key `id#n`. That is
deterministic only for a deterministic body, so two concurrent steps with the same id are refused
(a defect naming the id). `Effect.retry` of a step re-runs it as `id#2`, `id#3`, …

Determinism contract for code between steps: no clock, randomness or I/O outside a step. The program
API offers journaled `Fractal.now` and `Fractal.random` (DQ5). Typed failures are journaled; defects
fail the step (a bug), interrupts (host or daemon gone) are never journaled and resume.

Long-lived loops do not grow one journal forever: a recurring program is a schedule of finite runs,
and an iterative program that needs more than a few thousand steps runs rounds as child runs (§6).

### 6. Linked child runs and worker turns

`Fractal.run(mission, inputs)` asks the daemon to create a child run with
`create_child_mission_run`, the path `uses-mission` already takes: `parent_step_run` is the program
step, the idempotency key is `program:STEP:KEY`, so a resumed program adopts its child instead of
starting another. The child is natively linked: cancelling the parent run cancels the child through
`cancel_descendant_mission_runs_tx` (R08); a capacity-full child start waits (mission graph runtime
§Run ownership). The engine learns the child's terminal state from its own store transition, not by
polling, and journals it under the child step's key.

A worker turn (`Fractal.step(...).pipe(Effect.provide(Fractal.Worker.layer(agent)))`) is a child run
of a one-step mission whose step is assigned to that agent, with the step's `goal` and `success`
schema; the result is the child's declared product. A run-scoped reviewer
(`agent/${ST_MISSION_RUN}/reviewer`, one-shot) is declared by that child mission, so independence
holds as in LandingMission.

Cancellation is eager: when the reconciler terminates a program step (run cancelled, timed out), it
tells the engine, which sends `cancel` to the host; the host interrupts the program and kills its
effects' process groups.

`Fractal.ask` is a native person ask on the program step (`work ask --step` semantics, deadlines and
defaults), resolved through the same journal key.

### 7. Serialized sections

Two concurrent landings must not publish one host at once (R09).

| | Native lane around publish | **Child run of `factory/publish`, `concurrent-runs max=1`** |
|---|---|---|
| Native semantics | a lane needs an owning run that works its front entry | a capacity-full child start waits; already native |
| Visibility | lane entries | a run per publication, with its own journal |
| Cancel | leave the lane | cascades from the landing run |
| Coalescing | the owner can skip superseded heads | a queued publish for an older head finds the host at or past it and is a no-op |
| Cost | lane bookkeeping (0.03 s) plus an owner | child run start (no agent: one reconciler pass) |

Choice: a child run. It needs no standing owner and reuses §6.

### 8. Visibility and stall detection

- `missions show` and the run view project `program.step` claims under the program step: each key
  with its latest status, duration and failure, and child runs linked by key.
- `missions waiting-on` names the innermost open key: a running effect, a child run or an ask.
- Stalls: a program step whose journal shows no new entry within its `stalled-after` raises the
  run fault with scope `program/stalled`, naming the open key. A dead host that the engine cannot
  restart raises `program/host` on every program step it held.

### 9. One execution model

The data DSL lowers into a program instead of a step graph. A data-DSL mission becomes a generated
program body whose bundle the publisher pins; the KDL a person reads stays a projection.

| Data DSL construct | Lowers to |
|---|---|
| `depends-on { step "x" completed }` | `yield*` order |
| sibling steps without mutual dependencies | `Effect.all([...], { concurrency: 'unbounded' })` |
| agentless `exec` step + `exit_code is 0` gate | `Fractal.step({ id }, Fractal.command(argv))` failing on a non-zero exit |
| `assigned-to` worker step, products | worker turn (§6); products are its success schema |
| field / exists / exec gate | a step that reads through the host contract and fails typed |
| human gate | `Fractal.ask` |
| `retry { attempts n }` | `Effect.retry(Schedule.recurs(n - 1))` |
| `finally { revert }` | `Effect.onError` / `catchTag` with the failed key in scope |
| `uses-mission` | `Fractal.run` (§6) |
| run-data side channels | step values |

Native graph execution remains for published KDL missions until each is republished from the DSL.
The `Fractal.steps.*` catalogue (classify, check, merge, publish, verify, revert, review) is one set
of program steps that both authoring forms call.

### 10. Priority

The daemon starts the host directly, so the host and its effects run at the daemon's live-path
priority, never under macOS `taskpolicy -b`. A step that is background work says so on its
command (`Fractal.command(argv, { priority: 'background' })`); this is the program form of
`ST_SEAT_PRIORITY`.

### 11. Program API

The author-facing API is the `Fractal` namespace of the `fractal` package (D7), on `effect/workflow`:

| `Fractal` | `effect/workflow` | Engine |
|---|---|---|
| `Fractal.mission(intent, program)` | `Workflow.make` with `intent.input` as payload and `intent.output` as success schema | one `program` step; the bundle's entry runs the workflow body for the step run's current attempt |
| `Fractal.step({ id, success, error }, effect)` | `Activity.make` named by the step's key (`id`, `id#2`, …) | a `program.step` entry under that key, replayed by the key |
| `Fractal.ask({ id, for, request, deadline, default })` | `DurableDeferred.make` named by the ask's key | a native person ask on the program step; the answer is journaled under the key |
| `Fractal.run(mission, input)` | a child workflow execution | a linked child run (§6), its terminal result journaled under the key |
| — | `WorkflowEngine` | the host's engine: `activityExecute` returns a journaled result or runs the activity and records it through the host protocol; `deferredResult` reads the journal |

Inputs and outputs are Effect Schemas. `intent.input` is a Struct: each field renders as a native
mission `input`, `missions start` checks the inputs against it, and the program receives the decoded
value. The success value is encoded with `intent.output` and journaled under the reserved key
`$result`; a typed failure is encoded with its `_tag` and fields and fails the step with it.

A program waiting on an ask or a child run stays loaded in the host. A replaced host replays the
program to the same wait.

## Design questions

- **DQ1 Bundle distribution:** decided: replicated blobs stored at `apply` (§4).
- **DQ2 Effect processes across a daemon crash:** decided: the host records itself and every effect
  process group it starts in the engine's state directory and removes an entry when that group ends;
  a daemon kills every recorded process and group before it starts a host. Proved by a kill test that
  asserts no survivor.
- **DQ3 Host isolation per bundle:** one process for all bundles, or a Bun worker per bundle hash, so
  one program's busy loop cannot starve others.
- **DQ4 Large values:** step values above the claim field limit go to documents and the journal keeps
  the reference.
- **DQ5 Deterministic helpers:** which non-step sources (clock, randomness, environment) the API
  journals, and whether replay checks the body's step sequence against the journal and refuses a
  mismatch.
- **DQ6 Execution clock:** whether time a program waits on a child run or an ask stops the step's
  timeout clock, as `work wait` does for seats.
- **DQ7 Native graph retirement:** whether native step-graph execution is retired once every
  data-DSL mission lowers to a program.
- **DQ8 Host liveness:** decided: the engine pings the host and kills a host that does not answer
  within a deadline; its programs resume on the next host. Whether a busy loop should take only its
  own bundle down stays DQ3.

## Build plan

Each milestone ships as reviewed and merged pull requests, and routine-landing latency is measured
on a scratch instance at each.

| Milestone | Ships |
|---|---|
| M1 Engine | `program` steps and the `program.step` journal (§5); the supervised host with replay, eager cancellation, the per-member process group (DQ2) and the liveness ping (DQ8); bundles pinned by hash in the member's blob store, read from the declared path on first dispatch; linked child runs with cancellation cascade and serialized sections (§6, §7); the `effect/workflow` binding (§11); mission-run authority (§3); inner steps in the run view and `waiting-on`, and the `program/stalled` and `program/host` faults (§8). |
| M2 Publication | `fractal apply` stores a program mission's bundle as a replicated blob (§4); the `fractal` package's loader renders `Fractal.mission(intent, program)` to a bundle and its KDL declaration, and validates both. Factory's landing mission becomes the first production program. |
| M3 One model | The data DSL compiles into programs (§9), with byte and behaviour parity for existing data-DSL missions; published KDL missions keep running on the native graph until they are republished. |

## Spike results (scratch, macOS, load 80–280)

LandingProgram's routine-change program, unchanged apart from its runtime import, on instance `dpe`
with a local bare repository. Times are seconds after `fractal missions start` returned.

| Run | First step started | Live (publish done) | Run terminal |
|---|---|---|---|
| 9 routine runs | median 0.70 (0.34–2.05) | **median 3.97 (3.41–5.57)** | median 4.78 (4.39–6.17) |
| admission refused → revert | 0.09 | refused 2.94; **main reverted 3.45** | failed 4.12 (`Reverted`) |
| daemon SIGKILL after merge journaled | — | 6.09 | completed 8.43; one merge commit |
| daemon SIGKILL during check | — | 5.48 | completed 6.51 |
| daemon SIGKILL during publish | — | 6.34 | completed 7.63 |
| run cancelled during check | — | — | cancelled; merge never ran; main unchanged |
| revision pins a different hash | — | — | failed at once with the hash mismatch |

Inner step handoff: 0.01–0.02 s. The remaining time is the step effects themselves (render, git,
`fractal apply`) and one reconciler pass before the first step and after the last.
