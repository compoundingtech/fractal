# fractal

Typed authoring for Fractal declarations: agents, missions, schedules and objectives kept as TypeScript modules in a directory tree, validated with Effect schemas or the native readers' rules and rendered to KDL v2. This is the private workspace package `fractal`; it is not published to npm.

| Entrypoint | Runtime | Contents |
|---|---|---|
| `fractal` | isomorphic | The `Fractal` authoring namespace; declaration schemas and the KDL lowering (`agent`, `mission`, `schedule`, `objectiveNode` nodes, `emit`, `fractalKdl`) as named exports |
| `fractal/tree` | Node | `loadSubjectTree`, `programMission`, the define forms and reference types |
| `fractal/testing` | Node | `scratchDaemonLayer`, an isolated daemon for conformance tests |
| `fractal/source-map` | Node; the read trace needs Bun | `sourceMap` and `traceSourceReads`: the `fractal.source-map/v1` document of a rendered tree |
| `fractal/effect` | Bun program host, isomorphic | The `Fractal` namespace with the program forms of `mission` and `step`, plus `ask`, `run`, `command`, `env`, `cwd` and `Worker`; the host contract (`ProgramEntry`, `HostContextV1`) as named exports |

## Authoring

Each declaration is the default export of `<id>/agent.ts`, `<id>/mission.ts`, `<id>/schedule.ts` or `<id>/objective.ts` below a tree root. `meta: import.meta` ties the declaration to its module; the loader derives the ID from the module's directory, so declarations never repeat it. Imported declarations are typed references.

```ts
// garden/worker/agent.ts
import { Fractal } from 'fractal'

export default Fractal.agent({
  meta: import.meta,
  workspace: '/work/garden',
  restart: 'always',
  harness: { kind: 'claude', model: 'sonnet' },
})
```

```ts
// garden/first-note/mission.ts
import { Fractal } from 'fractal'

import worker from '../worker/agent.ts'

export default Fractal.mission({
  meta: import.meta,
  state: 'ready',
  reportTo: worker,
  goal: 'WELCOME.md introduces this workspace in one short paragraph.',
  steps: [{ id: 'write', timeout: '15m', assignedTo: worker, goal: 'WELCOME.md exists.' }],
})
```

An objective (`docs/st3/objectives.md`) is a target over something Fractal measures. A fleet declares intervals and counts over claims, named `NAMESPACE/NAME` by their directory; `objective.ts` needs no `meta` because nothing references an objective, and a `name`, if given, must equal the path ID.

```ts
// ops/change-rollout/objective.ts
import { Fractal } from 'fractal'

import releaseManager from '../release-manager/agent.ts'

export default Fractal.objective({
  about: 'A change, from its author marking it ready to every member that serves it',
  indicator: Fractal.indicator.interval({
    subjectPrefix: 'custom/acme/change/',
    start: { kind: 'custom.acme.change-ready', at: 'ready_at' },
    expect: { kind: 'custom.acme.change-merged', set: 'hosts', at: 'merged_at' },
    stop: { kind: 'custom.acme.change-applied', member: 'host', at: 'applied_at' },
    cancel: 'custom.acme.change-abandoned',
    labels: ['class'],
  }),
  window: { span: '7d', minSamples: 20 },
  targets: [
    { stat: 'p50', within: '60s' },
    { stat: 'p95', within: '5m' },
  ],
  onBreach: Fractal.response.fault({ owner: releaseManager, condition: 'change-rollout-slo-missed' }),
})
```

A count takes one `per-hour` target with `min` or `max`: `Fractal.indicator.count({ subjectPrefix: 'custom/factory/merged-pr/', event: { kind: 'custom.factory.pr-merged', at: 'merged_at' }, labels: ['repository'] })` with `window: { span: '24h' }` and `targets: [{ stat: 'per-hour', min: 10 }]`. Only the count's own row is judged against the target; each label row is a part of it, shown without a verdict.

