# Host-local resource admission

This document specifies finite execution resource lanes. It extends the existing
[seat lane](seat-lanes.md) boundary and refines [program-engine D5](program-engine.md#7-serialized-sections).
Ordered [merge lanes](lanes.md), workflow population limits, and resource admission remain distinct.

## Ownership

```text
Effect scope / data DSL exec step
             |
             v
execution-host writer: holds + FIFO + shared fit + fenced membership
             |
             v
payload gate -> execution generation -> whole-tree retirement proof
```

Effect owns branching, concurrency, retries and scoped finalizers. The execution host owns one
serialized admission decision, the admitted generation, process placement, and retirement. The OS
owns enforcement. Graph observations describe those facts; a replicated grant-shaped claim cannot
launch a process. No standing lane worker or replicated graph semaphore grants resource capacity.

A named lane policy binds an execution host, `maxMembers`, optional CPU/memory footprint, and
required coverage. The host refuses a conflicting policy for an existing name. Membership binds
the lane, authority epoch, owner run/step/attempt/invocation, and execution generation. Independent
roots consume room once in the host accountant shared with seats and their raises/bursts. A child
run without an OS process does not reserve fictitious memory.

A durable **host hold** closes admission without occupying a membership. Its authenticated owner,
revision, reason, affected operations and receipt persist independently of the initiating run.
Resolving an ask, terminating a run, or restarting a daemon does not clear a hold. Resolution checks
its owner and expected revision and wakes dependent admissions.

## Program API

```ts
import { Effect } from 'effect'
import { Fractal } from 'fractal/effect'

const verify = Effect.scoped(Effect.gen(function* () {
  const lane = yield* Fractal.resources.lane('verification', {
    host: 'worker',
    maxMembers: 1,
    capacity: { cores: 4, memory: 8 * 1024 ** 3 },
    coverage: 'enforced',
    admissionTimeout: '30m',
  })
  return yield* lane.exec('verify', ['run-verification'], {
    cwd: '/workspace/project',
    timeout: '20m',
  })
}))
```

`maxMembers` defaults to one and `coverage` to `logical`. Capacity is optional; memory is bytes in
the program API. `admissionTimeout` is an optional duration. The acquired handle exposes
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
  lane "verification" host="worker" max-members=1 coverage="enforced" admission-timeout="30m" {
    capacity cores=4 memory="8G"
  }
  exec "verify" {
    host "worker"
    command "run-verification"
  }
}
```

The step must contain exactly one exec declaration, and its actual host must match the lane host.
The name and explicit host are required. The remaining defaults and options match the program API:
`max-members` maps to `maxMembers`, `admission-timeout` to `admissionTimeout`, and lane-child
`capacity` to the independent root's footprint. Numeric bounds for a shared containment slice
accommodate its configured members; independent root reservations still add in host fit.

The DSL lowers to the same native request, payload gate and retirement operations as the program
handle, not to a second scheduler. Different missions naming the same host/lane contend at that
same boundary. The step waits ready during admission and opens its payload budget on confirmed
release. Failed admission reports a distinct admission reason rather than executing the payload.

An existing step's direct `capacity` continues to raise its assigned agent's seat lane. Agentless
steps cannot declare that direct raise. Nesting `capacity` inside the new explicit resource `lane`
does not reinterpret old capacity declarations. Omitting the new lane leaves published step JSON
and definition hashes unchanged; old history remains readable. A physical independent exec root
is not a seat descendant merely because environment variables name that seat. Logical exclusivity
without a new footprint can coexist with a seat's existing raise.

New lane-bearing revisions are held by the existing fleet reader-capability barrier until every
current member advertises `mission_resource_lane=1` on its own `daemon.started`. Planning and direct
publication report `mission-resource-lane-unavailable` while any reader is older; capacity/program
support alone does not advertise resource-lane support. Plain revisions and already published
history are unaffected.

## Admission, clocks and cancellation

```text
queued -> admitted -> prepared -> payload released -> execution ended -> retired
   |          |            |              |                 |
withdraw   cancel       uncertain      cancel tree       descendants remain
```

The host evaluates holds, logical occupancy and physical fit in its writer transaction, preserving
FIFO membership order. Waiters register a host/lane wake dependency before rechecking. Release,
hold resolution, preparation cleanup and authority changes wake the affected work; there is no
mission reserve-probe loop. Seat FIFO, burst reserve, scoped raises and delayed memory lowering
retain their existing meanings; declared priority/budget metadata is not activated by this API.

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

## Coverage and boundaries

Linux uses systemd user scopes in named slices with process placement/readback before the payload
gate. Required unsupported enforcement refuses rather than detaching. CPU quota is a ceiling,
not reserved cores; memory maximum is a hard accounting bound, not proof that memory is available.
No implicit `memory.high`, swap or IO policy is added to existing seats.

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
