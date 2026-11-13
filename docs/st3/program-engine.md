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
- **R05 Generations pin code; publication replaces active stale runs:** dispatch and resume
  execute the code pinned by their generation. New-revision publication preflights a fresh
  replacement before ordinary source cancellation and cleanup; the replacement waits for
  terminal cleanup, confirmed owned-runtime quiescence, and effective program-effect fencing,
  then restarts on the new code without adopting the source journal.
  Admission refusal leaves the source active with a visible fault. Explicit run revision
  remains a distinct generation transition; replay never substitutes code in a journal.
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
- **T02 Shared crash domain per member:** one host process runs every program on a member, each
  attempt in its own worker; a host process crash interrupts all of them, and each resumes from its
  journal.
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
| A program bug (loop, OOM, crash) | stalls or kills the daemon and every seat | a crash kills the host and the daemon restarts it, programs resume; a busy loop is confined to its program's worker, which the host terminates after the liveness deadline (DQ8); the daemon's writes to the host are queued, so it never blocks | kills one run's agent |
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
| daemon → host | `start` | `step`, `attempt`, `run`, `module`, `sha256`, `payload`, `env`, `cwd`, `journal[]` | start or resume one program step |
| host → daemon | `journal` | `step`, `attempt`, `seq`, `key`, `status` (`started`/`completed`/`failed`), `value`/`error`, `ms` | an inner step began or ended |
| daemon → host | `ack` | `step`, `attempt`, `seq` | the completed/failed entry is durable (R01) |
| daemon → host | `nack` | `step`, `attempt`, `seq`, `error` | the entry could not be written; the program fails with a defect |
| daemon → host | `cancel` | `step`, `attempt`, `reason` | the step ended or moved to another attempt; stop the program and its effects |
| host → daemon | `ending` | `step`, `attempt` | this attempt began termination for any reason; precedes its terminal notification and is not proof of worker exit |
| host → daemon | `cancelled` | `step`, `attempt` | this host awaited the worker's termination/exit and fenced its effect groups after explicit or host-initiated termination; matched to the host incarnation and attempt, not another host's acknowledgement |
| host → daemon | `run` | `step`, `attempt`, `seq`, `key`, `mission`, `input` | start or adopt the linked child run for `key` (§6) |
| host → daemon | `ask` | `step`, `attempt`, `seq`, `key`, `for`, `title`, `request`, `deadline`, `default` | open or adopt the person ask for `key` (§6) |
| daemon → host | `resolved` | `step`, `attempt`, `seq`, `key`, `exit` | the child run or ask for `key` ended; its result is journaled |
| host → daemon | `done` | `step`, `attempt`, `exit` (`{_tag: Success, value}` or `{_tag: Failure, error}`) | the program ended |
| host → daemon | `unresponsive` | `step`, `attempt`, `ms` | the program's worker missed the liveness deadline and began termination; the failure is visible before actual worker exit (DQ8) |
| daemon → host / host → daemon | `ping` / `pong` | `seq` | host liveness (DQ8) |

Both sides drop a message for another attempt than the one running, and the daemon drops messages
from a host generation it no longer runs. A host that exits takes exactly its own programs with it;
the next host resumes each from its journal.

A bundle is one self-contained ES module whose default export carries the entry
`Symbol.for('fractal.program.v1')`. The program API (`Fractal.mission/step/command`) and Effect
are inside the bundle, so the host is version-free and the bundle hash pins the runtime library
too. The engine reads the module from the store's blob of the revision's hash into a verified,
content-addressed file; the host verifies the hash again before loading and runs each program
attempt in its own worker, while its main thread owns the pipe, the effect process groups and
liveness. It starts each step effect in its own process group and kills those groups when the
group's leader exits, on `cancel`, when it terminates the attempt's worker (an ended or
unresponsive program) and when its stdin closes; groups that outlive a crashed host or daemon are
ended by the next daemon (DQ2).

