# Host-local resource admission

This document specifies finite execution resource lanes. It extends the existing
[seat lane](seat-lanes.md) boundary and refines [program-engine D5](program-engine.md#7-serialized-sections).
Ordered [merge lanes](lanes.md), workflow population limits, and resource admission remain distinct.

## Ownership

```text
Effect scope / data DSL exec step / shell exec
             |
             v
execution-host writer: holds + class/backfill/aging + shared fit + fenced membership
             |
             v
payload gate -> execution generation -> whole-tree retirement proof
```

Effect owns branching, concurrency, retries and scoped finalizers. The execution host owns one
serialized admission decision, the admitted generation, process placement, and retirement. The OS
owns enforcement. Graph observations describe those facts; a replicated grant-shaped claim cannot
launch a process. No standing lane worker or replicated graph semaphore grants resource capacity.

A named lane policy binds an execution host, `maxMembers`, a workload profile key (or historical
raw CPU/memory declaration), and required coverage. There is no implicit finite-execution
footprint: an unresolved declaration without complete raw values is refused. The host refuses a
conflicting policy for an existing name. Every request has a class: `landing`, `check` or `agent`
(the default). Classes are served in that order. Landing is strict FIFO, without backfill.
Within the other classes, FIFO is preferred, but a fitting later ticket may backfill a head
that does not fit until that head has waited 15 minutes. A lower class never bypasses a
higher-class head. Membership binds the lane, authority epoch, owner run/step/attempt/invocation, and execution
generation. Independent roots consume room once in the accountant shared with seats and their
raises/bursts, including CI and shell work. A child run without an OS process reserves no memory.

An earlier profile-less installed policy can acquire its first explicit profile while preserving
the same member limit and coverage. Already granted profile-less custody remains valid through
retirement; profiled admission on that lane waits for those holders to drain. This transition
does not permit changing an already explicit profile or silently rewriting a request.

A durable **host hold** closes admission without occupying a membership. Its authenticated owner,
revision, reason, affected operations and receipt persist independently of the initiating run.
Resolving an ask, terminating a run, or restarting a daemon does not clear a hold. Resolution checks
its owner and expected revision and wakes dependent admissions.

## Workload profile keys

The execution host's `finite_resources.profiles` map owns reusable workload seeds, for example:

```json
{"profiles":{"check.unit":{"cores":4,"memory":8589934592,"disk":16106127360,"io":"heavy"}}}
```

The same key is exposed as CLI `--profile`, Effect `profile`, and KDL `profile`. It is distinct
from the physical lane name and priority class. A known key supplies CPU, charged/enforced memory
maximum, optional disk bytes and optional IO class; callers cannot override those values. The
host resolves before fit, freezes the estimate in the durable request, and reuses that snapshot
on replay and nested execution rather than looking up a newer seed. Lane identity retains the
key, not mutable seed revisions. Clients cannot supply `profile_estimate`.

Disk location is separate: CLI `--disk-path` defaults to the caller's working directory for a
keyed root; Effect `diskPath` and KDL `disk-path` must be explicit when the seed reserves disk.
Until the raw-caller cutover, no-key raw declarations remain supported, and an unknown key
requires a complete explicit raw declaration. There are no fleet-host defaults or client lookups.
Seed estimates expose their seed revision, declared source/reason, zero memory/disk samples and
no learned revision. Optional seed `memoryHigh` is copied as `memory_high_bytes`; without it no
MemoryHigh is synthesized. Charged/enforced memory maximum remains the seed's `memory`. Learned
adjustments are not activated here.
Linux keeps the named parent lane's aggregate envelope separate from each execution generation's
child slice. CPU/memory limits (and any explicit MemoryHigh) apply per generation; aggregate
bounds never shrink below live launch, admitted or retiring snapshots. Both levels enable IO
accounting, and enforced launch verifies the actual parent/child controls before payload release.

### Host-owned profiles with Landing

A named host profile and Landing authorization compose without another capacity resolver.
For a host whose `finite_resources.profiles` contains `land-check-native`, a program declares:

```ts
const lane = yield* Fractal.resources.lane('factory-landing', {
  host: 'worker',
  profile: 'land-check-native',
  maxMembers: 1,
  class: 'landing',
  diskPath: '/workspace',
  coverage: 'enforced',
})
```

The equivalent direct DSL lane is:

```kdl
lane "factory-landing" host="worker" profile="land-check-native" max-members=1 class="landing" disk-path="/workspace" coverage="enforced"
```

Both paths use the same host-owned profile resolver as shell execution. Do not duplicate seed
amounts in the caller or supply capacity, disk-byte or IO overrides for a known profile: those
overrides are refused. `diskPath` selects the filesystem, not a disk-byte override. The resolved
estimate is frozen once in the durable request and retained through replay/restart, even if the
host seed changes.

The profile supplies the footprint; it does not grant Landing authority or add another memory/IO
budget. Admission charges the existing host totals and applies the protected Landing carve-out
described below. The owning run's original authenticated initiating principal must be authorized
by `finite_resources.landing.allowedActors`. An unauthorized root is visibly refused as
`unauthorized-landing`, never downgraded to another class.
[Linked children](program-engine.md#6-linked-child-runs-worker-turns-and-asks), including children
published with `Fractal.start`, inherit that principal and request their own DSL/program lane.
A handle, requester label, `Fractal.start` reporting destination (`reportTo`) or parent lane is not
a substitute for that authorization. `reportTo` is routing only: it never sets
`initiating_principal` or authorizes Landing. It accepts an exact `agent/...` subject and replaces
the mission's destination for failure, cancellation and stall reporting, without implying
`--report-completed` or changing the mission's `stalled-after` (default: 30 minutes). Omission
retains the mission's reporting default; a different explicit `reportTo` conflicts with the
existing child publication identity.


## Shell callers

```sh
fractal resources exec --lane verification --profile check.unit --class check \
  --max-members 1 --disk-path /workspace --receipt-file outcome.json \
  --admission-timeout 4h --timeout 2h -- run-verification
```

The CLI uses the local daemon's internal resource path and the same finite membership machinery
as programs and DSL steps. It does not require a client-v0 route. Memory and disk accept integer
byte counts, exact unsigned e/E notation (`2e9` equals `2000000000`), or binary suffixes such as
`24G`. Signed, fractional, malformed and overflowing byte strings are typed parse errors; no
floating-point rounding is used. Admission and payload timeouts are separate budgets.
Waiting, admitted and refused diagnostics go to stderr with `blockedBy`; payload stdout is not
mixed with admission diagnostics, and the payload's exit status is preserved.
`--lane` remains required except with `--inherit-parent`; `--max-members` is a positive integer
and defaults to one. Raw `--cores` and `--memory` are optional only as a pair. Existing no-key callers can continue using
that pair together with `--disk BYTES --disk-path PATH` and `--io heavy`.

`--receipt-file PATH` is root-only: inherited execution, `--inherit-parent` and `--require-parent`
are refused before launch with `nested-receipt-unsupported`. It atomically replaces PATH only
after a terminal root outcome and confirmed retirement (or confirmed nonlaunch). Its body uses
the exact canonical A.3 receipt fields, including head/clean observations, coverage, class,
owner, profile and memory fields, plus `schema_version: 1` and nullable `receipt_subject`.
It includes root `exec_id`, host/state, nullable exit code/signal, `launched`, queue/wall timing,
actual root `argv` and `argv_sha256`. The digest is lowercase SHA-256 of UTF-8 arguments joined
by NUL without a trailing NUL; NUL-containing arguments are refused. Nested proof joins the
eventual outer root receipt instead of fabricating a terminal root outcome from a child's exit.
`FRACTAL_RESOURCE_EXEC_ID` joins the outer root receipt; `FRACTAL_RESOURCE_MEMBER_EXEC_ID`
identifies the current execution generation. Neither value is authorization.
Shell, direct DSL and program payloads receive the same host-derived
`FRACTAL_RESOURCE_PARENT`, `FRACTAL_RESOURCE_EXEC_ID` and
`FRACTAL_RESOURCE_MEMBER_EXEC_ID` markers before release, overwriting caller-provided values.
Direct roots use their runtime ID for both execution selectors; nested shell members retain the
outer shell root selector. Endpoints reconnect to the admitting authority. These native markers
support inherited verification; nested `resources exec` submission still requires a shell parent
and is not supported under a direct DSL/program parent.
Receipts contain actual root argv, so never put credentials in argv. Environment values are sent
only in the live launch request and are neither receipt fields nor persisted command data.
After restart, a command whose live environment is unavailable fails closed rather than replaying
lost secrets.
Use optional `--request-id UUID` to recover a transport-uncertain submission with an owner-scoped
identity. UUID syntax is validated before submission. The first accepted intent owns that ID:
identical semantic retries return the existing execution without relaunching or replacing its
live environment; changed intent is refused. Environment values never enter the persisted
submission-intent digest. Request IDs also work for inherited child executions.
An identical inherited retry adopts its existing execution even after parent retirement;
live parent custody is required only when accepting a new child, never for replay adoption.

`show --request-id UUID` returns `{"execution":null}` when absent.
`cancel --request-id UUID --wait` returns `{requestId,cancelled:true,tombstone,execution}` after
confirmed retirement or a durable cancellation tombstone. Tombstones fence late submissions
for seven days; never reuse expired identities. Use `--json` for machine-readable responses.

`exec`, `show`, `cancel` and `member-scope` accept `--as ACTOR` after the subcommand. Agent
selectors use the authenticated Unix agent transport; local operator selectors such as
`person/operator` use `unix_as`, not an agent identity.


`fractal resources cancel EXECUTION_ID` uses the same authenticated execution-owner boundary and
waits for confirmed retirement or nonlaunch, then prints the terminal execution (`--json` for
machine-readable output). It does not infer process ownership from cgroup names or environment
labels. `show EXECUTION_ID` exposes the terminal receipt subject.

### Offline membership decision

`fractal --json resources in-member --owner-uid UID` reads only the local kernel membership
through the runtime's shared lane grammar. It performs no config discovery, API request,
daemon connection or authorization. Its fixed JSON is
`{"inside":false,"laneSlice":null,"binding":null}` outside a recognized member, or
`{"inside":true,"laneSlice":"CANONICAL_PARENT_SLICE","binding":"BINDING_HASH"}` inside a matching
member hierarchy (including descendants). The original raw lane name is not recoverable from
the slice identity. `--owner-uid` selects the expected user hierarchy, not an identity grant.

Both normal inside and outside decisions exit zero. An unreadable kernel membership or
unsupported OS is a genuine nonzero error; wrappers must fail closed, not reinterpret it as
outside and start another root. This is only a local nesting decision and never proves live
host custody, authorizes a privileged helper, or replaces the authenticated exact member-scope
check below. No custody layout or configurable cgroup root is exposed.

### Exact member verification for privileged helpers

`fractal --json resources member-scope --exec-id EXECUTION_ID` reports the runtime-owned DTO:
`{execId, rootExecId, requestId, lane, class, profile, generation, state: "running", cgroupPath}`. The authenticated API
`GET /v1/internal/resources/{id}/member-scope` resolves shell, direct DSL and program roots
from existing membership custody. Bound callers authenticate through their exact run credential
(run, step and attempt) or native agent invocation; local-operator inspection remains unchanged.
It checks the owner, host, released root, live grant, enforced coverage and exact runtime
generation, then verifies that target generation's kernel cgroup placement. It does not prove
the HTTP querying process is inside that cgroup, and never authenticates from environment markers.
Direct roots have no shell request UUID (`requestId` is null); the response shape is unchanged.
Parent slices, sibling generations, ended/retired roots, retiring memberships and logical-only
execution are not proof of a live enforced member. Missing or ambiguous custody refuses with
`resource-member-scope-refused`; foreign ownership remains `resource-exec-owner`.

A root helper must independently read its own unified `/proc/self/cgroup` entry and require
**exact equality** with the reported `cgroupPath` for `FRACTAL_RESOURCE_MEMBER_EXEC_ID`; prefix matching
or a parent/sibling match is insufficient. The environment ID is only a lookup key, never
authority. Reject absent, stale, retired or mismatched execution evidence before any mutation.

Launch the resource payload as the user, with only `sudo /exact/pinned/helper` inside it. Each
privileged helper needs a security review and narrowly pinned sudoers permission for no arguments
or a closed argument enum. Do not grant root shells, `systemd-run`, arbitrary manager commands,
or another escape from the admitted user member. This inspection API grants no sudo permission.

Linux defaults to enforced coverage: a transient scope beneath `app-lanes.slice` with memory and
CPU limits. macOS refuses `resources exec` before launch with `unsupported-platform` because it
has no whole-tree retirement proof; `ls`, `show`, `hold` and `release` remain available.
A nested resource exec validates and inherits its parent's membership through an environment
marker instead of reserving a second footprint; custody lasts until every execution root retires.
Disk accounting uses declared reservations, not measured disk peaks; memory peaks come from cgroup
`memory.peak` on enforced hosts and are absent on logical-only hosts.

`fractal resources ls` lists host holders and waiters; `fractal resources show` exposes membership
details and blocking reasons. `fractal resources hold` and `fractal resources release` operate on
the existing durable host holds, including authenticated ownership and revision checks.
Use `ls --host worker --class check` to narrow inventory, and `show EXECUTION_ID` to inspect one
execution. A hold's affected operation is the lane name, or `*` for all lanes:

```sh
fractal resources hold worker --operation verification --reason 'maintenance'
fractal resources release worker HOLD_ID --expected-revision 1 --resolution 'maintenance complete'
```

Ownership comes from the authenticated run/attempt or native agent invocation, not mutable
environment labels. An ad-hoc local operator has an explicit local/person owner. Nested
membership inheritance requires that same authenticated owner; inventory never exports grants.
Use `--require-parent` for nested-only checks: without an inherited membership marker the CLI
refuses instead of creating a root; the daemon still authenticates the marker's grant, owner and
active parent state. `--inherit-parent` implies that requirement and uses the parent's frozen
lane, profile and resource snapshot, allowing `--lane` to be omitted. Contradictory explicit
lane, profile or class values are refused. `FRACTAL_RESOURCE_EXEC_ID` alone never satisfies the
parent requirement. Nested checks inherit existing admission, so omit `--admission-timeout`
rather than using a zero budget.


Shell payload environment values exist only in the submitting daemon's memory, keyed by execution
ID, until launch (or terminal refusal/cancellation). Neither names nor values are retained in the
execution record, inspection responses, or receipts; `command.environment` is always empty there.
A daemon restart loses pending launch environments, including explicitly empty ones. An execution
that has not launched refuses with `environment-unavailable` when it reaches launch; resubmit it
with its live environment. Existing native generations keep their execution facts and custody and
are observed/retired without relaunching or substituting the daemon's environment.
The runtime's durable launch-operation identity is passed separately as typed control-plane
metadata. Payload keys such as `ST3_START_OPERATION` are never parsed into generation receipts,
start-rejection evidence, producer custody, or runtime operation tracing.

On first upgraded store open, historical shell command environments are cleared in place while
execution history, outcomes, receipts, and membership custody remain readable. The writer enables
SQLite secure deletion, truncates the WAL, rewrites the database with `VACUUM`, and truncates the
WAL again before recording completion. Busy checkpoints or rewrite errors fail store open; stop
competing readers/writers and retry the open. This scrubs the active database and WAL, not external
backups, snapshots, or payload-produced output. Do not downgrade to a writer that persists shell
environments. Secrets deliberately printed by payloads or supplied as argv remain the caller's
responsibility.


## Program API

```ts
import { Effect } from 'effect'
import { Fractal } from 'fractal/effect'

const verify = Effect.scoped(Effect.gen(function* () {
  const lane = yield* Fractal.resources.lane('verification', {
    host: 'worker',
    maxMembers: 1,
    profile: 'check.unit',
    class: 'check',
    diskPath: '/workspace',
    coverage: 'enforced',
    admissionTimeout: '30m',
  })
  return yield* lane.exec('verify', ['run-verification'], {
    cwd: '/workspace/project',
    timeout: '20m',
  })
}))
```

`maxMembers` defaults to one and `coverage` to `logical`. A known `profile` resolves on the host;
raw `capacity` remains supported during caller cutover, with memory in bytes. `admissionTimeout` is an optional duration. The acquired handle exposes
`lane.exec(id, argv, options)` and `lane.run(id, mission, input)`. The latter starts a linked child
run whose actual exec roots consume the same native membership and fencing context. Arbitrary
Effect code inside the scope does not move its processes into the lane or fence external writes.

Acquisition crosses `resource.lane.acquire` on every evaluation, including replay; it is not an
ordinary replayed Activity result. The host reattaches/validates the exact request and generation.
Inside a durable `Fractal.step` activity, acquisition request keys are scoped by that activity's
durable identity, independently of the physical lane policy name. Skipping a completed activity
on replay cannot renumber another activity's membership, and distinct concurrent activities
request separate memberships of the same lane, contending at the host admission boundary.
Successive acquisitions in one durable scope retain occurrence keys; sharing the same acquisition
identity across fibers within that scope remains a duplicate-ID defect.
Execution uses `resource.lane.exec` or `resource.lane.run`. Scope finalization sends
`resource.lane.release` and awaits retirement; JavaScript interruption is not physical release.
Expected native failures use `ResourceError` with a distinct `code`, separate from payload exit
status and linked-child failure.

## Data DSL

```kdl
step "verify" timeout="20m" {
  agentless
  lane "verification" host="worker" profile="check.unit" max-members=1 class="check" disk-path="/workspace" coverage="enforced" admission-timeout="30m"
  exec "verify" {
    host "worker"
    command "run-verification"
  }
}
```

The step must contain exactly one exec declaration, and its actual host must match the lane host.
The name and explicit host are required. A known `profile` replaces the raw nested `capacity`
block; unknown keys require complete raw values. Other defaults and options match the program
API: `disk-path` maps to `diskPath`, `max-members` to `maxMembers`, `admission-timeout` to
`admissionTimeout`, and lane-child `capacity` to the independent root's footprint. Numeric bounds for a shared containment slice
accommodate its configured members; independent root reservations still add in host fit.

The DSL lowers to the same native request, payload gate and retirement operations as the program
handle, not to a second scheduler. Different missions naming the same host/lane contend at that
same boundary. The step waits ready during admission and opens its payload budget on confirmed
release. Failed admission reports a distinct admission reason rather than executing the payload.

An existing step's direct `capacity` continues to raise its assigned agent's seat lane. Agentless
steps cannot declare that direct raise. Nesting `capacity` inside the new explicit resource `lane`
does not reinterpret old capacity declarations. Omitting the new lane leaves published step JSON
and definition hashes unchanged; old history remains readable. A physical independent exec root
is not a seat descendant merely because environment variables name that seat. Logical coverage
still requires an explicit profile and reserves its declared footprint alongside seat raises.

New lane-bearing revisions use the existing fleet reader-capability barrier:
`mission_resource_lane>=1` is required for ordinary lanes and `>=2` for non-agent classes, disk
reservations or IO-heavy requests. Each current member advertises its version on its own
`daemon.started`. Planning and direct publication report `mission-resource-lane-unavailable` while
any reader is older. The shell CLI requires version 2 from its local daemon before submission.
Plain revisions and already published history are unaffected.
Keyed declarations and keyed program support additionally require `resource_profile_keys>=1`;
the CLI checks that capability for keyed calls, receipt files and nondefault member limits.


## Admission, clocks and cancellation

```text
queued -> admitted -> prepared -> payload released -> execution ended -> retired
   |          |            |              |                 |
withdraw   cancel       uncertain      cancel tree       descendants remain
```

The host evaluates holds, class priority, logical occupancy and physical fit in its writer
transaction. Landing is strict FIFO with no same-class backfill. Within check or agent,
before the head has waited 15 minutes a later ticket may pass only when the earlier ticket does not fit and the later one
fits all current budgets: ARC-aware memory, mount disk floors and reservations, and heavy-IO
tokens. Fresh fit observations, not a previous blocked reason, decide backfill. Lower classes
cannot pass a higher-class head.

At 15 minutes the waiting head reserves the capacity it needs by closing admission to all
later tickets in its class and lower classes until it is admitted. This fence is not another
memory, disk or IO charge: existing holders retain their reservations, and the head is charged
exactly once when admitted. Withdrawal or terminal refusal removes the fence; cancellation
wakes the next waiter. Requests exceeding host lane room are refused as `impossible`, even
with zero holders, rather than retaining an unfulfillable reservation.

Waiters register a host/lane wake dependency before rechecking. Release, hold resolution,
preparation cleanup and authority changes wake affected work. The 15-minute aging deadline
is a scheduled one-shot wake, combined with any earlier admission deadline, not a poll loop;
once aged there is no repeating aging timer. Host-memory and disk observations invalidate fit
when usage changes, never grant capacity on elapsed time alone. Seat starts, scoped raises and
finite executions share the native host accountant rather than independent CI or shell budgets.

Seat starts, bursts and finite lanes share one ARC-aware host accountant:
`eligible_arc = min(max(0, arc.size - arc.c_min), evictable_arc)` (zero without ZFS),
where `evictable_arc` sums `mru_evictable_data`, `mru_evictable_metadata`,
`mfu_evictable_data` and `mfu_evictable_metadata`. `reclaimable_arc = eligible_arc` only
while memory PSI `full avg60 < 1%`; otherwise credit is zero. Missing or malformed ARC/PSI
observations give zero credit, never an optimistic estimate.
`unreclaimable_used = max(0, MemTotal - MemAvailable - reclaimable_arc)`, and
`outside = max(0, unreclaimable_used - live_lane_usage)`. Live lane usage is the hierarchical
`anon + kernel` counter of `app-lanes.slice`, counted once for seats and finite members.
System services and other unlaned consumers therefore count without double charging lane members.
Free memory is host lane room minus declared held footprints, outside, and the kept burst reserve.
Finite lanes keep a burst reserve only while a live laned seat could ask for a burst; a finite
request is not itself a burst-capable seat.

This deliberately credits less than size minus minimum: dirty and in-use ARC buffers cannot be
freed immediately. In one measured host shape, ARC above minimum was 27.6 GiB but only 7.3 GiB
was evictable; the bound is 7.3 GiB, not 27.6 GiB. PSI suppresses even that credit while reclaim
lags. The observation remains live: the next lower-pressure memory observation re-evaluates fit.

A footprint exceeding host lane room is refused with code `impossible`, never left in FIFO.
With no admitted members, granted bursts or host hold, the head fits exactly when its footprint
fits room minus outside (and any reserve required by live seats). Waits expose `blockedBy` as
`holder`, `hold`, `outside`, `psi`, `reserve`, `fifo`, `disk` or `io`; outside/PSI reasons include
measured bytes. Resource-lane phase telemetry carries the same attribution. Queue and occupancy
gauges expose `st.resource.host` and `st.resource.lane`, including zero-holder series, so a
sustained nonempty queue without holders is observable from the first deployment.

Optional disk requests reserve their declared peak on the filesystem containing `disk.path`.
Admission requires available bytes above the host's per-mount floor after outstanding peak
reservations; a reservation remains charged until confirmed retirement, not merely payload exit.
`io: 'heavy'` additionally consumes a host-policy slot. Host configuration `finite_resources`
contains `diskFloorBytes` (default 2 GiB), `diskFloors` (path-to-floor overrides, resolved by
filesystem), and `heavyIoSlots` (default one; explicit `null` marks an unconstrained host).
Multiple floor overrides for the same filesystem use the largest floor. These are host policy
values, not caller semaphores or independent CI limits.

`finite_resources.landing` is the sole host policy for landing admission:

- `allowedActors`: exact actor subjects permitted to request class `landing` (default `[]`).
- `memoryBytes`: bytes carved out of existing host lane room (default `0`).
- `heavyIoSlots`: slots carved out of the existing `heavyIoSlots` total (default `0`).

For example, with total `heavyIoSlots = 2`, a landing reservation of `heavyIoSlots = 1`
leaves at most one heavy-IO slot for ordinary work; it does not create a third slot.
Ordinary finite work and seat starts/raises exclude unused landing memory. Landing custody
consumes the reservation first, so it is not charged twice. Host holds, outside-memory
pressure, disk floors, total room and total IO limits still apply to landing.
The reservation is minimum protected headroom, not a ceiling: landing may also consume
ordinary room. An authorized landing backlog can starve check/agent work by design.

The host grant boundary authorizes every request path (shell, program and DSL) from native
invocation ownership, the owning mission run's immutable `initiating_principal` in its
canonical `mission-run.created` receipt, or authenticated local-operator ownership.
The receipt records the serving boundary's authenticated principal, never `--as`, a claimed
actor or `requester` attribution. A BoundRun creator resolves to its own recorded initiating
principal. System subscription/linked/scheduled children and supersession inherit their
parent's original principal; a retry requested by another authenticated principal records
that principal instead. Historical receipts with no principal remain unauthorized.
An unconfigured host refuses landing visibly with `unauthorized-landing`; it never
downgrades the class. Inspection and terminal receipts retain `class: landing`, and shell
refusals carry `refusalCode` (`refusal_code` and `refusal_reason` in the durable receipt).
`declared_priority` remains declaration metadata, not a second admission-class control.

This is a same-UID cooperative control, not a cryptographic actor boundary: actor identity is
asserted through the daemon's existing caller binding and native ownership records. A process
that can rewrite the user's daemon state/configuration is outside this threat model.
Deploy a daemon supporting these keys before adding them to host configuration; unknown host
policy fields are rejected rather than ignored.
After rollout, restart a historical factory intake root once from the Factory Manager seat
through native `fractal missions start`; pre-roll roots cannot lend landing authority.
Legacy client-v0 `mission.start` has no authenticated write principal and creates roots without
one, even when its session supplies an allowlisted `authority_actor`. Such roots cannot use landing.

Agentless work requests its landing lane **on the step itself** (a DSL exec lane or program
resource), authorized by that run's recorded initiating principal. Native exec caller identity
is unchanged here: a new run started by an anonymous agentless native exec records no
initiating principal, even when `--as` or `requester` names an allowlisted actor, and cannot use
landing. Acquiring a parent lane does not confer Landing authority on an anonymously started run.

The host persists `queuedAt`, `admittedAt`, `payloadReleasedAt`, `executionEndedAt` and `retiredAt`.
Admission/preparation, payload execution and retirement are separate intervals. `admissionTimeout`
does not consume the payload's `timeout`; retirement grace does not extend execution. The enclosing
mission deadline bounds the entire scope. In-boot budgets use host monotonic elapsed time;
restart recovery does not grant a fresh execution budget and exposes clock uncertainty.

Queued cancellation withdraws the exact request. A racing grant either cannot pass the payload
gate or is cancelled as that exact generation. A launched root retains custody until its whole
tree is proven retired. Natural exit, terminal journal status, renewal expiry, or a missing client
socket is not evidence of quiescence. Release also cancels linked children and waits for their
terminal ownership state before freeing the membership. Unknown preparation, failed teardown,
old-authority survivors and unknown remote effects retain custody and block conflicting work.
Positive nonlaunch evidence can remove an exact rejected preparation; absence of a record cannot.

## Durable execution receipts

Terminal top-level shell resource executions (`parent` absent) and direct DSL/program resource
roots record the existing immutable `resource.exec-receipt` claim in the replicated claims log.
Shell receipts are committed atomically with the terminal execution update; nested shell executions
do not append another root receipt. Direct DSL/program receipts bind the exact owner step run,
attempt and runtime generation and are emitted only after confirmed whole-tree retirement.
The claim is system-only, once per resource subject, with durable retention. It observes execution,
not admission authority; a receipt kind or terminal state alone is not retirement proof.
`state` is `completed`, `refused` or `cancelled`; shell semantics retain terminal refusal or
cancellation before payload launch.

Fields carry `exec_id`, executing `host`, `class` (`landing`, `check`, `agent`), `owner` and
`coverage` (`logical`, `enforced`). Declared profile fields are optional `memory_bytes`,
`disk_reserved_bytes` (the declared reservation, zero when none was requested), and optional
`io` (`heavy`). `wall_duration_ms` and `queue_wait_ms` are elapsed durations.
Optional observations are `head_start`, `head_end`, `clean_start`, `clean_end`, `exit_code`,
`exit_signal` and `memory_peak_bytes`. The memory peak is measured usage, not the declared
memory limit; disk reservation is not measured disk usage. Missing observations remain absent,
not invented zeroes, false clean flags, empty heads or successful exit codes. In particular,
pre-launch refusal/cancellation may have no exit status or worktree observations, and logical
coverage does not imply a measured memory peak.

Subjects use this layout, where `hex` is lowercase hexadecimal encoding of the exact UTF-8 bytes:

```text
resource/exec/{hex(head_start) or _}/{hex(host)}/{class}/{hex(exec_id)}
```

The head comes first so the existing claims subject-prefix query can find a head across all
hosts and classes. An unobserved start head uses the reserved `_` segment (`resource/exec/_/`),
which is not a hexadecimal revision. For a known head, `resource/exec/{hex(head)}/` selects every receipt for that
head; appending `{hex(host)}/` and then `{class}/` narrows the same prefix by host and class.

For example, read a page for a head and select a host/class using the ordinary claims response:

```sh
head_hex=$(printf '%s' "$head" | od -An -v -tx1 | tr -d ' \n')
fractal --json claims ls --kind resource.exec-receipt \
  --subject-prefix "resource/exec/$head_hex/" --limit 500 |
  jq --arg host "$host" --arg class "$class" \
    '.claims[] | select(.body.fields.host == $host and .body.fields.class == $class)'
```

To select a host/class across all heads, omit `--subject-prefix` and use the same field filter.
There is no independent indexed host/class filter in `claims ls`. These examples select only
one page: continue an ascending query with `--after-index` set to the response's `next_cursor`
until it is absent. Filter every page, including pages with no matching host/class rows.

### Binding a direct proof to its retired execution

Direct DSL/program roots use this same receipt schema and authority, not a separate proof record.
Before payload release, the durable runtime-generation launch snapshot freezes the payload's
actual `argv`, excluding credential and daemon wrappers. `argv_sha256` uses the shared shell
convention: lowercase SHA-256 of UTF-8 arguments joined by NUL, with no trailing NUL.
At confirmed retirement, receipt emission combines that immutable command binding with the
same runtime generation's exit code/signal and the membership's exact owner, class and profile.
Direct-root receipts have `state: retired`; `runtime_generation` and `retired_at` bind the
physical attempt. Subjects are
`resource/exec/{hex(head_start) or _}/root/{hex(host)}/{hex(runtime_id)}/{hex(generation)}`.
Repository observations are optional recorded facts, not assertions inferred from desired state.
Nonzero and signal outcomes remain truthful; terminal denial never fabricates a launched receipt.

Use the indexed exact receipt lookup with the proof's **step-run ID and attempt**, additionally
selecting a runtime generation when the step legitimately has multiple roots. The read returns the
authoritative receipt reference and its bound fields; it does not mint a grant. This is distinct
from browsing head-prefix claims above: a latest-success or latest-desired-declaration join cannot
identify a proof. `CommandResult` has no exec-ID field and none is needed for this lookup.

The exact read is
`GET /v1/internal/resources/exec-receipt?stepRun=<proof-step>&attempt=N`,
with optional `&generation=...`. The corresponding CLI is:

```sh
fractal --json resources exec-receipt --step-run "$proof_step" --attempt "$proof_attempt"
# Add --generation "$proof_generation" to select one runtime generation explicitly.
```

The typed store read is `Store::resource_exec_receipt(&ReceiptQuery) -> ReceiptReference`
in `store::resource_exec_receipts`. The response includes `claimId`, `subject`, `origin`,
and typed bound `fields`, never grants or a new credential. Missing/ineligible bindings,
ambiguity and stale generations return typed `resource-receipt-*` errors.
Run-bound callers may read proof steps of their own run.

A light finalizer runs after the program resource scope closes (scope release awaits retirement),
or after the DSL proof's retirement boundary. Pass the proof's step-run ID and attempt explicitly;
the finalizer's own `ST_STEP_RUN` is not the proof's identity. Require the exact host, attempt and
runtime generation, confirmed retirement, expected argv digest, expected head/clean observations,
class/profile and exit outcome before accepting evidence.

Ambiguous roots, missing launch snapshots or outcomes, unconfirmed retirement, stale generations
and wrong host/attempt/head/command visibly refuse. Historical launches without a frozen command
binding remain ineligible: neither a changed desired declaration nor daemon restart may relabel
old evidence. Do not fall back to a previous successful attempt or fill missing fields from current
configuration. Logical coverage never fabricates measured cgroup peaks. Existing program/DSL
retirement accounting and shell receipt semantics remain unchanged.


## Coverage and boundaries

Linux uses systemd user scopes in named slices with process placement/readback before the payload
gate. Required unsupported enforcement refuses rather than detaching. CPU quota is a ceiling,
not reserved cores; memory maximum is a hard accounting bound, not proof that memory is available.
No implicit `memory.high`, swap or IO policy is added to existing seats.

Scope teardown, retirement proof, lane removal and legacy start closure wait for the systemd
observer's next admitted observation after a local control invalidates its snapshot. The
two-second observation bound still applies: waiting releases the observer lock and owns no
subprocess. This is admission synchronization, not a retry of a failed manager command.
Transport failures and manager backoff remain prompt errors, and stale or incomplete observations
never prove that a scope has retired.

If a listed unit is collected before its properties are read, a complete `not-found` property
record is retained as its current state, rather than marking the manager unresponsive. Scope
absence still requires an inactive unit with no cgroup and a snapshot newer than its completed
join. PTY launcher acknowledgement and observed pre-migration process membership complete that
join boundary before the old scope can disappear.

Darwin offers host-local logical exclusivity and scoped cancellation, not Linux-equivalent
whole-tree CPU/memory enforcement. Required `enforced` coverage returns `UnsupportedCoverage`.
Logical-only execution does not imply an enforced resource cap. Detached process groups cannot
prove quiescence of escaped descendants; uncertain retirement retains custody instead of promising
safe successor execution. Arbitrary shell SSH is not a remote target mutation fence.

## FACTORY integration boundary

Factory publication remains a linked child workflow with its own journal. `concurrent-runs max=1`
limits that mission's population; it does not serialize different missions on one executor. A
native capacity-one executor lane supplies that physical exclusivity beneath the workflow.

Executor reservation files/claims and periodic reserve probes can be removed only after every
affected launcher has cut over to verified native admission and retirement. Gate-slot remains the
external owner until the corresponding workload/host is explicitly cut over; do not nest its
admission with a second reservation for the same footprint. This API is not permission to activate
the separate native jobs stack.

**The target-host activation adapter is not implemented by this boundary.** Retain the current
target hold bridge until the mutation endpoint checks the target authority, hold revision and exact
activation generation at the actual target mutation boundary. An executor grant does not authorize
a remote mutation. Refusal at that boundary must release executor custody rather than park while
holding it. Failed activation quarantine belongs to an explicit target host hold; terminal run
status neither clears that hold nor proves executor retirement. Admission and process exit also
do not replace signed check evidence, checked-tree trust, or merge policy.
