# Seat status truth

One place in Fractal decides what a seat's status is: the agent card that `overlay_agent_resources`
(`crates/fractal-api-core/src/api.rs`) finishes on every read, with the read's own clock. Fractal TUI, the phone app,
the web app and `fractal agents show` render that card. None of them re-derives a state from the raw
fields, so they cannot disagree.

## States that need a clock

| Card | Rule |
| --- | --- |
| `state: running` | The harness reported `ready`, `idle` or `working` inside the shared `HARNESS_OBSERVATION_FRESHNESS` window (five-minute refresh plus thirty seconds), even when the runtime row still says `pending`. A seat can outlive its row. A seat whose runtime row is stuck and whose last report is an idle one older than that window but newer than a heartbeat (11 minutes) reads `waiting`, like any other live idle seat; it is never `down`. |
| `state: desired` / `starting` | A declared seat inside its launch window (two minutes from its last runtime claim, or from its declaration if it never reported). |
| `state: down`, `down_since` | A declared seat with no running session and no harness report within a heartbeat (11 minutes; a driver at an empty prompt reports every five minutes) once the launch window has passed. `down_since` is when it last reported after its launch, or when it began waiting if it never has; the report time is the driver's own clock value, so a driver with a skewed clock moves `down_since` by the same amount. Never shown as starting, working or idle. |
| `current_work[].holder_down_since` | Set on every step a `down` seat holds: the step's progress and activity were written by a holder that has no session. |
| `harness_state: working`, `observation: stale` | A turn can run for hours without a new harness claim, so a `working` report is kept until the harness says otherwise. `silent_since` already says how long the seat has been quiet. |
| `harness_state: indeterminate`, `observation: stale` | Only a `ready` or `idle` report lapses after the window: Fractal has not heard from the harness. Fractal TUI and the phone app show this as idle for a reachable seat; the card does not claim it. |

## What `down` means, and what it does not

`down` is a statement about Fractal's session for the seat: a declaration exists, the runtime row is not
running, and no harness has reported lately. It is not proof that no process exists. A pty daemon
and its driver can outlive a runtime row stuck at `pending`; while such a seat reports, the card
says `running`; when it stops reporting, the card says `down` and `fractal terminals screen` finds no
session for it too. Finding the process and relaunching a seat that declares `restart always` belong to
the launch and reconcile path, not to the label.

## A seat Fractal cannot place on a host

When two hosts both claim a seat's runtime, Fractal cannot say which one runs it, and `fractal terminals`
refuses with `runtime-authority-indeterminate`. The card keeps `state: waiting` and
`reachability: indeterminate`, and its `reason` reads "Fractal cannot tell which host runs this agent; its
processes may be running" unless the harness gave a reason of its own. The card does not call the
seat running, stopped or down, and the conflicting claims stay as they are. `fractal agents show` prints
the same sentence in place of "Still starting". The apps keep their present word for the state until
they learn to print the reason.

## A seat that dies mid-turn

Keeping a stale `working` report means the card never lapses it just because its observation ages.
What ends it is the runtime row: the reconciler on the seat's host observes its pty and driver on every pass
and writes `exited` or `vanished`, after which the card reads `stopped` or `failed`, not
`running` (`a_seat_whose_runtime_exits_mid_turn_stops_reading_working_with_its_row`). If that
reconciler is not running, or never writes the row, the card keeps `running`/`working` and
`silent_since` is the only signal; `down` covers only seats whose row never reached `running`.
A `down` card also reports `harness_state: indeterminate`, so the two fields never disagree.

## A seat running on a fallback model

The card carries `live_model`: the model the seat's own session last reported for the card's
incarnation (`harness.usage` `context_occupancy` or `session_cumulative`: Claude's status line,
the omp/pi channel, OpenCode), or null when it reported none. Response spend never counts: it
includes subagents' responses under their own models. A driver whose session reports no model
(Codex) has a null `live_model` and no drift fault. The declaration still names the wanted model; the two
differ while the harness silently fell back (omp down its ladder on a 429). The reconciler
raises a non-blocking per-seat fault (`Seat runs on a fallback model`, condition
`harness-model-drift`) naming declared vs live model and since when: it never stops, parks,
or crash-loops the seat, and recovers when usage returns to the declared model or the
declaration adopts the live model. If the live model moves to another undeclared model while
drifted, the fault is recovered and raised again naming the new model, keeping its first
`since`. The fault is owned by the seat's declaring agent and is never a person's item. A
person instead sees one `model-drift` attention item over every drifting agent they own
(`fractal alerts ls`, under updates, and Now's today group): "3 agents declared X run Y since
09:14Z (oldest agent/...)". It is derived from the current faults, so it follows them and is
gone once no agent drifts. `fractal agents show`
prints `LIVE MODEL`, and a step script reads `.live_model` from `agents show --json` to flag
what a one-shot reviewer actually ran on. Model names compare after the pricing table's
canonicalization (provider prefix, bracketed variant, date snapshot stripped). A Claude alias
(`opus`, `sonnet`, `haiku`) matches any API ID of its family, and a declaration that is neither
an alias nor a priced API ID is not compared, so it never raises the fault.

## Example

At audit time a seat `example/builder` showed `desired` with a `working` step and fresh-looking
`last_activity_at`, while `st terminals screen` found no running session. After the change its card
reads:

    STATE        down · no session since 2026-10-12T12:31:00.000Z
    CURRENT STEP Build the parser · working · held by a seat with no session since 2026-10-12T12:31:00.000Z

Proof: `api::client_v0::tests::a_declared_seat_with_no_session_past_its_launch_window_is_down_since_it_began_waiting`,
`…a_fresh_harness_report_beats_a_runtime_row_stuck_pending`,
`…a_stale_working_report_stays_working_while_a_stale_idle_one_lapses`, and the Fractal TUI, phone and web
tests that render `down`.