An interval or count keeps a sample or event only when its start (or event) claim has one of the values of each `only: [{ field, values }]`, one per field; a claim without the field is left out, and left-out ones are counted with the excluded. An interval's `stop` may name its `start` kind when both name different `at` fields: one claim is then the whole sample, as `start: { kind: 'github.workflow-run', at: 'created_at' }, stop: { kind: 'github.workflow-run', at: 'completed_at' }` reads a CI run from creation to completion. Such a stop takes no `expect` or `member`.

`Fractal.objective` checks the rules the native reader enforces and throws on the first broken one. The other indicators (`latency`, `share`, `statement`, `transaction`, `writerHold`, `cpu`, `database`, `divergence`) are native telemetry: Fractal ships them in [`slo/objectives.kdl`](../../slo/objectives.kdl), rendered by [`slo/objectives.kdl.genie.ts`](../../slo/objectives.kdl.genie.ts), and the tree loader refuses them. `src/objective.ts` and `src/kdl.ts` import nothing outside the package, so Genie bundles that file without installing dependencies.

The namespace also holds `step`, `gate`, `person`, `input`, `product`, `t`, `doc`, `pr`, `childMission`, `runId`, `completed`/`failed`/`terminal`, `Timing`, `Launch`, `Review`, `Fragment`, `Mission`, `Round`, `omp` and `pinMission`. `Fractal.defineAgent`, `Fractal.defineMission`, `Fractal.defineSchedule` and `Fractal.defineObjective` are the low-level forms: `{ meta, intent }`, where `intent` may be a function of the loader's `context`.

### Declared admission budgets (breaking change)

Agents and missions, including `fractal/effect` program missions, author admission
metadata only through `budget`:

```ts
budget: {
  scheduling: { priority: 'background', lane: 'check' },
  resources: { memoryMax: '8G', diskMax: '20G' },
}
```

Mission resources additionally accept positive safe-integer `cpuSeconds` and
`tokenBudget`: per mission run, cumulative across attempts, with token pricing
pinned at admission. Sizes are positive integer strings with an optional K/M/G/T
suffix (case-insensitive). These are declared metadata, not enforced limits.
Unknown keys are refused at every budget level. Steps, loops and rounds have no
budget. Keep mission and step `timeout` at the top level, never in `budget`.

This ports effect-utils #2480. The retired authoring keys are refused, including
when a decoder ignores excess properties:

| Removed            | Replacement                  |
| ------------------ | ---------------------------- |
| `declaredPriority` | `budget.scheduling.priority` |
| `declaredLane`     | `budget.scheduling.lane`     |
| `declaredBudgets`  | `budget.resources`           |

Native KDL is unchanged: `declared-priority`, `declared-lane` and
`declared-budgets` retain their order and spelling. Omission emits nothing;
`resources: {}` emits an empty `declared-budgets` block. Agents must move
`cpuSeconds` and `tokenBudget` to the mission budget.

### Native declared-resource references

Agents and top-level missions accept `resourceReferences`, including agent intents
in `Fractal.agent`/`defineAgent` and kit seat inputs derived from `AgentIntent`:

```ts
resourceReferences: [
  {
    name: 'roll-authority',
    subject: 'resource/demo/release-completion',
    reason: 'Read the declared roll authority.',
  },
  { name: 'source', uri: 'git+ssh://example.org/demo%2Frepo' },
]
```

Each entry requires a unique `name` within its owner and exactly one of `subject`
or `uri`. `reason` is optional but must be nonblank when supplied. Names and the
local portion after `resource/` follow native subject-name validation: at most
512 ASCII characters, starting alphanumeric, then alphanumeric or `._-@/`, with
no empty or `..` path segments or trailing slash. URIs follow native absolute-URI
validation: any scheme, at most 4096 ASCII bytes, valid percent escapes, and no
whitespace or forbidden URI characters. URI bytes are preserved, not normalized;
native publication derives their `resource/uri/<sha256>` identity.