The daemon starts the host with `FRACTAL_PROGRAM_RUNTIME`, or else `bun` on its `PATH`; a member
without either cannot run program steps. Cancellation is eager: every evaluation of a run tells the
engine which of the run's program attempts left their step (ended, failed, timed out, retried,
cancelled with the run, or replaced by a new generation), and the engine sends `cancel` at once and
cancels the child runs and asks the attempt still waits on (§6).

For publication supersession, sending `cancel` is not sufficient to admit the replacement.
The engine waits for that host's matching `cancelled` acknowledgement (or host exit), then
fences recorded leftover effect groups by verified boot/process-start identity. A failed
kill or unprovable identity remains visible as `supersession-effect-fence`; it cannot be
converted into success by a cleanup deadline. Owned runtimes of all predecessor generations
and descendants must likewise be confirmed quiescent. Acknowledgements and runtime observations
wake reconciliation; no polling or replacement execution timeout is introduced. New program
credentials are minted inside the run writer fence only for the current live attempt and
after checking that it does not await automatic publication cutover.

The host does not acknowledge a merely requested `Worker.terminate()`: it awaits that
worker's exit promise, including native I/O still in progress, while continuing to serve
other programs. An ended attempt remains addressable until exit, so a late cancellation
cannot confuse removal from the run map with quiescence.

This lifecycle applies to normal completion, defects, unexpected worker exit, liveness
failure, explicit cancellation, and same-step replacement. The host sends `ending` before
`done` or `unresponsive`, then `cancelled` only after actual exit. The daemon retains
every exact host-incarnation/step/attempt termination independently of its outcome:
removing an attempt from the running map or failing its step cannot erase that fence.
Daemon-initiated failure registers its cancellation proof under the same mutex that
removes the running entry, before persisting failure. The worker-message forwarding
path accepts only `journal`, `run`, and `ask`, not host lifecycle notifications.

A replacement fences only its own supersession lineage: itself, every superseded
predecessor, and their descendants across all generations. Both pending worker
terminations and recorded effect groups use this scope. A cancellation belonging to an
unrelated mission does not hold this replacement or its finalizers, even while that
unrelated native worker is still terminating.

### 3. Declaration and authority

```kdl
mission "factory/land-program" state="ready" timeout="2h" {
  concurrent-runs max=8
  input "change" kind="text"
  goal "Land one change on main and prove it live, or leave main as it was."
  authority { publish "scratch/*"; write "resource/factory/*" }
  step "program" timeout="1h" {
    program "/abs/bundles/land-aff9555cbaf1.mjs" sha256="aff9555c…48f09" stalled-after="10m"
  }
}
```

`program` makes the step agentless and gives the engine sole ownership of its outcome: the step
may not declare an assignee, gates, products, nested or used missions, or `after-run`, and it may not
be a `finally` step: cancellation cleanup ends final agentless work that has no runtime process, so
a program compensates inside its body.

`sha256` is the bundle's 64 lowercase hex digits; the path must be absolute. The optional
`stalled-after` is how long the program may go without a new journal entry before its run reports
it stalled (§8). The step definition stores them as `program {bundle, bundle_hash,
stalled_after_ms?}`.

Steps act as the run (R07). The engine starts every effect with a run credential scoped to
`(run, step run, attempt)`; the daemon binds a request carrying it to the actor
`mission-run/RUN`. That actor holds what the revision's `authority` block grants, intersected with
the authority of whoever started the run, and nothing else. The daemon writes journal claims as
that actor. Changing `authority` is a mission revision, so factory classifies it as an
authority-class change.

#### Run authority

