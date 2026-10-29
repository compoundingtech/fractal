# What a run waits on

`st missions waiting-on RUN` answers why one run is not moving. It answers in milliseconds, so you
can ask it as often as you like.

```sh
st missions waiting-on mission-run/example/build/1
st missions waiting-on mission-run/example/build/1 --quiet 45m --json
```

The HTTP read is `GET /v1/mission-runs/{run}/waiting-on`, which takes `quiet_after_ms`.

## What it reads

It folds no claims and replays no history. It reads state Fractal already keeps current:

- The published work list: every open step of a current generation, with its effective state,
  holder, lease, last progress, declared wait, wake attempts and blockers. Its refresher refolds
  only the steps whose claims changed ([operational queries](operational-queries.md)).
- The published missions list: the open human gates and the agents roster that mission
  presentations read, which gives each seat's state.
- One primary-key read of the run and a range of its own `step_runs` rows. This names the steps
  that are finished or not yet ready. Its cost is the run's own steps.

`st missions show RUN` builds a presentation from each step's whole history. On a large store that
costs minutes of daemon CPU, so it is the wrong tool for "why is this stuck".

When a daemon publishes no lists (`ST3_PUBLISHED_LISTS=off`), the read refuses with HTTP 503
`waiting-on-unavailable`. It does not fall back to folding. While the work list is first being
prepared, it answers HTTP 503 `work-list-not-ready`, which is retryable.

## What each open step waits on

Each open step gets one kind. These five are causes:

| Kind | Meaning | Whom it waits on |
| --- | --- | --- |
| `ask` | An open `work ask`, or a human gate, waits for an answer. This includes the step that asked. | the person |
| `seat-down` | The seat that holds the step, or that would take it, is stopped, failed, suspended, down, retired or faulted. | the seat |
| `no-progress` | A seat holds the step and has recorded no progress (or claim) for longer than the quiet limit. | the holder |
| `unclaimed` | Ready for longer than the quiet limit, and nobody claims it. | the assignee or the queue holding it |
| `hold` | Blocked with a reason, in a declared `work wait`, in a retry backoff, or Fractal's wait for another run. | outside the fleet |

The rest are not causes:

| Kind | Meaning |
| --- | --- |
| `working` | Progress within the quiet limit. |
| `verifying` | Submitted, and Fractal checks its gates. |
| `ready` | Ready within the quiet limit. |
| `queued` | Ready, waiting its turn behind other work on a busy seat. |
| `watching` | Fractal's keep-open step. |

The quiet limit is 15 minutes by default: the time Fractal waits before it nudges an idle holder.
`--quiet` changes it for one read.

## A run's verdict

- **stalled**: some step is `seat-down`, `no-progress` or `unclaimed`. This holds whatever else in
  the run still moves.
- **waiting**: nothing moves, and a step waits on an `ask` or a `hold`.
- **moving**: otherwise.
- **finished**: the run completed, failed or was cancelled.

The run's cause is its first open step's kind in the order of the cause table. The reason names
that step, whom it waits on, and for how long.

A run with no open step, only steps not yet ready (such as one waiting on `--after` before its
first step opens), is not in the work list: `st missions waiting-on` says it has no open step.