Lowering preserves array order and emits actual native children:

```kdl
resource "roll-authority" subject="resource/demo/release-completion" reason="Read the declared roll authority."
resource "source" uri="git+ssh://example.org/demo%2Frepo"
```

Omission and an empty array emit no children. These are declared reference edges,
not kit metadata, admission `budget.resources`, or mission-owned `resources`
(managed resource bodies). Those fields remain independent.
`ResourceReferenceSchema` and its encoded `ResourceReference` type expose the
same reusable validation contract. The native-shape golden is
`fixtures/resource-references.kdl`; `src/resource-references.unit.test.ts`
covers lowering, parsed-node roundtrips, omission, ordering and malformed inputs.

### Harness startup messages

OMP, Codex and Claude harnesses accept an optional `message: { text, id }`:

```ts
Fractal.agent({
  id: 'garden/worker',
  harness: Fractal.omp({
    model: '@generalist_root',
    message: { text: 'Read the garden plan and begin.', id: 'garden-start-v1' },
  }),
})
```

This lowers to `harness "omp" { model "@generalist_root"; message "Read the garden plan and begin." id="garden-start-v1"; }`.
For Codex or Claude, supply the same `message` field on the harness record.
The native runtime delivers it as a one-shot first-turn message keyed by
`(subject, id)`, not as a recurring prompt. Changing either the text or the id
restarts the agent. Text must be nonblank, contain no NUL, and fit in 64 KiB of
UTF-8; the nonempty id must fit in 256 UTF-8 bytes, matching native admission.
Blankness follows Unicode `White_Space`, as in the native Rust reader:
U+0085 alone is blank, while U+FEFF alone is not.
All rendered strings (including node names and property keys) use KDL v2
escaping for control characters, Unicode newlines, BOM and directional controls.
Unpaired UTF-16 surrogates cannot represent Unicode scalars and are rejected.

`step()` and `gate.*()` validate eagerly, so a step handle's `assignedTo` must be an object with an `id`; a path-derived reference works in an inline step object, which the loader lowers.

## Loading and rendering