**Grammar.** A top-level mission may declare one `authority` block in the shape of the agent
`*-authority` blocks, one `VERB "PATTERN"` rule per line. `publish`, `start`, `revise` and `cancel`
take mission IDs exactly as `mission-authority` does (no `mission/` prefix); `write` takes a
subject, or a registered family alone, before `/*`. A pattern is exact or a terminal `/*`
namespace covering everything under it. Parsing refuses the block in a nested or loop body, a
repeated block or rule, an empty block, an unknown verb and any other `*`. There is no `"none"`
form: a mission without the block grants its runs nothing. The block is a field of the mission
spec, kept in canonical order and absent when undeclared, so existing revisions hash unchanged and
changing it is a mission revision. Publication holds a revision that declares it until every
member advertises `mission_authority=1`, as for `capacity`. `cancel` is reserved: it parses and
grants nothing yet, because setting another run's outcome takes a person or agent actor.

**Starter.** A run's starter is the actor of its `mission-run.created`: its requester, or, for a
child a program starts (§6), the parent run, `mission-run/PARENT`. The requester stays the human.
The starter's authority is evaluated when the run writes, not copied at start, so a starter that
loses authority takes it from its running runs.

**Effective authority** of `mission-run/RUN` is the `authority` of its current generation's
revision intersected with its starter's. A run starter contributes its own effective authority,
recursively, so a child never exceeds its parent. A person, agent or daemon starter contributes all
authority in free mode, and the rules that restrict it (`rule.set` in enforce mode) decide each of
the run's writes as well. With exact-or-namespace patterns the intersection is exact: for each
pair with one verb, the narrower pattern if one covers the other, nothing otherwise.

**Credential.** For each start or resume of an attempt the engine mints a store capability of kind
`program-run` whose subject is the step run and whose incarnation is the attempt (the secret is
hashed at rest and never journaled), and passes it as `ST_RUN_CREDENTIAL` in the `start`
message's `env`. The host spreads the run env last, so an effect cannot replace it.

**Binding.** The ancestry walk reads `ST_RUN_CREDENTIAL` from each ancestor's exec-time environment
beside `ST_AGENT`, nearest ancestor first; one environment carrying both binds neither. The daemon
binds a request to `mission-run/RUN` only while the credential's attempt is its step run's current,
live attempt, so the credential dies with its attempt and needs no revocation; one that no longer
binds leaves the caller unverifiable. A run-bound request follows the bound-agent rules: no internal
routes, no person headers other than its own run, and `actor`/`requester` must be the run. Handlers
that hold a bound caller to itself read one bound identity (`BoundActor`) for agents and runs
alike, so a run writes no seat's `doc/agents/…` document and is the writer its document writes
name. The CLI defaults its actor to the run when the credential is present and names it on the
trusted Unix client, which the daemon accepts only from a request its credential binds to that
same run. The same-UID boundary is cooperative, as for agents.

**Private integration instances.** A test running inside a mission exec is not a person,
even when it creates a private daemon. Socket or state-directory ownership does not grant
person attribution. A run credential belongs to the issuing store: a scratch store cannot
resolve the enclosing fleet run's credential. Clearing a leaf client's environment alone
does not erase that credential from its ancestors.

Use the existing explicit agent caller contract for isolated integration tests:

1. Create a fresh private directory (mode `0700`), config, state, runtime paths and Unix
   sockets. Do not join a fleet, configure peers, or select an ambient daemon endpoint.
   Start the daemon outside an agent harness tree (for example a separate user service),
   without `ST_AGENT`, `FRACTAL_AGENT` or `ST_RUN_CREDENTIAL` in its startup environment.
   Keep it in the caller's user namespace so native peer/process inspection remains possible.
2. For **each scratch client exec**, supply `ST_AGENT=agent/fixture/scratch`, omit
   `FRACTAL_AGENT` and `ST_RUN_CREDENTIAL`, and pass the explicit scratch endpoint.
   Use `--as agent/fixture/scratch` on `apply` and `missions start`. A test helper should
   expose that actor directly rather than infer a person from an absent ambient agent marker.
   This identity is a test author, not a configured seat that the daemon must launch.
