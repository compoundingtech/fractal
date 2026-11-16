# client-v0 proposal: run ETA

Status: timing fields landed in #5036; ETA follows the #5154/#5172/#5120
stack. The additions are additive and optional: older daemons omit them.

Person pins are specified separately in #4923 (arrangement-v2 ordered
membership) and #4962.

## Current behavior and intended measure

The web sidebar needs run and step timing and a remaining-duration estimate
for an open run. Timing comes from existing claims; ETA uses derived samples.

The current contract defines `MissionRunSummary` and `MissionStep` in
`docs/st3/client-v0/schemas/client-v0.schema.json`.
The presentation rules in `docs/clients/ui-contract.md` serve display state
computed once on the published card. An ETA instead belongs to one open run:
a mission row reads its current open run. It does not belong on
`MissionPresentation`.

## Negotiation and wire shape

### Run and step timing

`MissionRunSummary` and `MissionStep` expose these nullable timestamps:

```text
started_at: integer | null     // Unix milliseconds
completed_at: integer | null   // Unix milliseconds
```

Run start is run creation. Run end is `accepted_at_unix_ms` on the
first `mission-run.state` claim that transitions the run into `phase: "terminal"`,
as projected by `project_mission_run_update` in
`crates/fractal-store/src/store.rs`. It is not `outcome.at`:
`run_outcome_from_claim` only exposes terminal-to-terminal overrides.

Step timing uses the selected attempt: start is its first execution opening,
including `work.claimed` for worker-backed steps and the reconciler-owned
`step-run.state` working transition for agentless steps. Completion is its
first authoritative terminal transition, not submission awaiting verification,
release, or lease expiry. Reclaim preserves the first opening; retry selects a
new attempt. Unknown edges are null. These lifecycle timestamps are distinct
from the execution intervals folded by `fold_step_clock_open`.
Timing enrichment is bounded to open runs plus the latest finished run.

### Run ETA

Add an optional `eta` field to `MissionRunSummary`:

```text
eta?: {
  remaining_low_ms: integer,    // p25 of the remaining duration
  remaining_high_ms: integer,   // p75 of the remaining duration
  basis: { method: "per-step" | "critical-path", samples: integer },
  as_of: integer              // Unix milliseconds
} | null
```

The ETA is served as remaining durations plus `as_of`, not as absolute
timestamps. Absolute times become wrong as soon as a run pauses. With
durations, a client can show "over by 10m" and can show a pause honestly.
While the principal holds the run, a client may show "waiting on you" instead
of the ETA. The server still serves the value.

`null` means one of two things: some remaining step has zero completed
samples, or the run is finished. Older daemons omit the field. `as_of` is the
publication cut that produced the value.

## Capture and delivery across layers

ETA computation runs on the server. The time basis is the wall time of each
step, excluding only the periods when a person holds the step (a decision or
a person wait). Queueing and agent waits count, because the reader experiences
them as real delay.

For each remaining step, the daemon takes nearest-rank p25 and p75 of that
duration for the same step path under the current definition hash. Candidate
runs are the newest 64 terminal runs per definition hash, ordered by first
terminal time, then subject. Only completed runs and completed selected
attempts with usable start and completion edges contribute samples; unusable
candidates are not replaced by older runs. Person-held time is subtracted.
If dependencies are declared in the mission definition's `step.dependencies`,
the daemon sums along the critical path, and `method` is `critical-path`.
These declarations supply the graph's dependency edges; runtime step
`blockers` are current `work.person-asked` subjects, not dependency edges.
Otherwise the daemon sums over the remaining steps, and `method` is `per-step`.

`samples` is the smallest sample count among the remaining steps. With 1 or 2
samples the value is still served, so a client can say "(1 prior run)". A running
run with no remaining steps returns zero for both durations and `samples: 0`.

## Contract surface and consumer coordination

### Contract decisions and implementation

`eta` belongs on `MissionRunSummary`, not `Mission` or `MissionPresentation`.
Timing fields landed in #5036; ETA sampling and serving follow the
#5154/#5172/#5120 stack. Both use always-on optional fields, not a capability
gate. The schema and `README.md` define the implemented wire contract.

## Verification required for implementation

- Check timing against run creation and the `accepted_at_unix_ms` of the
  `mission-run.state` transition into `phase: "terminal"`, not a later
  terminal-to-terminal outcome override. Check the first execution opening and
  authoritative terminal transition of the selected step attempt, including
  agentless steps. Submission awaiting verification, release, and lease expiry
  must not complete a step; retry selects a new attempt. Preserve the
  open-runs-plus-latest-finished-run bound.
- Check per-step and critical-path estimates using declared
  `step.dependencies`, not runtime person-work `blockers`; check the
  newest-64-terminal-runs-per-definition-hash candidate bound, skipping unusable
  starts without backfilling older runs, and nearest-rank quartiles. Check
  exclusion of person-held time without excluding queueing or agent waits.
  Check the smallest sample count, including 1 or 2 samples, zero durations
  and `samples: 0` when a running run has no remaining steps, and `null` for a
  zero-sample remaining step or a finished run.
- Check remaining durations and the publication-cut `as_of`, including the
  display of a paused or overdue run and omission by older daemons.