`loadSubjectTree({ root, context, bundleDir? })` imports every declaration module below `root`, lowers typed references to IDs without copying authored step and gate handles, assembles missions with `missionWire`, decodes the result with the strict schemas, and checks objectives with `Fractal.objective`. Program missions are bundled into `bundleDir` (see [Publishing a program](#publishing-a-program)). Render a loaded subject with the named exports:

```ts
import { agent, emit, mission, objectiveNode } from 'fractal'

const kdl = emit([subject.kind === 'agent' ? agent(intent) : mission(intent)])
const objectiveKdl = emit([objectiveNode({ ...intent, name: subject.id })])
```

Validate the output with `fractal validate` before publishing it with `fractal apply`.

### Source maps

`fractal apply --set … --source-map MAP.json` records where each published revision came from ([owned sets](../../docs/st3/owned-sets.md#source-maps)). `fractal/source-map` builds that document for rendered subjects. It imports nothing outside Node, so a loader can use it from a pinned checkout; the caller passes `parseSync` from `oxc-parser`.

```ts
import { parseSync } from 'oxc-parser'
import { sourceMap, traceSourceReads } from 'fractal/source-map'

// Before the tree is imported, in Bun: wrap the literal consts of the tree's modules.
const trace = traceSourceReads({ parse: parseSync, roots: [root] })
// …load and render the tree…
const { document, coverage } = sourceMap({
  parse: parseSync,
  subjects: rendered.map(({ subject, file, kdl }) => ({ subject, file, kdl })),
  reads: trace.reads(),
  locate: (file) => (file.startsWith(`${root}/`) ? { repository: 'acme/fleet', path: relative(root, file) } : undefined),
  texts: true,
})
```

Each KDL node with its own authoring site gets a `span` and `frames`, outermost first: the `declaration` call site in the subject module, `helper` call sites, and the `value` that was read; `constructor` and `fragment` frames name the call or the reused step object that authors it. Nodes are keyed by their KDL path (`mission[ID]/step[ID]/goal[0]`): agents, missions, steps and gates by their first string argument, other nodes by their index among siblings of the same name. Unlisted nodes inherit the nearest listed ancestor. Positions are 1-based lines and UTF-16 columns with exclusive ends.

The AST pass follows the default export through constructor calls (`Fractal.mission({…})`, `define*({ meta, intent })`), consts and static imports to the authoring object, its properties and array elements, and step objects by their `id`. The read trace covers values that helpers compute: it wraps literal object and array consts only, never frozen values, and nested values only where the initializer is itself a literal, so step and gate handles keep their identity. Values pass through unchanged, so the rendered KDL is byte-identical with and without the trace; `src/source-map.unit.test.ts` checks that.

## Relation to `@fractal/declarations`

[`@fractal/declarations`](../../clients/typescript/fractal-declarations/README.md) is the generated, dependency-free encoder for a subset of the grammar. This package covers more of the grammar (gates, loops, inputs and products, documents, observers, calendar schedules) and path-derived trees. For the shared subset both produce the same declarations; `src/parity.unit.test.ts` and `crates/fractal/tests/validate.rs` check that. [COVERAGE.md](./COVERAGE.md) records grammar coverage and conformance evidence; [docs/requirements.md](./docs/requirements.md) and [docs/spec.md](./docs/spec.md) hold the contract.

## Programs

`fractal/effect` holds the program form of a mission (design: [`docs/st3/program-engine.md`](../../docs/st3/program-engine.md)), built on Effect's `effect/workflow`. A program's body branches on step values. Its bundle runs in the daemon's program host, which journals every step and replays the journal after a restart; this package implements the bundle's side of that host contract.

```ts
import { Effect, Schema } from 'effect'
import { Fractal } from 'fractal/effect'

import factory from '../agent.ts'

export default Fractal.mission(
  {
    meta: import.meta,
    id: 'factory/land',
    goal: 'Land one change on main.',
    reportTo: factory,
    input: Schema.Struct({ change: Schema.String }),
    output: Schema.Struct({ commit: Schema.String }),
  },
  ({ change }) =>
    Effect.gen(function* () {
      const out = yield* Fractal.step(
        { id: 'merge', success: Schema.String },
        Effect.map(Fractal.command(['./merge', change]), (result) => result.stdout.trim()),
      )
      return { commit: out }
    }),
)
```

| Call | `effect/workflow` | Journal |
|---|---|---|
| `Fractal.mission(intent, program)` | a `Workflow` with `intent.input` as payload and `intent.output` as success | the success, encoded with `output`, or the typed failure fails the program step |
| `Fractal.step({ id, success?, error? }, effect)` | an `Activity` named by its key: `id`, then `id#2`… for repeats, so `Effect.retry` runs it again under a new key | its encoded value, or its typed failure with `_tag` and fields; defects and interrupts are not journaled |
| `Fractal.step({ id, success }).pipe(Effect.provide(Fractal.Worker.layer({ mission, input, product })))` | a `DurableDeferred` | a worker turn: a linked child run of `mission`; the product `resource/<child run>/<product>`, decoded with `success`, is the value |
| `Fractal.ask({ id, for, title, request, deadline?, default? })` | a `DurableDeferred` | a native person ask on the program step, answered `AskAnswer` or failed `AskCancelled` |
| `Fractal.run(mission, input, { id? })` | a `DurableDeferred` | a linked child run, `ChildRun` or `ChildFailed` |
| `Fractal.turn({ id, agent, instructions, context, products?, freshContext? })` | a `DurableDeferred` over native `agent.turn` | a real assigned finite work episode; returns `WorkerReport`, refuses with `ResourceError`, or ends with `ChildFailed` |
| `Fractal.review({ id, subject, rubric, reviewer, authors? })` | a `Worker.layer` child run, followed by live verification | adopts the same independent reviewer on replay; moved heads and edited comments invalidate its result |
| `Fractal.command(argv, { env?, cwd?, priority? })` | — | not journaled: run it inside a step; the host runs it as a process group it ends on cancel |

Every `input` field must be required and encode to a string, since each one is a native `input "name" kind="text"`; a payload that does not decode refuses the run with `InvalidInput`. Without `error`, a step or program journals any tagged error as `_tag` plus its fields and the step fails with that plain object (typed `Journaled<E>`), live and on replay, so `catchTag` matches; an untagged error is a defect. With `error`, the schema rebuilds the error as its class, which a program's `error` class needs. Only one fiber may use a step id: concurrent branches would take `id#n` keys in completion order, which replay does not reproduce, so a second branch using the id is a defect. A journal entry the daemon refuses ends the attempt with a defect that the body cannot catch.

A program mission's default export carries `[ProgramEntry]` (`Symbol.for('fractal.program.v1')`), the entry the program host calls with a `HostContextV1`. `fixtures/programs/factory/land-program` is the factory's landing program; `src/program-bundle.unit.test.ts` runs its bundle against an in-memory host.

### Standard native seat turns

Publish a normal `mission.ts` at `fractal/seat-turn/mission.ts` in your subject tree:

```ts
import { Fractal } from 'fractal/effect'
import owner from '../../owner/agent.ts'

export default Fractal.seatTurn({ meta: import.meta, reportTo: owner })
```

The complete example is `fixtures/programs/fractal/seat-turn/mission.ts`. Tree loading,
bundling and publication work exactly as for any program mission; the SDK does not
silently install a published mission. An explicit `id` supports a different tree path.
The three required text inputs are `agent` (full `agent/<id>`), `instructions`, and
`context`. Context is JSON `{ inputs: Record<string,string>, products: [{ path, kind,
fields }], freshContext?: boolean }`. Product paths are run-relative and fields are
scalar constraints. Inputs remain inert context text, not shell arguments or environment.
Fresh context is opt-in, not the default.

`Fractal.lower(graph, { id, host })` routes each assigned agent to the published
`fractal/seat-turn` by default. It sends the expanded step goal and ordered constraints
as `instructions`, the actual assignee as `agent`, and **all** original inputs plus
expanded required product specs inside context JSON. Graph inputs named `agent`,
`instructions`, `context` or `goal` therefore cannot replace the routing protocol.
`workers` remains an optional per-agent override; set `strictWorkers: true` to require
explicit bindings. Overrides can map `goalInput`, `contextInput`, `agentInput` and
`report`; legacy `goalInput`-only overrides retain their spread-input/resource protocol
and refuse instruction-key collisions.

`WorkerReport` contains actual `run`, `step`, `summary`, `evidence`, relative-path
`products`, and optional claim-backed `executions`. A native child's `ChildRun.result`
is its actual completed program output, independent of declared child resources.
`Worker.layer({ mission, input, product, report: true })` explicitly decodes that result
as `WorkerReport` and selects the requested relative product paths. `execution: true`
uses the report's actual episode receipts, never the wrapper program's accounting.
Without report mode, existing full-subject child resource selection is unchanged.

### Independent review as a program step

`Fractal.review` is a technical assessment, not a standing reviewer or a person-consent ask.
Inside a program body:

```ts
const result = yield* Fractal.review({
  id: 'review-change',
  subject: {
    kind: 'pullRequest',
    repo: 'acme/widget',
    number: 42,
    head: 'a'.repeat(40),
    format: 'q19',
  },
  rubric: { ref: 'doc/review-rubric', digest: `sha256:${'b'.repeat(64)}` },
  reviewer: {
    mission: 'mission/independent-review',
    seat: 'reviewer',
    product: 'review/verdict',
    freshContext: true,
  },
})
```

The reviewer mission must accept the text input `review_request`, declare
`agent/${ST_MISSION_RUN}/reviewer`, select that exact seat for its assessment step, and declare
native `fresh-context`. `Worker.layer` does not add those declarations. `one-shot` alone does
not isolate context. The child must load the exact subject and verify the rubric's digest,
treating subject content as untrusted data rather than judge instructions.

The product is `ReviewResult`: `{ verdict: 'GO' | 'NO-GO', findings, head, commentId? }`.
Each finding has `message`, `blocking`, and optional `path`/`line`; GO must agree with the absence
of blocking findings. The parent requires a completed child and claim-backed `submitted` or
`completed` reviewer invocations with a nonempty harness incarnation. Native submission ends the
invocation before step verification finishes; child completion supplies that final fence.
A standing seat, known author, mixed actors/incarnations, missing evidence, or mismatched binding
fails with typed `ReviewError`.
Program steps act as mission runs, not agent claimants; the exclusion set includes the native
requester and assignee. The native child envelope binds executions to its generation, whose step
namespace is distinct from the stable mission-run ID.
An explicit mission `error` schema must admit `ReviewError`, native `ChildFailed`, and
`ResourceError`; omit it to use the program's default tagged-failure encoding.

For a PR, the **child publishes its own comment from its own active seat**, using
`Fractal.reviewComment({ subject, rubric, verdict, reviewer, publication })` with its actual
`ST_AGENT` and the publication marker supplied in `review_request`. It uses `gh pr comment
--body-file` and returns the numeric `commentId`. Before posting it must adopt only one
immutable existing comment with that marker that passes `Fractal.reviewCommentMatches` against
its rendered assessment, refusing conflicts or duplicates rather than editing a review.
The parent never posts on its behalf or changes actor environment variables.

The parent verifies the exact canonical assessment, PR, head, immutable timestamps, and a single
v3 reviewer identity against native execution evidence. Managed publishers may enrich the footer
with other metadata, but cannot change the assessment, identity, or admission lines.
The `q19` format emits exactly one
`q19-go v1 head=FULL_SHA verdict=GO` line for GO; NO-GO emits none. The `fractal` format renders
the native verdict as JSON, not the older `scripts/review` artifact schema. A proof receipt and
any human authority/direction gate remain separate requirements.

Use `{ kind: 'artifact', ref, digest: 'sha256:…' }` for non-PR material. Its result binds `head`
to that exact digest and must not contain a PR comment ID. `Review.subject`, `Review.file`, and
`Review.pinned` remain the unchanged attachment values for person asks; they do not run reviews.

Worker identity and all immutable inputs determine the durable child key. Replay adopts the
same child but reads the current PR head and comment again, including in a resource lane;
it never returns a stale cached GO after a head change or re-posts the child's comment.
These are transition-triggered observations, not a polling loop. Fresh-context remains a
precondition on the pinned child mission: execution evidence identifies its actor/incarnation
but does not expose the mission's context-isolation flag.

### Scoped resource lanes

`Fractal.resources.lane(name, options)` acquires a host-authoritative membership in an Effect
scope, not a JavaScript semaphore. Use its native operations for admitted work:

```ts
const check = Effect.scoped(Effect.gen(function* () {
  const lane = yield* Fractal.resources.lane('checks', {
    host: 'executor',
    maxMembers: 1,
    capacity: { cores: 4, memory: 8 * 1024 ** 3 },
    coverage: 'enforced',
    admissionTimeout: '30m',
  })
  return yield* lane.exec('check', ['tools/check'], { cwd: '/source', timeout: '4h' })
}))
```

Capacity memory is bytes. Admission timeout bounds waiting and preparation; command timeout
starts at payload release, not at scope entry. The host shares its existing seat/step/burst fit
accountant: assigned-agent `capacity` remains a seat raise, never a second reservation. Required
enforcement refuses unsupported coverage rather than silently running a detached command.

`lane.run(id, mission, input)` passes the exact membership to linked execution roots; a child
workflow alone is not a memory reservation or a relocated parent harness. Arbitrary Effects in
the scope do not inherit process containment. `lane.exec` and `lane.run` cross the daemon
boundary; they never call the ordinary program-host `spawn`.
Lane-bound child programs also route ordinary `Fractal.command` calls through that exact native
membership. Their command identities include the journaled step key, so replaying an earlier
step cannot cause a later command to adopt the wrong execution. Republish child bundles with
the resource-capable SDK before using `lane.run`; legacy detached host `spawn` is refused in
this context rather than silently escaping containment.

`ResourceError.code` distinguishes `admission-expired`, `unsupported-coverage`,
`impossible`, `clock-uncertain`, `execution-timeout` and exact-generation
retirement/cancellation outcomes. Queue expiry is not a failed payload execution.
The tagged failure retains both `code` and the host's `message` through program-result
encoding, so admission diagnostics remain available to callers and on replay.

Each acquisition reattaches and validates with the owning host even when a journal contains a
grant-shaped result. Keep acquisition outside journaled `Fractal.step` bodies. Scope closure
sends exact-generation release and awaits confirmed retirement; neither a command's exit nor
a completed program journal entry alone frees capacity. Interruption withdraws the exact
queued request, or cancels its racing admitted generation, through the same native boundary.
Durable host holds are independent admission conditions and are not cleared by scope closure.


### Publishing a program

`loadSubjectTree({ root, context, bundleDir })` bundles every program mission it loads with `bun build --target=bun --minify-whitespace` (`bun` on `PATH`, or the `bun` option) and writes the bundle to `<bundleDir>/<sha256>.mjs`; `bundleDir` must be absolute, and a tree with a program mission and no `bundleDir` is refused. Minified whitespace drops the comment Bun writes above each inlined module with its path from the build directory, so the same sources and lockfile give the same bytes wherever the tree and its dependencies are checked out. Before it pins the bundle the loader checks that its default export carries the entry. The mission then loads like any other, so `emit([mission(intent)])` renders it:

```kdl
mission "factory/land-program" report-to="agent/factory" state="ready" timeout="2h" {
  concurrent-runs max=8
  input "change" kind="text"
  input "reviewer" kind="text"
  goal "Land one change on main and prove it live, or leave main as it was."
  authority {
    publish "scratch/*"
    start "factory/step/review"
    write "resource/factory/*"
  }
  step "program" timeout="2h" {
    program "/bundles/<sha256>.mjs" sha256="<sha256>" stalled-after="10m"
  }
}
```

The intent's `timeout` bounds the mission and its program step, `stalledAfter` becomes the program's `stalled-after`, `concurrentRuns` becomes `concurrent-runs max=N`, and `authority` (`publish`, `start`, `revise` and `write` rules, rendered in canonical order) becomes the run authority block. `fractal validate` checks the KDL and that each bundle still has its pinned sha256; `fractal apply` publishes the mission and carries the bundle to the daemon. `fixtures/land-program.kdl` is the landing program's rendering; after an intended change to the program or to code its bundle inlines, `nix develop .#web -c env FRACTAL_UPDATE_GOLDENS=1 pnpm --filter fractal test` rewrites it. Use the web shell's pinned Bun, as hosted TypeScript CI does: the default Rust shell uses a separate Nixpkgs input and can produce different bundle bytes.

A loader that walks its own tree renders one program mission the same way with `programMission({ root, file, definition, bundleDir, bun? })`: `file` is a `mission.ts` below the tree `root`, `definition` its default export, and the subject ID its real directory below `root`'s real path. The result is `{ kdl, bundle, sha256 }`, the bytes `loadSubjectTree` would render for that subject.

## Checks

```sh
nix develop .#web -c bash -c 'pnpm install --frozen-lockfile && pnpm --filter fractal typecheck && pnpm --filter fractal test'
cargo test -p fractal --test validate
```
