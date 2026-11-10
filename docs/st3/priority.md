# Live-path priority

A person must always be able to ask Fractal what is happening and attach to a terminal, even while the
host's builds, tests and CI use every core and saturate the disk. Fractal keeps its live path ahead of
that work.

The live path is the Fractal daemon and each terminal's PTY server. The daemon answers every command.
The PTY server owns a terminal's screen and serves each attach. A harness and everything it starts
are work: its compiles, its tests, its tools.

## Seat priority

A standing agent seat's harness starts below the daemon by default, and everything it starts
inherits that. Finite work starts at normal priority: every exec task, such as a gate's tests or a
mission step's command, and every seat a mission run owns (`agent/RUN/...`). Someone waits on
finite work, and on macOS `taskpolicy -b` throttles CPU and disk IO hard enough to slow it several
times under load. One setting changes the standing-seat default for the install, and one line
changes it for any seat or exec task.

| Setting | Linux | macOS |
| --- | --- | --- |
| `background` (default) | `nice -n 10` and IO class best-effort, level 7 | `taskpolicy -b` |
| `normal` | the daemon's own priority | the daemon's own priority |

Only standing agent seats and what they start go to background by default: their harnesses,
helpers and shells an agent opens. An exec task or run-owned seat goes to background only when it
declares `ST_SEAT_PRIORITY "background"` itself; the install's `seat_priority` does not apply to
it. A terminal a person owns (`pty/person/NAME/ID`, such as a shell opened from the
TUI or the phone) and anything else a person drives interactively always run at normal priority,
whatever `seat_priority` and `ST_SEAT_PRIORITY` say. The daemon, the PTY servers and the `fractal` CLI
keep normal priority. The split is the same on Linux and macOS; on Linux the daemon and PTY servers
also keep their scope weights, so normal-priority work still cannot crowd out the live path there.

The prefix changes priority only. It does not change a seat's parent or lifetime: a seat's parent
is its PTY server, not the daemon, so restarting the daemon leaves every seat and its children
running, exactly as before. Nothing needs root, a new user or
a delegated cgroup controller, so it works on a fresh user install. On Linux, `nice` is added to
the daemon's own value, which is 0 under `fractal service install`. A harness that lowers its own
priority further stays lower. The scope weights below divide the machine between cgroups; `nice`
orders processes inside one, and a harness's work runs in a scope of its own.

**Install-wide.** In `~/.config/st3/config.toml` (Home Manager: `services.smalltalk.seatPriority`):

```toml
seat_priority = "normal"   # or "background", which is also what a missing key means
```

Run `fractal service restart`, then `fractal agents restart AGENT --as PERSON` for each running seat: a
seat keeps the priority it started with. `fractal doctor` names the value the daemon started with.

**One seat.** Declare `ST_SEAT_PRIORITY` in the seat's environment. It replaces the install's
value for that seat, in either direction:

```kdl
version 2
agent "garden/builder" {
  workspace "/work/garden"
  harness "claude" {}
  env { ST_SEAT_PRIORITY "normal" }
}
```

An unrecognized value reads as `background`, so a typo never gives a seat more CPU than the default.

IO level 7 acts only under the `bfq` and `mq-deadline` disk schedulers. On an NVMe disk with the
`none` scheduler, as on most hosts, it changes nothing; the CPU priority is what applies there.

## Declared admission

**Declared admission — declared, not enforced.**
st stores and returns these values. They do not change scheduling, admission, process priority or resource limits.

Agent and mission declarations accept optional, declaration-local metadata.
This mission example includes CPU, token and per-person attention budgets:

```kdl
mission "garden/check" {
  declared-priority "normal"
  declared-lane "check"
  declared-budgets {
    cpu-seconds 300
    memory-max "8G"
    disk-max "20G"
    token-budget 100000
    attention "person/reviewer" asks=3
    attention "person/operator" asks=1
  }
}
```

These children belong directly inside `agent "…" {}` or `mission "…" {}`, not a step.
Declared priority accepts the complete set `background < normal`; declared lane accepts
`check` or `build`, without an ordering. Neither selects the runtime `seat_priority`,
`ST_SEAT_PRIORITY`, or an enforced capacity lane.

