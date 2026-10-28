# Seat lanes (design note)

Status: step 1 built (lanes on agents, behind a per-host switch); steps after it are design.

## What a seat lane is

A seat lane is a CPU and memory cap around one seat: its harness and every process the harness
starts, builds and test runners included. It is opt-in. A seat whose agent, mission or step
declares no `capacity` runs exactly as it does today, with today's build admission (the cargo shim,
`builds.slice`, `build-admission`), and sees no change.

```kdl
agent "example/builder" {
  capacity cores=1 memory="3G"     // base lane: sized for the harness thinking, not for builds
}
```

The declaration is `capacity`, not `lane`: the mission's named `lane "NAME" { entries … }` block is
a merge lane (see [lanes.md](lanes.md)) and keeps that one meaning.

## Linux mechanism

Every seat already runs in its own transient user scope, created by `systemd-run --user --scope` in
`crates/fractal-runtime/src/isolate.rs`. The user manager on our hosts has cpu, io, memory and pids
delegated, so no root is needed. A lane is that same scope started in `app.slice/app-lanes.slice`
with these properties (`crates/fractal-runtime/src/lane.rs`):

| Property | Value | Why |
| --- | --- | --- |
| `CPUQuota` | `cores × 100%` | caps CPU time |
| `MemoryMax` | `memory` | caps RAM; the kernel kills inside the lane past it |
| `MemorySwapMax` | `memory / 4` | the excess is killed in the lane, not moved to the host's swap |
| `OOMPolicy` | `continue` | only the killed process ends; systemd's default stops the whole scope, the seat with it |
| `TasksMax` | 8192 | as `builds.slice` |

There is no `MemoryHigh`. Measured on a build host (systemd 259): with `MemoryHigh` at 90% a child
asking for twice the lane sat in `mem_cgroup_handle_over_high` indefinitely (lane memory pressure
"full" 58%) and never reached `MemoryMax`, the same throttle stall `builds.slice` records. Without
it the child is killed (exit 137) and the task that started it keeps running. A lane needs systemd
254 or newer (`OOMPolicy` on a scope); an older user manager logs the lane as not enforced.

The cap is a property of the scope's cgroup, so every descendant is inside it, a child that leaves
its session included. The one thing that can take a child out is a process that asks systemd for a
new scope elsewhere. Today three do:

1. the cargo shim (`~/.local/bin/cargo`) runs build-like commands in a `builds.slice` scope;
2. `builds-slice-run`, cargo's test runner on the build hosts, does the same for test binaries;
3. `scripts/st-test` `governed()` wraps each building check in a `builds.slice` scope.

The first two already run in place when `BUILDS_SLICE_ACTIVE` is set, so a lane's environment sets
it, and `governed()` now follows the same rule (the variable survives the seat scrub and
`ci-local`, which drop `ST_*` and `FRACTAL_*`). The host's sccache server runs in `builds.slice`, so
compiles sent to it would leave the lane: a lane sets `SCCACHE_SERVER_UDS` to a socket of its own,
and the client starts that server inside the lane, on the same on-disk cache. A seat that names its
own server keeps it. The build admission queue (slots, load and memory checks) still applies to a
laned seat in step 1; its slots go only after lanes have run on every Linux member for a day without
a guard trip.

The lane is carried from the reconciler to the spawn under `FRACTAL_SEAT_CAPACITY`, which the
reconciler sets from the declaration (replacing any value the seat's own `env` names) and the
runtime removes before the harness starts. A task (`pty`, `exec`) an agent declares beside its
harness is not in the harness's lane.

A stopped seat's scope is ended with its seat (`isolate::end_scope`), so its lane is removed with
it; `--collect` removes a scope that empties on its own.

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

## macOS

No cgroups. A declared lane is logged at launch as "not enforced: this host has no systemd user
scopes (macOS has no per-tree CPU or memory cap for a user process)". Nothing is capped. Step 2
counts it for admission.

## Mixed versions

The member record gains an optional `capacity` field, omitted when absent, so an undeclared seat's
stored record and revision do not change. An older member reading a laned record drops the field
(the record has no `deny_unknown_fields`) and runs the seat without a lane. An older CLI refuses a
file that declares `capacity` at apply (unknown child), as for any new declaration. An older daemon
refuses a config with `lanes` set (`deny_unknown_fields`), so set the key only on hosts that run a
build with this change.

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
inside a mission) starts in a capped scope, its builds stay inside it, and stopping the seat
removes the lane. `a_laned_harness_runs_in_its_capped_scope_and_stopping_removes_it`
(`crates/fractal-runtime/src/pty.rs`) shows it for a terminal seat's harness;
`a_lane_caps_the_task_and_every_child_and_stopping_removes_it`
(`crates/fractal-runtime/src/process.rs`, skipped without user scopes) shows the caps, that a child
that left its session stays in the lane and shares its one core, and that the lane is gone after a
stop; `a_child_over_its_lane_memory_is_killed_and_the_task_keeps_running` (ignored until the guard
subtracts lanes) shows a child over the lane's memory is killed and the task keeps running.
Mission-level and step-level declarations, bursts and admission are step 2.