3. Keep default free-mode authorization on that isolated store, or explicitly grant the
   fixture agent the required publish/start authority when testing enforced rules. This
   does not grant it authority on another daemon. Never change fleet rules for a test.
4. Stop the private daemon and remove only its test-owned directory when the test ends.

The closest verified exec-time agent marker binds the scratch request before the walk
reaches the enclosing mission credential. Publication claims and run requesters remain
attributed to `agent/fixture/scratch`, never a person. A process carrying both agent and
run markers is invalid; a scratch agent naming a person or another agent is refused.
This is the ordinary cooperative same-UID agent boundary, not a new instance-owner
credential or a private-instance bypass: peer generation checks, ancestry failures,
foreign-actor checks and enforced authorization remain identical on every daemon.
Environment isolation and explicit endpoints are requirements of the test harness, not
capabilities that authenticate it to a fleet.

**Publication.** A run publishes with `fractal apply` like any other author: `apply` and
`fractal missions publish` take `--as mission-run/RUN`, and `apply` without `--as` publishes as
the harness's own seat, else as the run whose credential it holds. Its `publish` rules decide
which missions it may publish. The commands no run authority grants (agent lifecycle, owned sets,
mission retirement, run outcomes) still take only a person or an agent.

**Journal.** The engine writes `program.step` claims as `mission-run/RUN`; a run may always journal
on its own step runs, whatever it declares.

**Enforcement** is one decision at the store's write admission, beside the rules gate, for every
claim whose actor is a run and every document a run binds. A run's message send stays on the graph
append, where the decision applies, rather than the messages stream. The claim names the grant it
needs, from fields its writer cannot choose: `mission.published` on `mission/ID` needs
`publish ID`; a run's creation and a generation without a predecessor need `start ID`; a
generation with a predecessor needs `revise ID`, and so does everything a revision writes on the
runs it changes (its proposal, carried and terminalized steps, the superseded generation, the
run's and its descendants' states), each on the mission of the run it changes; any other claim
needs `write SUBJECT`. A refusal reaches its caller as `run-authority-denied`. The daemon's
`[programs] authority` says what a write outside the authority does: `enforce` refuses it;
`audit` lets it through and records `rule.audited` on `rule/run-authority`, so a new program can
discover the grants it needs. Each decision reads only each revision's `authority` member along
the starter chain.

**Reach.** Whatever it is granted, and in either mode, a run touches only itself and the runs it
started, transitively: a run's parent is the run of the step it was started under, the tree a
cancellation cascades down (§6). A claim on a run, its generations, step runs or revision
proposals, a `mission-run` declaration a run applies (creation, revision, reset or cancellation),
and a cancellation requested as the run, are refused as
`run-authority-denied` for any other run, so `write "mission-run/*"` reaches no sibling, parent or
unrelated run. A run starts runs only as the children its program starts, never by declaring one.

### 4. Versioning

A program's revision is its bundle hash: `sha256` is part of the step definition, so it enters the
step's `definition_hash` and the mission revision. A run generation binds one revision, hence one
bundle.

