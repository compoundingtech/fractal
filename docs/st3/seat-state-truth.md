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

## A custom shell's first work claim

Reconcile completion of a driver-less launch still records `starting`;
spawning alone is not readiness. A locally bound shell may establish running
authority with its first explicitly incarnation-fenced work claim, even before any
reconcile pass applies the launch completion. The launch worker durably records the
verified operation, incarnation, PID and kernel process-start identity when spawn
finishes. Admission checks that exact current subject/runtime operation and freshly
checks the process generation. Until reconcile applies completion, the latest runtime
observation must still be the one captured before spawn, or already name this starting
incarnation. A newer unrelated observation cannot be overwritten.
The original start and control requests must also remain retained and unchanged;
repairing or retracting them before application refuses the bootstrap claim.

The custom shell need not supply `--incarnation`: inside a native PTY, the work CLI
uses `PTY_SESSION`, `PTY_ROOT` and the launch-exported start operation to resolve
its exact subject/operation incarnation from the local registry before consulting
the possibly stale status projection. A missing exact local match does not borrow
the predecessor's status incarnation.

Running authority and the work claim commit in one writer savepoint; a refused claim
publishes neither. Later reconciliation cannot regress that incarnation to `starting`.

Missing process-birth evidence, a dead or reused PID, a different operation, subject or
incarnation keeps the existing refusal. Unbound callers and omitted incarnation fences
do not use this path. Native-driver binding and readiness rules are unchanged. Platforms
without exact process-generation liveness checks cannot use this bootstrap path.

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
`harness-model-drift`) naming declared vs live model and since when. The detector
itself never stops or parks a seat; the separately gated bridge below owns return
requests. The fault recovers when usage returns to the declared model or the
declaration adopts the live model. A changed declared/live pair or incarnation
starts a newly fenced onset rather than inheriting an unrelated clock.
The fault is owned by the seat's declaring agent and is never a person's item. A
person instead sees one `model-drift` attention item over every drifting agent they own
(`fractal alerts ls`, under updates, and Now's today group): "3 agents declared X run Y since
09:14Z (oldest agent/...)". It is derived from the current faults, so it follows them and is
gone once no agent drifts. The card also carries `declared_model` and
`model_drift`: `state` is `at-declared`, `drifted`, `unknown`, `stale`, or
`uncomparable`. Missing current-incarnation model evidence is `unknown`, never OK;
untimestamped, future-dated, or expired evidence is `stale`. Freshness uses the
shared harness observation window (five minutes plus thirty seconds), reapplied
at read time even when no claims change. `drift_since` comes from the reconciler's
matching incarnation-fenced episode, not the latest sample time; it is null before
the detector establishes an episode. `drift_age_seconds` is present only for
fresh, established drift. `fallback_reason` is a sanitized cause classification
from a matching native-session fallback episode, never raw provider error text.
Unknown or unmatched cause remains null/unknown, not an inferred transient.
`return_attempts` counts admitted bridge restart requests in the current runtime
incarnation; zero means no bridge request, not that OMP never attempted recovery.
Without an incarnation the count is null. `fractal agents ls/tree/show` prints
declared and live models, drift state, known onset, cause, and attempts together. JSON retains
`.live_model` for existing scripts. Model names compare after the pricing table's
canonicalization (provider prefix, bracketed variant, date snapshot stripped). A Claude alias
(`opus`, `sonnet`, `haiku`) matches any API ID of its family, and a declaration that is neither
an alias nor a priced API ID is not compared, so it never raises the fault.

### Interim idle return bridge

Until an OMP release with native next-turn return is pinned, the reconciler's
model-return bridge defaults **on for all declared seats**. It requests the same
supported restart-when-idle operation as `fractal agents restart`, never a forced
stop or a provider probe. A request requires fresh, comparable drift for **more
than ten minutes**, a settled idle turn with quiescent input/output, positively
classified transient cause, no bridge admission in the preceding hour, and a
free per-host return slot. The source selector must still match the declaration,
and the fallback selector must match the actually observed live model.

Quota and authentication fallbacks are explained drift, not restart candidates.
Unknown causes raise attention and never authorize restarts. The OMP channel
observes root fallback decisions and native model-change identity; a manual model
selection or a new native session invalidates the old cause. Across a fallback
chain, quota/auth/unknown evidence cannot be overwritten by a later transient
failure. Hooks use only public extension events, never OMP's internal imports.
Bounded redacted error facts cross the owned channel; the Rust decoder classifies
them before writing sanitized cause claims. Quota/rate-limit and auth evidence
take precedence over transient markers. Positive 5xx, socket-close or timeout
evidence is transient; unrecognized facts remain unknown. Raw provider text is
not retained in model-return claims.

Admissions are durable reasoned claims, fenced by episode, incarnation and
declaration. Repeated passes/replayed events do not admit another restart. The
hourly budget survives new incarnations and daemon replacement. Eligibility is
checked again before release; resolved drift, busy state or changed declarations
cancel the pending return rather than allowing a stale restart.

Policy overrides merge field by field: fleet → explicitly mapped persona →
agent config → agent declaration. Persona mapping is explicit configuration;
render-only persona metadata grants no runtime authority. For example, TOML:

```toml
[model_return.fleet]
enabled = true
max_drift = 600 # seconds; cannot lower the ten-minute floor
max_per_hour = 1 # cannot exceed one; zero disables admissions

[model_return.personas.review]
max_drift = 900

[model_return.agent_personas]
"agent/example.worker" = "review"

[model_return.agents."agent/example.worker"]
enabled = false
```

The selected agent declaration may override those values:

```kdl
model-return {
  enabled false
  max-drift 900
  max-per-hour 0
}
```

An explicit `false` or `0` wins over inherited enforcement. Disable this interim
bridge once the native-return OMP release is pinned; observability remains useful
independently of the bridge. This source change performs no fleet rollout.

### Drift metrics

Model telemetry follows the existing `st.*` OpenTelemetry instruments and daemon
resource identity. The reconciler publishes one complete locally owned declared
roster cut; collection callbacks never read claims or probe providers. The
`st.agent.model.state` gauge is one-hot across the five states, with `st.agent`
and `st.model.state` attributes. `st.agent.model.drift.age_seconds` is a seconds gauge
only for fresh, established drift. Missing onset or stale evidence omits the age
point, not a fabricated zero. Freshness is recomputed during collection even
without new claims; retired/remote rows disappear with the next complete cut.

The existing OTLP-to-Prometheus translation exposes these as
`st_agent_model_state{st_agent="agent/example.worker",st_model_state="drifted"}`
and `st_agent_model_drift_age_seconds{st_agent="agent/example.worker"}`.
Alert on `st_agent_model_drift_age_seconds > 600` for two minutes; separately
alert on `st_agent_model_state{st_model_state=~"unknown|stale"} == 1` for ten
minutes. Healthy coverage counts only `at-declared` over the entire declared
roster, not over observed models. Target/exporter absence requires the existing
inventory availability alert. Model selectors, session IDs and raw errors are
never metric labels. These gauges are queried directly, never with `rate()`.

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
