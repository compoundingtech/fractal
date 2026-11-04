# client-v0 proposal: run ETA

Status: client-v0 contract proposal for the web sidebar; no implementation or
wire behavior change yet. The additions are additive and optional: older
daemons omit them.

Person pins are specified separately in #4923 (arrangement-v2 ordered
membership) and #4962.

## Current behavior and intended measure

The web sidebar needs run and step timing and a remaining-duration estimate
for an open run. Timing comes from existing folds, with no new storage.

The current contract defines `MissionRunSummary` and `MissionStep` in
`docs/st3/client-v0/schemas/client-v0.schema.json`.
The presentation rules in `docs/clients/ui-contract.md` serve display state
computed once on the published card. An ETA instead belongs to one open run:
a mission row reads its current open run. It does not belong on
`MissionPresentation`.

## Negotiation and wire shape

### Run and step timing

As a prerequisite, add these nullable timestamp fields to `MissionRunSummary`
and `MissionStep`:

```text
started_at: Timestamp | null
completed_at: Timestamp | null
```

Run start is run creation. Run end is `accepted_at_unix_ms` on the
`mission-run.state` claim that transitions the run into `phase: "terminal"`,
as projected by `project_mission_run_update` in
`crates/fractal-store/src/store.rs`. It is not `outcome.at`:
`run_outcome_from_claim` only exposes terminal-to-terminal overrides.

For worker-backed steps, step start is the `work.claimed` claim. Agentless
steps have no `work.claimed`; their opening edge is the reconciler-owned
`step-run.state` transition to `working`. Step end comes from the terminal
`work.submitted` or `work.failed` claim for worker-backed steps, or the terminal
`step-run.state` transition for agentless steps. Use the claims'
`accepted_at_unix_ms` timestamps. The existing `STEP_TIMING_KINDS`,
`step_timing_events`, and `fold_step_clock_open` in
`crates/fractal-store/src/store.rs` cover both sources: the clock opens on
`work.claimed` or `step-run.state` with status `claimed`/`working`, and closes on
`work.submitted`, `work.failed`, `work.released`, or a `step-run.state` leaving
`claimed`/`working`. A release or other nonterminal closing edge stops an
execution interval; it is not step completion. The daemon serves the new
timing fields with the same bound as today's run enrichment: open runs plus
the latest finished run.

### Run ETA

Add an optional `eta` field to `MissionRunSummary`:

```text
eta?: {
  remaining_low_ms: integer,    // p25 of the remaining duration
  remaining_high_ms: integer,   // p75 of the remaining duration
  basis: { method: "per-step" | "critical-path", samples: integer },
  as_of: Timestamp
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

For each remaining step, the daemon takes p25 and p75 of that duration from
the completed runs of the same step path under the current definition hash.
If dependencies are declared in the mission definition's `step.dependencies`,
the daemon sums along the critical path, and `method` is `critical-path`.
These declarations supply the graph's dependency edges; runtime step
`blockers` are current `work.person-asked` subjects, not dependency edges.
Otherwise the daemon sums over the remaining steps, and `method` is `per-step`.

`samples` is the smallest sample count among the remaining steps. With 1 or 2
samples the value is still served, so a client can say "(1 prior run)".

## Contract surface and consumer coordination

### Open contract questions

1. Should `eta` sit on `MissionRunSummary` (as proposed) or on `Mission`?
2. Should the fields be always-on optional fields (as proposed, following
   recent mission timing fields), or gated by a capability version?

Defaults if unanswered: the recommended answers apply (`eta` on
`MissionRunSummary`; always-on optional fields); review happens after merge
per #4898.

### Implementation order

Each implementation PR is at most 300 lines:

1. Serve run and step `started_at`/`completed_at`.
2. Add the ETA fold and cache, and serve `eta`.

## Verification required for implementation

- Check timing against run creation and the `accepted_at_unix_ms` of the
  `mission-run.state` transition into `phase: "terminal"`, not a later
  terminal-to-terminal outcome override. Check worker-backed `work.claimed`
  and terminal work claims, plus agentless `step-run.state` working and
  terminal transitions. Distinguish interval closure from step completion,
  preserving the open-runs-plus-latest-finished-run bound.
- Check per-step and critical-path estimates using declared
  `step.dependencies`, not runtime person-work `blockers`; check the
  current-definition-hash sample set and exclusion of person-held time without
  excluding queueing or agent waits. Check the smallest sample count, including
  1 or 2 samples, and `null` for a zero-sample remaining step or a finished run.
- Check remaining durations and the publication-cut `as_of`, including the
  display of a paused or overdue run and omission by older daemons.