For a policy-less mission, publishing a new revision automatically supersedes already-active
stale runs, subject to [replacement admission and cleanup](mission-graph-runtime.md#automatic-publication-supersession).
Existing human-only review and when-idle cutover policies take precedence: publication does not
bypass review or cancel work before its declared idle boundary. Approved proposal cutover uses
a new linked run at the exact approved revision. Neither path rewrites a generation's bundle or
journal. The fresh replacement restarts all programs and steps, and effects must be idempotent
against prior workspace and external effects. Republish of the same revision does nothing; a
same-revision retry begun after publication remains selected until a later new-revision
publication. No journal is adopted and no new data-DSL syntax is introduced.

1. `fractal apply` of a mission with a `program` reads the declared path, checks its sha256 and
   sends the bytes with the publication (`program_bundles`, base64 by sha256, on the apply and
   owned-set apply requests). The daemon checks each against its hash, refuses one no program of
   the intent names, and stores the bytes as a blob keyed by that hash before it publishes. The
   step definition names the hash in a `bundle_hash` field, so the graph replicates the blob with
   the mission declaration, the way it replicates every blob a claim's `bundle_hash` names
   ([attachments](attachments.md#what-travels-and-what-does-not)). The member that runs the step
   reads the blob, never the author's path, which stays as provenance.
2. Every dispatch and every resume reads the blob of the revision's hash, whatever the source path
   holds now (R05).
3. A declaration whose source no longer matches its sha256 is refused by `fractal apply`,
   `apply --dry-run` and `fractal validate`, which all read the bundle locally. A publication whose
   program's blob the member holds neither from this publication nor from an earlier one is
   refused with `missing-program-bundle`, whatever path published it (a replica could never read
   it). A step whose blob is missing fails with `program bundle … is not in this member's store`
   (R06).
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
| `child` | string, optional | the child `mission-run/…` or ask subject the key waits on (§6) |

Idempotency key: `program:STEP:ATTEMPT:KEY:result` for both `completed` and `failed`, so a key has
one result per attempt whatever its status, and a double execution that produces a different
result is refused by the store (R03); the daemon then sends `nack` and the program fails instead of
continuing past a result that is not durable. `started` entries are not fenced.

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

### 6. Linked child runs, worker turns and asks

A program waits on two kinds of things the daemon owns: a linked child run (`run`, from
`Fractal.run` and worker turns) and a native person ask (`ask`, from `Fractal.ask`). The engine
journals a `started` entry for the key when the request arrives, and the program's run evaluation
does the rest: it starts or adopts what the key waits on, journals a second `started` entry whose
`child` names it, and, once it ends, journals the result under the key (with `child` set) and sends
`resolved`. Everything the evaluation looks at is a noted read, so the store transition that ends a
child run or answers an ask evaluates the parent run again; nothing polls. A replayed key never
asks again; a resumed attempt that asks for an open key adopts what that key already waits on. A
request the daemon refuses (an unknown mission, inputs the mission does not accept, an ask without
a person) fails the program step with the reason, as a defect would; a store failure leaves the
step as it is, and the next evaluation tries again. A child run or ask is journaled as `child` even
when it starts just after its program stopped waiting, and is then cancelled with the attempt.

**Child runs.** The evaluation calls `create_child_mission_run`, the path `uses-mission` takes:
`parent_step_run` is the program step, the inputs are the request's, and the idempotency key is
`program:STEP:ATTEMPT:KEY`, so a resumed attempt adopts its child (the run subject follows from the
key) while a retried attempt starts a new one (§5). The child runs for the parent run's requester in
its workspace. A start that finds the mission at its `concurrent-runs` limit waits: the evaluation
notes reads of the runs that hold the capacity, so the end of any of them evaluates the parent
again and the start is retried then. When the child ends, the program receives:

| Child | Exit |
|---|---|
| `completed` | Success `{ run, status: 'completed', products? }`; `products` holds the value of each product the child's steps declare, by full subject (`resource/mission-run/CHILD/…`) |
| `failed` / `cancelled` | Failure `{ _tag: 'ChildFailed', run, status, reason }`; `reason` is the run's end reason, else its failed step's |

A worker turn, `Fractal.step({ id, success }).pipe(Effect.provide(Fractal.Worker.layer({ mission,
input, product })))`, is a child run of `mission` with `input`. Its value is the child's product
`resource/RUN/PRODUCT`, decoded with `success`; the worker mission declares the agent, goal and
product. For an independent turn, use a distinct run-owned subject such as
`agent/${ST_MISSION_RUN}/worker` and `fresh-context` on its assigned step; optionally use `one-shot`
to retire it on exit. `Fractal.Worker.layer` does not fork the program caller's conversation,
mailbox or handoff into the child. A standing worker selected by the child retains its own
mailbox, and `one-shot` alone does not request a fresh session. See
[step context isolation](mission-graph-runtime.md#step-syntax).

Worker turns can request `Fractal.step({ id, success, execution: true })` to return
`{ value, executions }` instead of the decoded product alone. `value` remains domain data;
`executions` contains the child's claim-backed agent invocations, across its steps and attempts.
The child result and `ChildFailed` also carry `executions`, so failed execution can still report
spend. Older journal entries omit it: `undefined` means unknown, not zero. Mission output
schemas must allow an absent `executions` key when replaying those entries. Replay returns the
journaled execution snapshot without consulting a seat's current model or running the child again.
Each execution identifies its work claim, step, attempt, actor and harness incarnation, with the
declared model captured at claim time, observed models, nullable fallback evidence, response token
and micro-USD totals, timing, trace correlation and an incarnation-matched runtime exit receipt.
No usage observation is `usage: null`, not a zero-token observation. Either `unpriced_tokens`
or `unpriced_responses` distinguishes unknown price from free execution: responses without a
reported total still carry the response counter without inventing tokens. Exit evidence recorded after the child result was journaled is
available on mission detail reads, not retroactively inserted into the replayed snapshot.
An effect-form step is not an agent invocation; it retains its ordinary typed result and journal
duration. A process that fails before claiming work has runtime evidence but no claim-backed
execution record. These generic records do not contain domain decisions, review bindings or consent.

**Asks.** `Fractal.ask({ id, for, title, request, deadline, default })` opens a native person ask on
the program step, with `work ask --step` semantics: the same structured requests, deadlines and
default answers (the fleet's ask-deadline policy applies), and the same inbox. Its requester is
`daemon/program`: a program step has no claimant, so the step's attempt fences the ask instead of a
claim. Only a step whose definition runs a program accepts that requester, and the work API refuses
requests that name it. The step waits on a person (`waiting-person`) while any ask is open; its program stays loaded
and its execution clock pauses (DQ6), and a program may hold several asks at once. The program
receives:

| Ask | Exit |
|---|---|
| answered by a person | Success `{ answer?: { type, outcome?, id?, label?, text? }, summary?, resolved_by }`, `resolved_by` the person |
| resolved at its deadline | the same, with the default answer (none for a free-text ask) and `resolved_by: 'deadline'` |
| withdrawn while the run goes on | Failure `{ _tag: 'AskCancelled', ask }` |

Withdrawal is a typed failure, not a successful empty answer. A program may handle
`AskCancelled` (for example with `Effect.catchTag`) and continue. If it leaves that failure
uncaught, its waiting program step fails and the run follows ordinary failure handling,
including declared native `finally` steps; the attempt's other open children and asks are
cancelled. Native Data-DSL steps have no typed program handler: cancelling their ask fails
the waiting step with reason `ask cancelled by ACTOR`, using ordinary retries and `finally`
handling rather than resuming the step as ready.

An ask that ends with its run is never resolved: the program ended with it.

**Cancellation.** Cancelling the parent run cancels its children through
`cancel_descendant_mission_runs_tx` (R08), and its asks end with the run. When a program step ends,
fails, times out or moves to a retried attempt while its run goes on, the engine cancels the child
runs and asks that attempt journaled as `child` and holds no result for, in the same evaluation that
sends the host `cancel`. A program that ends while a child run or ask it started is still open (a
lost race) cancels it the same way.

### 7. Serialized sections

Two concurrent landings must not publish one host at once (R09).

| | Native lane around publish | **Child run of `factory/publish`, `concurrent-runs max=1`** |
|---|---|---|
| Native semantics | a lane needs an owning run that works its front entry | a capacity-full child start waits; already native |
| Visibility | lane entries | a run per publication, with its own journal |
| Cancel | leave the lane | cascades from the landing run |
| Coalescing | the owner can skip superseded heads | a queued publish for an older head finds the host at or past it and is a no-op |
| Cost | lane bookkeeping (0.03 s) plus an owner | child run start (no agent: one reconciler pass) |

Choice: a child run. It needs no standing owner and reuses §6: a publication whose start finds the
publish mission's one run active waits, journaled as `started` without a `child`, and starts in the
evaluation the active run's end triggers.

### 8. Visibility and stall detection

- `missions show` (text and `--json`, field `programs`) projects the current attempt's
  `program.step` claims under the program step: each key with its latest status, start and end,
  duration (`ms`), typed failure, and the child run or ask it waits on (`child`). The view folds a
  step's newest 200 entries and marks older ones `truncated`; the text lists the newest 12 keys.
  The innermost open key is the most recently started key without a result. The run view (client
  v0 mission detail, `run_details[].steps[].program`) carries the same journal with the step's
  open program fault, which the run's `blocker` names when no scheduler fault does.
- `missions waiting-on` gives a program step the kind `program` (never `watching`) and names its
  innermost open key: a running effect with how long it runs, or the child run or ask it waits on.
  The published work row carries that key (`program`, `gate_kind: program`), read from the step's
  newest 64 entries when the row is folded. A program step whose journal has no new entry within
  its `stalled-after` (default 30 minutes) is `no-progress`, so its run is stalled.
- Stalls: a running program step whose journal shows no new entry within its `stalled-after`
  raises a fault on the step with scope `program/stalled`, naming the open key (and the child it
  waits on). The run's evaluation arms the deadline (the last entry, or the start of execution,
  plus `stalled-after`) instead of scanning; each journal entry is a claim on the step, so it moves
  the deadline and closes the fault, as does the step leaving `working`.
- A host that cannot start or be restarted raises `program/host` on every program step it held,
  and so does a program worker the host terminated for missing its liveness deadline (DQ8), on
  that worker's step. A step whose host could not start keeps its state and is dispatched again 30
  seconds later; the fault closes when a host runs it. While `program/host` is open the step does
  not also raise `program/stalled`.
- Both faults close when their run ends (the inactive-fault sweep), and surface as operator
  attention like every reconcile fault that lasts.

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

The author-facing API is the `Fractal` namespace of the `fractal` package's `fractal/effect` entry
point (`import { Fractal } from 'fractal/effect'`, D7), on `effect/workflow`. The entry point's other
named exports (`ProgramEntry`, `HostContextV1`, `ProgramExit`, `ChildRun`, `ChildFailed`,
`AskAnswer`, `AskCancelled`) are the host contract a bundle implements and the exits it decodes.

| `Fractal` | `effect/workflow` | Engine |
|---|---|---|
| `Fractal.mission(intent, program)` | `Workflow.make` with `intent.input` as payload and `intent.output` as success schema | one `program` step; the bundle's entry runs the workflow body for the step run's current attempt |
| `Fractal.step({ id, success, error }, effect)` | `Activity.make` named by the step's key (`id`, `id#2`, …) | a `program.step` entry under that key, replayed by the key |
| `Fractal.step({ id, success }).pipe(Effect.provide(Fractal.Worker.layer({ mission, input, product })))` | `DurableDeferred.make` named by the key | a worker turn: `ctx.child` (protocol `run`) of `mission` with `input`; the value is the child's product `resource/RUN/PRODUCT` decoded with `success` (§6) |
| `Fractal.ask({ id, for, title, request, deadline, default })` | `DurableDeferred.make` named by the ask's key | `ctx.ask` (protocol `ask`): a native person ask on the program step; the answer is journaled under the key, `AskCancelled` when it is withdrawn (§6) |
| `Fractal.run(mission, input, { id })` | `DurableDeferred.make` named by the key (default: the mission's id) | `ctx.child` (protocol `run`): a linked child run; its end is journaled under the key, `ChildFailed` when it does not complete (§6) |
| — | `WorkflowEngine` | the host's engine: `activityExecute` returns a journaled result or runs the activity and records it through the host protocol; `deferredResult` reads the journal, else waits for the daemon's `resolved` |

Inputs and outputs are Effect Schemas. `intent.input` is a Struct: each field renders as a native
mission `input`, `missions start` checks the inputs against it, and the program receives the decoded
value. The success value is encoded with `intent.output` and journaled under the reserved key
`$result`; a typed failure is encoded with its `_tag` and fields and fails the step with it.

A program waiting on an ask or a child run stays loaded in the host. A replaced host replays the
program to the same wait.

**Publication.** The `fractal` package's tree loader (`loadSubjectTree({ root, bundleDir })`)
bundles each `<id>/mission.ts` that declares a program with `bun build --target=bun
--minify-whitespace`, which drops Bun's per-module path comments so the bytes do not depend on where
the tree or its dependencies are checked out, and writes it to
`<bundleDir>/<sha256>.mjs` after checking that its default export carries the entry. The mission
loads as its declaration: the header from the intent (`timeout`, `concurrentRuns`, `authority`),
one text `input` per field of `intent.input`, and one step `program` pinning the bundle by path and
sha256 (`stalledAfter` becomes its `stalled-after`). Rendering it and running `fractal apply` on the
KDL publishes the mission and carries the bundle (§4).

## Design questions

- **DQ1 Bundle distribution:** decided: replicated blobs stored at `apply` (§4).
- **DQ2 Effect processes across a daemon crash:** decided: each effect's process group ends with its
  leader (the host kills the group when the leader exits). The host records each group in the
  engine's state directory with the boot id and the leader's start time, and removes the record when
  it kills the group. Before it starts a host, a daemon kills each recorded group whose leader still
  has the recorded start time, or whose leader is gone while the group lives on, and drops records
  from an earlier boot without a kill, so a reused pid is never signalled. Proved by a kill test that
  asserts no survivor.
- **DQ3 Host isolation:** decided: one host process per member, one worker per program attempt, so
  one program's busy loop cannot starve the others (DQ8).
- **DQ4 Large values:** step values above the claim field limit go to documents and the journal keeps
  the reference.
- **DQ5 Deterministic helpers:** which non-step sources (clock, randomness, environment) the API
  journals, and whether replay checks the body's step sequence against the journal and refuses a
  mismatch.
- **DQ6 Execution clock:** whether time a program waits on a child run or an ask stops the step's
  timeout clock, as `work wait` does for seats. Today an open ask pauses it (the step is
  `waiting-person`) and a child run does not.
- **DQ7 Native graph retirement:** whether native step-graph execution is retired once every
  data-DSL mission lowers to a program.
- **DQ8 Program liveness:** decided: the host pings each program's worker; a worker that does not
  answer within the deadline is terminated and its step fails with the `program/host` fault naming
  the deadline, so a deterministic busy loop fails once instead of looping across restarts. The
  engine pings the host the same way and replaces a host that does not answer; its programs resume.

## Build plan

Each milestone ships as reviewed and merged pull requests, and routine-landing latency is measured
on a scratch instance at each.

| Milestone | Ships |
|---|---|
| M1 Engine | `program` steps and the `program.step` journal (§5); the supervised host with a worker per program, replay, eager cancellation, recorded effect process groups (DQ2) and liveness pings (DQ8); the step definition names its bundle by `bundle_hash`, `fractal apply` carries and validates the bundle and publishing the declaration stores the blob, so neither the field nor definition hashes change in later milestones (§4); linked child runs with cancellation cascade and serialized sections (§6, §7); the `effect/workflow` binding (§11); mission-run authority (§3); inner steps in the run view and `waiting-on`, and the `program/stalled` and `program/host` faults (§8). |
| M2 Publication | The `fractal` package's loader renders `Fractal.mission(intent, program)` to a bundle and its KDL declaration. Factory's landing mission becomes the first production program. |
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
