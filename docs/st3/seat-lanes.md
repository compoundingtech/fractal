# Seat lanes (design note)

Status: step 1 built (lanes on agents, behind a per-host switch); steps after it are design.

## What a seat lane is

A seat lane is a CPU and memory cap around one seat: its harness and every process the harness
starts, builds and test runners included. It is opt-in. A seat whose agent, mission or step
declares no `capacity` runs exactly as it does today, with today's build admission (the cargo shim,
`builds.slice`, `build-admission`), and sees no change.

A lane is a resource cap that seats cooperate with, not a security boundary. Every seat runs as the
same user as its user manager, so a seat can change its slice's limits or start a process outside
it. Lanes keep well-behaved seats from crowding each other out; they do not contain a seat that
tries to escape.

```kdl
agent "example/builder" {
  capacity cores=1 memory="3G"     // base lane: sized for the harness thinking, not for builds
}
```

The declaration is `capacity`, not `lane`: the mission's named `lane "NAME" { entries … }` block is
a merge lane (see [lanes.md](lanes.md)) and keeps that one meaning.

## Linux mechanism

Every seat already runs in its own transient user scope, created by `systemd-run --user --scope` in
`crates/fractal-runtime/src/isolate.rs`, one per launch of its harness. The user manager on our
hosts has cpu, io, memory and pids delegated, so no root is needed. A lane is a slice of the seat's
own, `app.slice/app-lanes.slice/app-lanes-<seat>_<digest>.slice` (the seat's dashes become
underscores, since a dash nests a slice, and a digest of the whole seat id keeps two seats that read
alike apart), capped before anything starts in it (`crates/fractal-runtime/src/lane.rs`):

| On | Property | Value | Why |
| --- | --- | --- | --- |
| the seat's slice | `CPUQuota` | `cores × 100%` | caps CPU time |
| | `MemoryMax` | `memory` | caps RAM; the kernel kills inside the lane past it |
| | `MemorySwapMax` | `memory / 4` | the excess is killed in the lane, not moved to the host's swap |
| | `TasksMax` | 8192 | as `builds.slice` |
| each harness scope | `OOMPolicy` | `continue` | only the killed process ends; systemd's default stops the whole scope, the seat with it |

