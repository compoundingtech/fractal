# What a run waits on

`fractal missions waiting-on RUN` answers why one run is not moving. `fractal missions blocked` lists every
blocked run in the fleet, one line each, grouped by cause. Both answer in milliseconds, so you can
ask them as often as you like.

```sh
fractal missions waiting-on mission-run/example/build/1
fractal missions waiting-on mission-run/example/build/1 --quiet 45m --json
fractal missions blocked
fractal missions blocked --quiet 2h
```

The HTTP reads are `GET /v1/mission-runs/{run}/waiting-on` and `GET /v1/blocked-runs`. Both take
`quiet_after_ms`.

## What they read

Neither read folds claims or replays history. They read state Fractal already keeps current:

- The published work list: every open step of a current generation, with its effective state,
  holder, lease, last progress, declared wait, wake attempts and blockers, and for a program step
  its innermost open journal key, read from its newest 64 journal entries when the row is folded.
  Its refresher refolds only the steps whose claims changed ([operational queries](operational-queries.md)),
  and each journal entry is a claim on its step.
- The published missions list: the open human gates and the agents roster that mission
  presentations read, which gives each seat's state.
- For one run, one primary-key read of the run and a range of its own `step_runs` rows. This
  names the steps that are finished or not yet ready. Its cost is the run's own steps.
- For each open agentless predicate step, one indexed lookup of its pinned definition, with
  nested paths resolved in that definition, and at most four read-only actual-cache peeks.
  Each peek checks freshness with a covering seek on `claims_subject_index`, reading no claim
  body. Cold or stale entries are unavailable; the peek never fills the cache or folds history.
  These reads do not run gates, refresh observations, walk launch lineage or write diagnostics.
  Nested work rows omit gate-kind metadata; waiting-on resolves predicate eligibility and gates
  from the pinned definition without changing the work row's client representation.

`fractal missions show RUN` builds a presentation from each step's whole history. On a large store that
costs minutes of daemon CPU, so it is the wrong tool for "why is this stuck".

When a daemon publishes no lists (`ST3_PUBLISHED_LISTS=off`), both reads refuse with HTTP 503
`waiting-on-unavailable`. They do not fall back to folding. While the work list is first being
prepared, they answer HTTP 503 `work-list-not-ready`, which is retryable.

## What each open step waits on

Each open step gets one kind. These five are causes:

| Kind | Meaning | Whom it waits on |
| --- | --- | --- |
| `ask` | An open `work ask`, or a human gate, waits for an answer. This includes the step that asked. | the person |
| `seat-down` | The seat that holds the step, or that would take it, is stopped, failed, suspended, down, retired or faulted. | the seat |
| `no-progress` | A seat holds the step and has recorded no progress (or claim) for longer than the quiet limit, or a program step's journal has no new entry within its `stalled-after`. | the holder (Fractal for a program) |
| `unclaimed` | Ready for longer than the quiet limit, and nobody claims it. | the assignee or the queue holding it |
| `hold` | Blocked with a reason, in a declared `work wait`, in a retry backoff, or Fractal's wait for another run. | outside the fleet |

The rest are not causes:

| Kind | Meaning |
| --- | --- |
| `working` | Progress within the quiet limit. |
| `program` | A program step whose journal moved within its `stalled-after`. The detail names its innermost open key: the most recently started key without a result, and the child run or ask it waits on. A program's ask is an `ask`, named with its key. |
| `verifying` | Submitted, and Fractal checks its gates. |
| `ready` | Ready within the quiet limit. |
| `queued` | Ready, waiting its turn behind other work on a busy seat. |
| `watching` | Fractal's keep-open step. |

The quiet limit is 30 minutes by default: the time Fractal waits before it nudges an idle holder.
`--quiet` changes it for one read.

An agentless predicate step with a known false field gate is a `hold`, not active verification.
Its detail names the gate, subject, field, observed value and expectation. Scalars are limited
to 80 characters with explicit truncation; arrays and objects show only their type and size.
The wait starts at the step's execution start and does not become a stall merely because time
passes. Field predicates are checked in declaration order, with at most four cache peeks.
Cold or stale observations, missing fields, runtime interpolation, non-field predicates, exec
subjects, effect subjects, pinned resources, or no false predicate within that bound retain the
existing `verifying` description. Terminal service-exec checks never run on this read path.

Named and default stall reporters use the same intentional-wait classification. Silence alone
does not report a run when every unfinished step is a known predicate hold, an unanswered human
gate, a native person ask, a blocker, a retry backoff, or a live holder's declared wait. Mixed runs
still report unexplained silence; cold or unknown evidence preserves the previous behavior.
Child-run waits remain conservative: waiting for a child does not prove its work is healthy.
The existing one-notice-per-silence identity and terminal-event reports are unchanged.

## A run's verdict

- **stalled**: some step is `seat-down`, `no-progress` or `unclaimed`. This holds whatever else in
  the run still moves.
- **waiting**: nothing moves, and a step waits on an `ask` or a `hold`.
- **moving**: otherwise.
- **finished**: the run completed, failed or was cancelled.

The run's cause is its first open step's kind in the order of the cause table. The reason names
that step, whom it waits on, and for how long.

`fractal missions blocked` lists every run with open work whose verdict is stalled or waiting. A stalled
run is listed under its stuck step's cause; a waiting one under its first cause. Within a cause,
the longest blocked run comes first.

A run with no open step, only steps not yet ready (such as one waiting on `--after` before its
first step opens), is not in the work list. `fractal missions blocked` does not list it, and
`fractal missions waiting-on` says it has no open step.