**Budget window.** A mission's `token-budget` is the total cost-weighted tokens for
one mission run, including all attempts and retries; retrying does not start a new window.
`cpu-seconds` is accumulated CPU seconds over that same mission-run window.
See the [CPU reservation ledger contract](#cpu-reservation-ledger-contract) for the intended accounting.
A step-run window is a future extension, not an accepted declaration location.

Cost-weighted tokens are `sum(tokens * (class price / base input price))` across
models and usage classes: input, output, cache writes (including 1-hour cache writes),
and cached reads. The unit is one base input token at the pricing version pinned
at run admission and recorded on the run. A run is never repriced when prices change.
Unknown pricing reports unknown usage, never zero. Usage reporting must show USD
alongside cost-weighted tokens.

**Attention window.** Attention has its own window type. A mission's attention usage
for one person is the number of distinct native person-ask subjects
(`work.person-asked`) that the mission sent to that person in the rolling 7 days
that end at the read time. The window spans all recurring runs of the mission and
does not reset per run. An answer does not reduce the count. Ask updates and migrated
legacy asks do not count. A nested mission counts its own asks plus those from all
delegated descendants. Each ask also counts against every declaring ancestor's
budget for that person; a child's declaration does not replace an ancestor's.
Reason: a person spends attention over calendar time, and a per-run window would
let a recurring mission ask many times a day without exceeding its limit.
`Store::attention_request_count` reports this measure from one replica's data; the
result is not fleet-complete.

**Attention declaration.** Only missions accept repeatable
`attention "person/<id>" asks=N` children in `declared-budgets`, one entry per
canonical person subject. Each entry has exactly one string argument, exactly the
`asks` integer property, and no children; unknown properties or budget members refuse
publication. `asks` must be in the shared integer range 1..9007199254740991.
Typed entries are strict objects `{"person":"person/reviewer","asks":3}` in the
`declared_budgets.attention` array. Authored order is preserved; an empty array is
omitted. Semantic errors identify `declared_budgets.attention` or
`declared_budgets.attention[<person>].asks`. A missing person entry is undeclared,
never zero or unlimited. These declarations do not implement admission or enforcement.

**Agents.** New agent declarations accept only `memory-max` and `disk-max` in
`declared-budgets`; `cpu-seconds`, `token-budget` and `attention` refuse publication
with `invalid-declared-admission` and the corresponding budget field. An agent has
no retry/run boundary and no budget window. Agent and provider quota is governed by
usage routing and the quota SLO, not per-incarnation CPU, token or attention budgets.

Memory and disk describe peak footprints, without an accumulation window; disk is
not IO traffic or a free-space floor. CPU and token values must be integers from
1 to 9007199254740991. Sizes reuse the `capacity memory=` parser: plain byte counts
or integer strings followed by case-insensitive `K`, `M`, `G`, or `T` (powers of 1024).
The checked final byte count must be greater than zero and no greater than
18446744073709551615. The original size string is preserved.

These definitions are data only: no counters, resets, aggregation, pricing capture,
usage reporting, reservations, or enforcement are implemented by declared admission.
The window and pricing rules define the intended accounting contract, not a runtime
accounting implementation.

Omission means not declared, never zero, unlimited, or a runtime default. There is no
inheritance or merging between declarations; replacement omission removes the value.
Empty budgets are omitted from stored typed fields and reads. Duplicate children,
unknown budget members, authored nulls, invalid types, invalid enum values and
out-of-range values refuse the entire publication, before writes. Semantic refusals
use `invalid-declared-admission` with `details.subject` and `details.field`; syntax
refusals retain the existing declaration syntax codes.

Typed declaration fields are `declared_priority`, `declared_lane`, and
`declared_budgets`; budget members use snake_case. Existing agent and mission reads
instead expose a single optional, nonnullable `declared_admission` group. This is
the mission example's read shape:

```json
{"declared_admission":{"priority":"normal","lane":"check","budgets":{"cpu_seconds":300,"memory_max":"8G","disk_max":"20G","token_budget":100000,"attention":[{"person":"person/reviewer","asks":3},{"person":"person/operator","asks":1}]}}}
```

Absent members, empty budgets, and the empty group are omitted. Readers preserve future
enum strings; TypeScript tolerant codecs expose `{_tag:"Unknown",raw:"future-level"}`
and re-encode the original string. Strict decoding deliberately rejects unknown cases.
These fields extend existing declaration claims only, with no new table, claim kind,
index, migration, or backfill. Old declarations without the fields retain their
canonical bytes and hashes.

CLI and API authoring use the same scope rules: agent budgets are peak memory/disk
only; mission budgets may also contain accumulated CPU, cost-weighted tokens and
per-person attention. Historical readers retain CPU/token fields in older agent
declarations; this does not permit new agent authoring of those fields. Replay and
historical interpretation are unchanged.

**Publication compatibility.** Every new declared-admission publication requires
`features.declared_admission=1` from every active incarnation, including the local
daemon. Publications containing attention additionally require
`features.declared_attention=1` from every active incarnation. Both capabilities
must appear in each incarnation's latest signed, authenticated, own-origin
`daemon.started` advertisement anchored to the admitted incarnation. Missing or
unsupported capability evidence refuses before writes with
`declared-admission-feature-barrier`. Attention-free declarations continue
publishing in mixed fleets with version-1 declared-admission support; upgrade active
members or fence old peers before publishing attention. Remove attention using
supporting readers before a mixed-version rollback. Old KDL and typed authoring
already reject unknown budget members rather than silently dropping attention.
There is no database, replay or enforcement change.

### Selector rules (priority, aging, no preemption)

Status: definitions only. No selector applies these rules yet. Both the seat-queue
shadow selector (owned by st-budgets and Fractal Product Lead) and the later
job-admission selector (owned by the gate-slot retirement program) use this contract.
The rules take a queue as a parameter: a seat queue or a job queue. Use only the
declared priority values and unordered lanes listed above.

**No preemption.** A selector chooses only among waiting entries. It never stops,
pauses or reorders running work, and never takes away a held claim or slot.

**Aging and grace.** Treat a waiting entry below the highest priority as one priority
level higher once it has waited longer than its queue's grace interval. Within one
priority level, keep FIFO order by queue join time.

The grace interval is the p90 of observed waits for normal-priority entries in that
queue, over a pinned time window. Measure each wait from ready or queue join to first
claim or admission, as applicable to the queue. Each replica computes the interval
from its own history. Every selector decision records the window, the method and
the value. If the window has no observed normal-priority wait, the grace interval is
unknown. Then the selector does not reorder by priority: it keeps FIFO order by queue
join time and records the grace interval as unknown.

**Current state.** The seat queue is FIFO with manual moves. The budgets-v1 step
`seat-shadow` will only record shadow decisions for one dedicated seat; FIFO still
decides. Turning on active admission needs Johannes's separate approval.

### Attention report

The client-v0 operation
`GET /v1/client/attention-budget?mission=M[&person=P][&at=UNIX_MS]`
reads the [attention window and declaration](#declared-admission) for one mission and person.
Embedded-web recipient sessions may read their own report. Local agents and ordinary
paired clients with `read.projections` may also read reports, including over
Fabric. External assistant grants cannot read the report.
Missing scope and expired sessions are refused; a raw Unix person header
does not establish recipient authority. Existing `person_filter` restrictions
limit person sessions to their own report: omitted person defaults to the session
person, and another person is refused. Agents without a session person must specify
`person`; omission never reads all people.

The CLI uses the same client operation:
`fractal budgets attention MISSION [--as ACTOR] [--person PERSON] [--at UNIX_MS] [--json]`.
For Unix calls, `--as` names the acting person or agent (default: `ST_AGENT`, including
the existing `FRACTAL_AGENT` alias, then the configured person); `--person` selects
the budget recipient and never establishes acting identity. A harness cannot name
another agent as its actor. Fabric calls use their paired credential instead.

The report distinguishes declared asks from undeclared, charged from inclusive usage,
not-charged requests, and unattributed requests for the person. It identifies the canonical
mission and person, exclusive window start, inclusive end, and local claim index.
The window is `(at - 7 days, at]`. `declared` reads the current revision; charging
uses each originating or ancestor run's initial revision, so adding a limit later
does not charge older runs. Every declaring ancestor is charged independently.
`fleet_complete` is always false. At most 128 newest request references expose only claim
IDs and subjects, never request text; `request_refs_complete` states whether all fit.
This is data only: no limit enforcement, delivery decision, write, or fleet-complete promise.

Reads examine at most 10,001 retained lifetime native ask claim rows for the
recipient through the existing recipient index, newest local admission first;
more than 10,000 refuses the whole report before applying the weekly window.
This includes updates and expired asks, not just distinct current-window requests.
Retained history therefore grows toward refusal even when weekly usage is low;
the report does not silently discard older history. Unrelated recipients and other
claim kinds are not scanned. A valid ancestry continuation beyond 64 vertices also
refuses the whole report, even if the mission was already found; missing provenance
remains distinct and may be reported as unattributed.

### CPU reservation ledger (contract)

Status: contract only, data only. No code implements it yet. Owner of the semantics:
st-budgets. Implementer: the gate-slot retirement program (job reserve/settle hooks
and the cgroup v2 `cpu.stat` collector), after its durable-job work. Decision: Johannes,
`step-run/ask-139e87e814cd8ef4cc9b64262722413d/ask` (R5), with inclusive charging from
`step-run/ask-ba7954f15024216f4209fa4f421b3f2a/ask` (R2).

**Purpose.** A mission's `cpu-seconds` becomes a total cap on the CPU seconds that its
jobs use in one mission run. One authoritative ledger per mission run records
reservations and settlements.

**Order of delivery.**

1. Report: a per-run CPU report (settled, reserved, unknown) ships and is observed on real runs.
2. Admission: then the ledger can refuse to start a job whose reservation does not fit.

The first stage, reporting, changes no behavior. Turning admission on for real missions
needs Johannes's explicit approval, because budget declarations are data only until then.

**Unit and measurement.**

- One CPU second is 10^6 `usage_usec` from cgroup v2 `cpu.stat` of the job attempt's own scope.
- The collector reads the scope at start and at end, and records the delta, the physical
  host and the observation timestamps. `usage_usec` includes descendant cgroups.
- A transient scope's cgroup disappears at exit. The collector reads it before removal,
  or uses the unit's `CPUUsageNSec` with `CPUAccounting=yes`.
- A job attempt without its own cgroup v2 scope has unknown usage. This includes macOS
  hosts, remote builders and a lost host. Unknown is never zero.

**Ledger records** (append-only, one ledger per mission run, fields named for the contract):

- `reservation`: ledger run, job id, attempt, host, `reserved_cpu_seconds`, the list of
  charged mission runs, and the time.
- `settlement`: the reservation id, then `measured_cpu_seconds` or `unknown` with a reason,
  coverage, and the time.

**Reservation size.**

- The reservation is the job attempt's provable CPU upper bound, in CPU seconds: the
  wall-clock deadline × min(the `cpu.max` rate of the attempt's scope and of every ancestor
  cgroup, the CPU count of the scope's `cpuset.cpus.effective`).
- The cgroup hierarchy enforces that minimum. The implementer records which cgroup
  supplied the bound.
- A job attempt without a deadline, or on a host without cgroup v2 (macOS, remote
  builders), has no provable bound. It can run in report mode only; in admission mode
  it is refused for a capped mission.
- The bound is conservative: an uncapped attempt reserves all of its effective CPUs for
  its whole deadline, so a capped mission can wait for a long time in admission mode.
  Settlement corrects the charge when the attempt ends. A per-job `cpu.max` or a shorter
  deadline gives a tighter bound.

**Reserve before each job attempt.**

- Before the attempt starts, the ledger adds one reservation for that attempt.
- The ledger charges the reservation to every mission run that declares `cpu-seconds`
  on the job's ancestry: the job's own run and every declaring ancestor (R2). A job is
  counted at most once in each ledger.
- In admission mode, the attempt starts only if, for every charged run:
  settled + open reservations + this reservation ≤ that run's `cpu-seconds`. Otherwise:
  - If the attempt could fit after open reservations settle, it waits in its queue.
    It is not killed and not failed.
  - If the attempt's reservation alone exceeds a charged run's `cpu-seconds` minus that
    run's settled usage, the attempt can never fit. The ledger refuses it with the typed
    reason `budget-cannot-fit`, which names the run and the numbers, and the report shows it.

**Settle after each job attempt.**

- When the attempt ends, the settlement replaces the reservation with the measured value.
- If the measurement is unknown, the reservation stays charged in full (R5).
- If the measured value exceeds the reservation, the measured value is charged, and the
  report marks a bound violation: the bound was wrong.

**Retries and delegation.**

- Each retry attempt is reserved and settled separately in the same mission-run window
  (the #6092 budget window); a retry does not start a new window.
- A delegated child run charges its own ledger and every declaring ancestor's ledger.

**Limits pin and scope.** The limit comes from each run's initial revision. Only runs
that start after the feature activates are charged (new runs only).

**Running work.** No missing, late or unknown usage stops or kills running work. Only the
admission of new attempts is affected.

**Authority and failure.**

- Each mission run has one authoritative ledger writer: the replica that admits the run's jobs.
- The single ledger writer is fenced like a step claim, so a replica failover cannot
  admit twice.
- In admission mode, `budget-ledger-unavailable` is a typed WAIT reason, not a refusal.
  It applies when the ledger is unreachable or when the writer cannot prove its copy
  current. The attempt waits, and running attempts continue.
- Keep this WAIT distinct from the refusal `budget-cannot-fit`.

**Report fields** (per mission run):

- `cpu_seconds` declared, or tagged `undeclared`;
- `charged_settled`;
- `charged_reserved_open`;
- `charged_unknown_retained`;
- `inclusive_measured` (all usage in the subtree, separate from charged);
- `unknown_attempts`;
- `refused_cannot_fit` (attempts refused with `budget-cannot-fit`).

## Linux

On Linux with a systemd user manager, Fractal starts each terminal in a transient scope that it records
on the session as the `st3.scope-unit` tag. The PTY server starts there and forks the harness.
Once the session is published, Fractal moves the PTY server alone into a scope of its own. The harness
stays behind.

| What | Unit | CPU weight | IO weight |
| --- | --- | --- | --- |
| Fractal daemon | `st3.service` | 1000 | 1000 |
| each PTY server | `st3-pty-server-RUNTIME-PID.scope` | 1000 | 1000 |
| each harness and its children | `st3-RUNTIME-DAEMONPID-N.scope` | 100 (default) | 100 (default) |
| each exec task, such as a gate | `st3-RUNTIME-DAEMONPID-N.scope` | 100 (default) | 100 (default) |

The move goes through the user manager (`StartTransientUnit` with the server's pid), so systemd
owns both scopes and collects each one when its last process exits. The server scope also sets
`MemoryLow=64M`, which takes effect only once the user manager itself has a memory.low (see
below). The move does not change what a stop ends; see [Stopping a session](#stopping-a-session).

The daemon moves a PTY server that an older release started, the first time its reconciler
observes it. It moves only a process that is still in the exact scope Fractal recorded for that
session, and never itself.

`fractal service install` renders `CPUWeight=1000` and `IOWeight=1000` into `st3.service`. The
replication worker keeps the default weight. An existing install keeps its unit until it is
reinstalled; `systemctl --user set-property st3.service CPUWeight=1000 IOWeight=1000` applies the
same weights in place.

IO weights apply only when the user manager delegates the io controller and the disk has an IO
controller such as iocost. `ionice` is no substitute: its classes act only under the `bfq` and
`mq-deadline` schedulers, and an NVMe disk usually runs `none`. Most distributions delegate only
`cpu memory pids` to `user@.service`.
To delegate io, as root:

```sh
mkdir -p /etc/systemd/system/user@.service.d
printf '[Service]\nDelegate=cpu io memory pids\n' \
  > /etc/systemd/system/user@.service.d/60-delegate-io.conf
systemctl daemon-reload
systemctl restart user@1000.service   # or log out of every session and back in
```

Memory protection needs root too. A cgroup's `memory.low` protects it from reclaim and swap only
up to what every ancestor is protected for, and `user.slice`, `user-UID.slice` and
`user@UID.service` have none by default. As root, each of these applies at once and persists:

```sh
systemctl set-property user.slice MemoryLow=16G
systemctl set-property user-1000.slice MemoryLow=16G
systemctl set-property user@1000.service MemoryLow=16G
```

Then, as the user, `systemctl --user set-property app.slice MemoryLow=8G` and
`systemctl --user set-property st3.service MemoryLow=3G` protect the daemon, and each PTY server
scope's `MemoryLow=64M` applies.

Without a systemd user manager, Fractal runs sessions detached and PTY servers share CPU and IO with
their harnesses.

## macOS

macOS has no cgroups. The daemon runs as a launchd agent with `ProcessType` `Interactive`, and the
PTY servers it starts keep that agent's default QoS. Fractal starts each harness, and each exec task,
under `/usr/sbin/taskpolicy -b`. That runs the program and every child it starts with its resources
throttled, below the daemon's default QoS. `nice` alone is not enough on macOS: on a
10-core Mac, a test run at `nice 20` (13 processes, 226% CPU) took the load average to 54 and made
the daemon answer slowly, and `taskpolicy -b` on the same processes brought it to 23 within a minute
and 16 within five. The cost is that throttled builds can be confined to efficiency cores; set
`normal` for a seat that needs more. A seat at `normal` priority starts without the policy.

## Stopping a session

Stopping a seat, or an exec task such as a gate, ends every process it started.

On Linux with a systemd user manager, everything a harness or exec task starts stays in its work
scope, `st3-RUNTIME-DAEMONPID-N.scope`: a process whose parent exited, one that started a session
or process group of its own, such as each test `cargo nextest` runs, and one started while the stop
runs. A stop ends that scope.

1. For a PTY session, pty stops the PTY server, then signals the process tree and process groups
   it measured before the signal. Fractal then sends SIGTERM to every process still in the work scope
   (`systemctl --user kill`), waits up to two seconds, and sends SIGKILL.
2. For an exec task, the whole work scope gets SIGTERM, not only the task's process group.
3. At the shutdown deadline, a kill sends SIGKILL to the terminal's or the task's process group and
   to the whole work scope. A PTY server still in the work scope leaves it first, so it outlives
   its harness and records the exit.
4. A harness can exit on its own and leave processes behind, and its session's record then removes
   itself. Fractal keeps the name of each launch's work scope beside the session's spawn lock. It ends
   that scope, SIGTERM and then SIGKILL as above, when the reconciler next finds the stopped
   runtime not running, when the session is removed, and before a restart launches the next
   incarnation. An exec task's record names its scope, and the same three moments end it. Each
   launch has a scope of its own, so ending an old one never reaches the next incarnation.

A stop or a retirement of an agent root also stops every exec and PTY task declared under it
(`exec/ROOT/TASK`), each through its own stop above, whether or not the root has a process of its
own. Those tasks are not started or restarted while the root is stopped, whatever their restart
policy, and the root is not recorded as stopped while one of them still runs on this host; a task
whose stop fails keeps its own fault, and the root stays unsettled until it ends. A task of an
earlier release could outlive a stop of its root; it no longer does. A task is stopped by the
host it runs on. Starting the root again declares them again.

A unit that is no longer loaded has ended: systemd collects a work scope once its last process
exits. Fractal never ends a PTY server's own scope, a service, or the unit it runs in.

Without a systemd user manager, on macOS and on Linux without a user bus such as many CI runners,
Fractal has no scope to end. A stop ends the PTY server's process tree and the process groups pty
measured before the signal, and an exec task's process group. Three kinds of process keep
running: one that left that tree before the stop because its parent exited (`nohup`, `setsid`, a
tool's background task), a process group started after pty measured the tree, and whatever a
harness left behind when it exited on its own.

A session that a release before this one started records no scope beside its spawn lock. Its
leftovers end when it is stopped while it runs, or while its own record still names the scope.

## Checking a host

`fractal doctor` reports a `priority` check. On Linux it warns when the daemon's own weights are below
1000, when the user manager does not delegate io (with the root command above), and when any PTY
server still shares its harness's scope. On macOS it warns when `taskpolicy` is missing.
`fractal doctor --strict` fails on those warnings.

To look by hand:

```sh
systemd-cgls --user-unit st3.service
for d in /sys/fs/cgroup/user.slice/user-$(id -u).slice/user@$(id -u).service/app.slice/st3-*/; do
  echo "$(basename "$d") $(cat "$d/cpu.weight") $(cat "$d/io.weight")"
done
```

## What this does not cover

- The Fractal CLI runs in its caller's cgroup. An agent that runs `fractal` from its own seat shares that
  seat's weight with its own build.
- A process that a harness moves out of its work scope itself, such as one it starts with
  `systemd-run --user`, is no longer in that scope and outlives a stop of the seat.
- Remote attach also crosses `fabric.service`, which Fractal does not install. Give it the live weights
  with `systemctl --user set-property fabric.service CPUWeight=1000 IOWeight=1000`.
- Weights share a resource under contention. They cannot make a disk that is queued for seconds
  answer in milliseconds. Keeping the Fractal state directory on a disk the builds do not use removes
  that contention entirely.