Each launch of the harness is a scope inside the slice. A restart (a context rotation, a crash, a
declaration change) ends the old scope with what the harness started in it, as today, and starts the
new one in the same slice. A job the seat wants to outlive a restart, such as a queued `st-test` run,
starts as a unit of its own in the slice: the harness's environment names it in
`FRACTAL_LANE_SLICE`, so `systemd-run --user --unit=st-test-<seat>-… --slice="$FRACTAL_LANE_SLICE" …`
runs it inside the seat's cap and beside, not inside, the harness's scope. Such a unit starts with
the user manager's environment, not the seat's, so it passes the lane's variables on itself
(`--setenv=BUILDS_SLICE_ACTIVE=1 --setenv=FRACTAL_LANE_SLICE --setenv=SCCACHE_SERVER_UDS`, each
`--setenv=NAME` copying the seat's value). Without `BUILDS_SLICE_ACTIVE`, a build host's cargo front
door moves each cargo command into `builds.slice`, out of the lane; `st-test` sets it for its own
checks when it finds itself in a lane, whatever started it, and puts the checks of a unit the seat
started outside its lane back in it (below). The slice ends, and every
job in it with it, when the seat stops for good: the reconciler ends it when a stop completes and the
seat's declaration is an explicit stop or gone. A stop for a restart of a live declaration keeps it.
Ending a lane also removes the slice's runtime limits (`systemctl --user revert`). A seat whose lane
was prepared before a daemon restart is found by its slice in the cgroup tree, so its lane still
ends; a seat without a lane costs that one look and nothing else.

There is no `MemoryHigh`. Measured on a build host (systemd 259): with `MemoryHigh` at 90% a child
asking for twice the lane sat in `mem_cgroup_handle_over_high` indefinitely (lane memory pressure
"full" 58%) and never reached `MemoryMax`, the same throttle stall `builds.slice` records. Without
it the child is killed (exit 137) and the task that started it keeps running. A lane needs systemd
254 or newer (`OOMPolicy` on a scope); an older user manager logs the lane as not enforced.

The cap is a property of the slice's cgroup, so every descendant is inside it, a child that leaves
its session included. The one thing that can take a child out is a process that asks systemd for a
new scope elsewhere. Today three do:

1. the cargo shim (`~/.local/bin/cargo`) runs build-like commands in a `builds.slice` scope;
2. `builds-slice-run`, cargo's test runner on the build hosts, does the same for test binaries;
3. `scripts/st-test` `governed()` wraps each building check in a `builds.slice` scope.

The first two already run in place when `BUILDS_SLICE_ACTIVE` is set, so a lane's environment sets
it. `governed()` signs checks, so it takes neither a variable's nor a name's word alone: it runs a check
in place only when it runs inside one of the two caps made for builds, `builds.slice` or a seat's
lane (an `app-lanes-<seat>.slice` directly in `app-lanes.slice`), and that slice or a cgroup below it
holds a real `memory.max` and a real `cpu.max` quota. Limits anywhere else do not count. In place,
the check runs with `BUILDS_SLICE_ACTIVE=1`, so a cargo front door that reads only that variable keeps
its cargo in the cap. Outside every cap, a caller that names a seat (`ST_AGENT`, as a unit the
seat started without `--slice` does) asks the daemon for the seat's slice (`st agents lane AGENT`,
the reconciler's own name for it) and, when the kernel shows that slice in `app-lanes.slice` with
both limits, starts the check in a scope there, with `BUILDS_SLICE_ACTIVE=1` and
`FRACTAL_LANE_SLICE`. Otherwise it starts the check in `builds.slice` as before, and a seat's run says
why it is not in the lane. The host's sccache server runs in `builds.slice`, so
compiles sent to it would leave the lane: a lane sets `SCCACHE_SERVER_UDS` to a socket of its own,
and the client starts that server inside the lane, on the same on-disk cache (the socket is named for
the seat, so a restarted harness reuses it). A seat that names its own server keeps it. The build admission queue (slots, load and memory checks) still applies to a
laned seat in step 1; its slots go only after lanes have run on every Linux member for a day without
a guard trip.

The lane is carried from the reconciler to the spawn under `FRACTAL_SEAT_CAPACITY`, which the
reconciler sets from the declaration (replacing any value the seat's own `env` names) and the
runtime removes before the harness starts. A task (`pty`, `exec`) an agent declares beside its
harness is not in the harness's lane.

A stopped harness's scope is ended with it (`isolate::end_scope`); `--collect` removes a scope that
empties on its own. The lane outlives it until the seat stops for good (above).

## The host guard

The kernel OOM-kills inside a lane on purpose. A host guard that counts every `oom_kill` in the user
slice would read that as a fault, so it counts only the kills outside lanes:

```
oom_kill(user.slice/user-UID.slice/memory.events)
  - oom_kill(user.slice/user-UID.slice/user@UID.service/app.slice/app-lanes.slice/memory.events)
```

`memory.events` is hierarchical and `app-lanes.slice` stays active after its last lane ends, so its
count only grows until the user manager restarts. Before the first lane starts the slice is absent
and counts 0. Until every host's guard reads this, the repository test that makes an OOM kill on
purpose is `#[ignore]`d.

## The per-host switch

`lanes = "on"` in `~/.config/st3/config.toml` (default `off`). It affects only seats that declare a
`capacity`. With it off, a declared lane is logged at launch as "not enforced: lanes are off on this
host", and the seat starts as today. A seat's lane is applied when the seat starts; a change to the
declaration or the switch takes effect at its next start.

### Placement for unlaned daemon seats

The optional `[unlaned_seat]` table in `~/.config/st3/config.toml` places only daemon-launched
seats that have no active lane. Omit it to keep the existing placement. Restart the daemon
after changing it; existing seats retain their placement. Laned seats keep their lane slice
and limits; directly constructed runtimes and non-systemd launches do not use this policy.
Do not combine it with a generic `st3-.scope.d` drop-in that overrides `Slice=`.

For placement without resource caps:

```toml
[unlaned_seat]
slice = "app-seats.slice"
```

To also set caps and OOM behavior:

```toml
[unlaned_seat]
slice = "app-seats.slice"
memoryMax = 8589934592
memorySwapMax = 0
oomPolicy = "kill"
```

Only `slice` is required; unknown fields are rejected. `memoryMax`, `memorySwapMax`, and
`oomPolicy` are independently optional: an absent field emits no corresponding `-p` argument.
When present, memory values are integer byte counts (`memoryMax` positive, `memorySwapMax`
nonnegative). `oomPolicy` is `continue`, `stop`, or `kill`; setting it requires a systemd version
supporting `OOMPolicy` on scopes (254+).
`slice` is a non-root `.slice` name containing letters, digits, underscores and single internal
hyphens. The launcher passes `--slice=`, `MemoryMax=`, `MemorySwapMax=` and `OOMPolicy=` directly
to `systemd-run`. Older daemons reject this table. Upgrade: install/start the new daemon first,
then add `[unlaned_seat]` and restart. Downgrade: remove the table first, then install the old daemon.


## macOS

No cgroups. A declared lane is logged at launch as "not enforced: this host has no systemd user
scopes (macOS has no per-tree CPU or memory cap for a user process)". Nothing is capped. Step 2
counts it for admission.

## Mixed versions

The member record gains an optional `capacity` field, omitted when absent, so an undeclared seat's
stored record and revision do not change. An older member reading a laned record drops the field
(the record has no `deny_unknown_fields`) and runs the seat without a lane. An older CLI refuses a
file that declares `capacity` at apply (unknown child), as for any new declaration. An older daemon
refuses a config with `lanes` set (`deny_unknown_fields`) and does not start, so set the key only on
hosts that run a build with this change, and remove it before downgrading a host to an older build.

## Later steps (decided, not built in step 1)

- **Bursts.** A heavy command asks for a burst; `st-test` asks for one sized to the checks it will
  run (covered checks skip, a docs-only run needs almost nothing). A burst raises the live scope
  with `systemctl --user set-property --runtime` and lowers it when the command ends. A step may
  adjust the base. A seat in `work wait` holds only its base.
- **Admission by fit.** One formula on every host, sized to that host: capacity is the host's cores
  and memory less a reserve for the daemon and the system (proposed: cores − max(2, cores/8),
  memory − max(4G, 12.5%)). Memory is firm: the sum of every lane's base and burst memory must
  fit. CPU is shareable: quotas may oversubscribe, weights decide under contention. Whatever waits
  says why, naming the host's free capacity.
- **Starvation.** Waiters age. After 15 minutes the host drains to fit (admits nothing new until
  the oldest waiter fits). A lane or burst may declare a minimum and start smaller. After 30
  minutes the seat should move to a host with room (by hand until moves are automatic). A wait
  over an hour is a fault to the run's report-to. The daily lane-wait p90 per host is the capacity
  signal.

## Step 1 deliverable

Behind the switch, a seat that declares `capacity cores=… memory=…` on its agent (top-level or
inside a mission) runs in a capped slice of its own, its builds stay inside it, a job it starts as
its own unit there outlives a restart of its harness, and stopping the seat for good removes the
lane. The tests:

- `a_lane_caps_the_seat_keeps_its_jobs_across_a_restart_and_ends_for_good`
  (`crates/fractal-runtime/src/process.rs`, skipped without systemd 254 user scopes): the slice's
  limits, `OOMPolicy` on the scope, a child that left its session stays in the lane and shares its
  one core, a job unit in the slice survives a stop and relaunch, and ending the lane ends the job,
  the slice and its limits; with the switch off, no lane.
- `a_laned_harness_runs_in_its_seats_capped_slice_until_the_lane_ends`
  (`crates/fractal-runtime/src/pty.rs`): the same for a terminal seat's harness.
- `a_child_over_its_lane_memory_is_killed_and_the_task_keeps_running` (ignored by default; run by
  hand where the guard subtracts lanes, after telling whoever runs it): a child over the lane's
  memory is killed and the task keeps running.
- `a_stop_ends_what_an_ended_harness_left_running` and `restart_types_follow_success_and_failure`
  (`crates/fractal-reconcile/src/reconcile.rs`): an explicit stop ends the lane; an exit or a
  relaunch of a live declaration keeps it.

Mission-level and step-level declarations, bursts and admission are step 2.
