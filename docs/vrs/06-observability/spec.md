# Observability specification

This document owns the mechanism behind [requirements](requirements.md): the crate stack, exporter
configuration, span roots, unit propagation, test strategy, and delivery order. Naming,
provenance, and span-label rules are referenced from the dotfiles context `observability` tree
(`01-conventions`); the six producer obligations from its `09-integration` spec. st2-side
obligations land here; registry/dashboard/census obligations are deferred cross-repo
(O11Y-R08).

## Crate stack

```
opentelemetry                   = "0.30"
opentelemetry_sdk               = { version = "0.30", features = ["logs"] }
opentelemetry-otlp              = { version = "0.30", default-features = false,
                                    features = ["http-json", "reqwest-blocking-client",
                                                "internal-logs", "logs"] }
# tracing facade (PR3) — spans and logs unify on one subscriber
tracing                         = "0.1"
tracing-subscriber              = "0.3"
tracing-opentelemetry           = "0.31"
opentelemetry-appender-tracing  = { version = "0.30",
                                    features = ["experimental_use_tracing_span_context"] }
```

The appender-tracing `experimental_use_tracing_span_context` feature is load-bearing: without
it, emitted log records do not receive the active tracing span's trace/span ids and every
record exports uncorrelated.

The otlp feature set is load-bearing, not stylistic:

- **No gRPC client.** The fleet pipeline is OTLP/HTTP JSON only (`otel-stack.md`); no gRPC clients
  anywhere.
- **Blocking reqwest client only.** With both `reqwest-client` and `reqwest-blocking-client`
  enabled (the defaults include blocking alongside async), the crate compiles but every runtime
  client-selection cfg arm requires *not*-having the other feature, so export fails with
  `NoHttpClient` at span-export time. Exactly one of the two must be enabled.
- **Blocking chosen over async** because st2 has no tokio reactor; the async batch exporter
  panicked without one. See the prototype evidence
  ([.experiments/2026-08-25-rust-to-otelite-capture.md](.experiments/2026-08-25-rust-to-otelite-capture.md)).
- **`internal-logs`** keeps exporter-internal errors observable instead of swallowed.

## Exporter and provider setup

One module, `src/telemetry.rs`, owns init and teardown via `Telemetry::init(unit)` /
`Telemetry::shutdown()`:

- **Endpoint**: none configured in code. The exporter resolves `OTEL_EXPORTER_OTLP_ENDPOINT` and
  related `OTEL_*` variables from the environment automatically. Unset → no SDK provider or
  exporter is built (R02); since PR3 the human-readable stderr `tracing` layer still runs so
  diagnostics stay visible — see [Log bridge](#log-bridge-pr3-landed).
- **Protocol**: HTTP JSON (`http-json` + protobuf-free wire), batch exporter, targeting the local
  Alloy forwarder at `127.0.0.1:4318` by convention.
- **Resource**: `service.name` = `st2-<unit>` selected per entrypoint (below; `src/main.rs`
  passes `supervisor`, `hook`, or `cli`), `service.version` from `crate::version::machine_version`, and
  `host.name` from the existing host detection. The remaining R04 fleet attributes
  (`service.namespace`,
  `service.instance.id`, `sk.site`, `sk.role`, `deployment.environment.name`) are not set yet —
  tracked as an [open question](open-questions.md).
- **Flush/shutdown**: `force_flush` + global `shutdown` registered to run at process exit. The
  batch exporter buffers; without explicit flush at exit, tail spans of short-lived CLI runs are
  lost. This pairing is required for delivery, not optional.

### service.name values (R05)

| Process unit | `service.name` |
| --- | --- |
| Supervisor loop (`st2 up` daemon / systemd unit) | `st2-supervisor` |
| One-shot CLI invocations | `st2-cli` |
| Hook executions (`st2 driver claude-observe`; other hook surfaces not instrumented yet) | `st2-hook` |
| Claude's status-line tee (`st2 driver claude-statusline`) | **none — no pipeline is built** |

**The one deliberate exemption, and the rule behind it.** A subcommand whose cadence is set by a
harness's refresh timer rather than by an operator or an event does not initialize the telemetry
pipeline at all (`Telemetry::local_only`). Claude's status-line tee is the only such surface
today: `refreshInterval: 5` makes it ~720 short-lived processes per hour per seat, and Claude
waits for each to exit, so the final collect-and-export at shutdown would sit in the render path.
Measured against a bound-but-never-accepting collector, a tee that builds a pipeline takes 5.0 s
on the path that logs and `claude-observe` takes 10.0 s, against 0.01–0.06 s with none
(`08-harness-context`, `DQ-C13`).

The rule is about **cadence, not about being a hook**: `claude-observe` is event-driven, is named
in the table above, and stays instrumented. Anything added to the exempt list needs the same
argument — a harness-driven repeat rate and no operation worth a span — not merely being a
hook-set script.

## Reconciliation trace hierarchy

Instrumented exclusively through the `tracing` facade (`tracing::info_span!` plus
`tracing-opentelemetry` status extension), one bounded trace represents one reconcile pass:

```
st2.reconcile_pass
├── st2.catalog.lock
├── st2.catalog.discover
├── st2.hooks.verify                 # only when a consumer requires hooks
├── st2.catalog.materialize
├── st2.runtime.observe              # omitted for an externally supplied snapshot
└── st2.reconcile.execute
```

`st2.reconcile_pass` remains the compatibility root at the supervisor-loop, one-shot catalog,
selected-task, and single-file-spec sites. Its `span.label` and `st2.reconcile.path` are the enum
`catalog | selected | spec`. Root outcome attributes are `st2.host`, `st2.crash_loops`,
`st2.unparked`, `st2.report.errors`, `st2.report.warnings`, `st2.reconcile.skipped`, and
`st2.result = pass | fail`; non-empty errors set OTel status `ERROR`. A deterministic INFO event
(target `st2`, message `reconcile pass complete`, `result = pass | fail`) closes every root so
log-based assertions need no fault injection.

| Span name | Parent | `span.label` | Operation boundary | Attributes and status | Path applicability |
| --- | --- | --- | --- | --- | --- |
| `st2.reconcile_pass` | none | `catalog` \| `selected` \| `spec` | One complete reconcile pass | Root attributes above; `ERROR` when the pass returns/collects an error | Catalog loop/once, selected task, spec loop/once |
| `st2.catalog.lock` | `st2.reconcile_pass` | `shared` | Shared catalog-authoring lock acquisition | `st2.result`; `ERROR` on acquisition failure | Catalog, selected |
| `st2.catalog.discover` | `st2.reconcile_pass` | `catalog` | Recursive desired-state snapshot | `st2.catalog.spec_count`, report warning/error counts, `st2.result`; `ERROR` when discovery reports errors even though the pass may continue | Catalog, selected |
| `st2.hooks.verify` | `st2.reconcile_pass` | `lifecycle hooks` | Required lifecycle-hook receipt/set verification | `st2.hooks.consumer = codex \| pi \| codex+pi`, `st2.result`; `ERROR` on verification failure | Catalog or selected, only when required |
| `st2.catalog.materialize` | `st2.reconcile_pass` | `catalog` \| `selected owner` | Aggregate catalog/selected-owner materialization call | Materialization failure and report warning/error counts, `st2.result`; `ERROR` when materialization reports errors | Catalog, selected |
| `st2.runtime.observe` | `st2.reconcile_pass` | `all sessions` | Authoritative `Runner::list_sessions` call | `st2.runtime.session_count`, `st2.result`; `ERROR` on list failure | Catalog, selected, spec; omitted by `_with_sessions` because that snapshot is external |
| `st2.reconcile.execute` | `st2.reconcile_pass` | `apply plan` | Aggregate mutation call around `execute_with_presentation_cursor` | Plan launch/GC/teardown counts, newly added report warning/error counts, `st2.result`; `ERROR` only when execution adds errors | Catalog, selected, spec |

Every first-party root and child has a non-empty `span.label`. The exporter-enabled
`AtomicBool` in `src/telemetry.rs` is the hierarchy gate; `tracing::enabled!` is insufficient
because the stderr formatter remains installed without an endpoint. When the tracer exporter is
unset, child constructors return before span construction, label handling, collection allocation,
or count inspection. All children are aggregates and trace volume is bounded by the table.

Attribute policy follows the central `01-conventions` contract:

| Attribute family | Value type | Cardinality | Privacy | Metric-label policy |
| --- | --- | --- | --- | --- |
| `span.label` | enum string | bounded | public | forbidden |
| `st2.reconcile.path`, `st2.result`, `st2.hooks.consumer` | enum string | tiny/bounded | public | spanmetrics-only |
| All `*_count`, `st2.crash_loops`, `st2.unparked`, `st2.report.errors`, `st2.report.warnings` | integer | bounded numeric | public | forbidden |
| `st2.reconcile.skipped` | boolean | tiny | public | spanmetrics-only |
| `st2.host` | string | bounded fleet identity | internal | forbidden |

No span or status description carries an id, filesystem path, selector, or error prose.

Explicitly rejected spans: pure reconcile planning, identity validation,
`compile_generated_tasks`, debounce, report absorption, wait/sleep, watcher callbacks, and
wrapper functions. Per-task and per-owner spans are also rejected from this hierarchy: they need
a separately specified hard detail budget. Provider-session lifecycles and exec sidecars remain
follow-up surfaces beyond the PR2 launch/reap counters.

### Native-driver diagnostic transitions (O11Y-R09)

`src/driver_diagnostic.rs` emits one `st2.driver.diagnostic` span plus its
correlated `st2 native driver diagnostic transition` INFO event only when a
typed failure tuple changes or recovers. `span.label` is the closed stage.
Span/event attributes are `st2.driver.stage`, `st2.driver.reason`,
`st2.driver.source`, `st2.driver.support`, and `st2.outcome`; the raw
`st2.driver.producer_version` is span/log-only. No agent, runtime, session, or
message id is needed on this transition, and no prompt/message/path value is
recorded.

The span's stage/reason/source/support/outcome attributes are the same closed
values used by the counter below. Versions and identities are specifically not
counter labels or `span.label`. With no trace exporter the span is not
constructed; with no meter provider the counter returns before touching its
instrument.

## Metrics (PR2)

Landed RED-minimal set per interview decision Q5; every label value comes from a bounded enum,
and identifiers never become metric labels (ids stay in span attributes). `src/metrics.rs` owns
the instruments; every record call early-outs unless a meter provider is installed.

| Instrument | Type | Labels |
| --- | --- | --- |
| `reconcile_passes_total` | counter | `result` = `pass` \| `fail` |
| `task_launches_total` | counter | `driver` = `codex` \| `claude` \| `opencode` \| `pi` \| `omp` \| `exec` \| `other` |
| `task_reaps_total` | counter | `driver` (same enum as launches) |
| `hook_invocations_total` | counter | `hook` = registry name (`claude-observe`), `event` = bounded Claude hook-event set, unknown → `other` |
| `message_deliveries_total` | counter | `result` = `pass` \| `fail` |
| `crash_loops_total` | counter | — |
| `driver_diagnostic_transitions_total` | counter | `stage`, `reason`, `source`, `support`, `outcome = failure | recovery` (all closed enums) |
| `resource_observe_requests_total` | counter | `outcome` = `accepted` \| `backpressured` \| `settledUnchanged` \| `settledChanged` \| `settledFailed` \| `absentBinding` \| `staleGeneration` \| `providerUnavailable` \| `other` |
| `resource_observe_dispatch_seconds` | histogram | — |
| `resource_observe_settle_seconds` | histogram | — |
| `reconcile_pass_duration_seconds` | histogram | — |
| `session_start_duration_seconds` | histogram | — |

All duration histograms share seconds-scale explicit bucket boundaries
`0.001`, `0.005`, `0.01`, `0.025`, `0.05`, `0.1`, `0.25`, `0.5`, `1`,
`2.5`, `5`, and `10` (`DURATION_BUCKET_BOUNDARIES` in `src/telemetry.rs`)
instead of the SDK's millisecond-tuned defaults. This keeps sub-second
reconcile passes, spawns, observe dispatches, and settlements distinguishable.
Observe metric statuses use the same camelCase durable-wire spelling; kebab-case
is reserved for human CLI text.

Scope notes: passes are counted at all three `st2.reconcile_pass` sites (catalog loop pass,
one-shot up, and the single-file spec path — `reconcile_pass_specs_with_sessions`, which now
emits the same root span shape); `fail` means the pass collected errors. Reaps count the
restart path in the launch loop, where driver context exists. Deliveries cover bus deliveries
onto a recipient inbox (`deliver_record`, send + retry paths); ding/native transport outcomes
are separate follow-ups. Hook invocations are observed at the single in-process application
point (`st2 driver claude-observe`); hook scripts the harnesses execute directly are not
visible to st2. The `driver` label is a closed enum resolved by precedence: `exec` task kind
first, then a typed driver declaration, then an observational argv/shell token heuristic
(alphanumeric tokens matched in launch order: `codex`, `claude`, `opencode`, `omp`, `pi`; anything
else → `other`). Because the heuristic inspects arbitrary user work, a hand-authored seat may
be labeled by what its command line merely mentions — the label is diagnostic only and never
influences reconcile decisions.

The meter provider shares PR1's plumbing: `Telemetry::init` installs an `SdkMeterProvider`
with a `PeriodicReader` + OTLP/HTTP-JSON metric exporter behind the same
`OTEL_EXPORTER_OTLP_ENDPOINT` guard and resource; unset → no provider and the global meter is
a silent no-op (R02 zero-overhead). `Telemetry::shutdown` force-flushes metric points alongside
spans so short-lived CLI runs deliver them.

## Log bridge (PR3, landed)

Resolved by interview decision Q6: the `tracing` facade unifies spans and logs on one
subscriber (`src/telemetry.rs`):

- **stderr fmt layer — always installed.** Human-readable lines keep today's diagnostics
  visible with or without an endpoint. This is a deliberate deviation from PR1's literal
  zero-output unset-endpoint behavior: migrated `tracing` sites must not go silent. Level
  filtering defaults to INFO; `RUST_LOG` overrides.
- **Span layer** — `tracing-opentelemetry` exports spans through the existing tracer provider.
- **Log bridge** — `opentelemetry-appender-tracing` exports events through a new SDK logger
  provider sharing the endpoint, HTTP-JSON protocol, blocking client, and resource. With the
  `experimental_use_tracing_span_context` feature, records emitted inside a span carry its
  trace/span ids.

`Telemetry::shutdown` force-flushes and shuts down logger, meter, and tracer providers together.

Emission-site migration rule: non-user-facing diagnostics (`eprintln!` warn/error paths in
crash-loop handling, park-channel setup, catalog watching, ding transport ambiguity, driver
session degradation) became `tracing::warn!`/`error!` with unchanged message text. USER-FACING
CLI OUTPUT STAYS `println!`/`eprintln!`: command results (`installed`, boot reports, `ls`
tables), lock banners, and validation reports are interfaces, not diagnostics.

## Systemd unit propagation

`src/service.rs` builds the supervisor unit and serializes the operator's `OTEL_*` environment
into `Environment=` lines alongside the existing `PATH`/`PTY_ROOT` serialization, so
`st2 up --install-unit` preserves R02 (ambient endpoint) under systemd. Unit tests in
`service.rs` extend the existing serialization assertions.

## Testing strategy

- **Integration tests** (`tests/otel_export.rs`, cargo integration tests): the receiver is a
  prebuilt `otelite` binary passed by path via `ST2_OTELITE_BIN` (the effect-utils flake package
  output; gate wiring supplies it, and a gate run hard-fails without it unless
  `ST2_ALLOW_OTEL_SKIP=1` explicitly allows a local skip). Each test spawns
  `otelite capture` on an ephemeral port (`--http-port 0`), points the binary under test at it
  via `OTEL_EXPORTER_OTLP_ENDPOINT`, drives one command, then stops the receiver by closing its
  stdin — EOF flushes the capture to disk — and asserts on the captured traces (span names,
  resource attributes), metrics (PR2), and log records (PR3: the deterministic
  `reconcile pass complete` INFO record must carry the `st2.reconcile_pass` span's trace/span
  ids, proving tracing→OTel correlation end to end). Precedent: dotfiles op-proxy tests use
  `captureEnvTrace`; dotfiles branchy checks consume
  `effect-utils.packages.<system>.otelite`.
  - Caveat baked into harness design: `otelite capture` treats stdin EOF as termination, so the
    harness closes stdin deliberately as the stop signal rather than leaking `/dev/null`.
- **No-op proof**: a test asserts that with `OTEL_EXPORTER_OTLP_ENDPOINT` unset, the command
  completes normally with no export activity — guarding R02.
- **Flake check wiring**: the check pulls effect-utils' `otelite` package output, mirroring the
  branchy-check pattern, so CI proves R03 end-to-end without network access to dev3.

## Delivery: gh stack of three PRs

1. **PR1 — traces.** SDK init, OTLP/HTTP-JSON exporter with the exact feature set above, resource
   attributes, trace roots, unit `OTEL_*` propagation, otelite-based integration tests and flake
   check wiring, plus this VRS tree.
2. **PR2 — metrics.** Metric set finalized per open questions; shares provider/exporter/resource
   plumbing from PR1; otelite assertions extended to metrics.
3. **PR3 — log bridge.** Landed per Q6: `tracing` facade adopted, spans and logs on one
   subscriber; correlated diagnostics migrated; otelite assertions extended to logs.

Each PR lands CI-green independently; PR2/PR3 depend on PR1's plumbing only.

## st observability

The current daemon, CLI, replication worker and hook-exporter reference is
[st observability](../../st3/observability.md).

### Fleet work flow metrics (O11Y-R18)

`current open steps -> reader-only snapshot -> shared 60-second cache -> daemon OTLP`

| Instrument | Type | Unit | Labels | Source |
| --- | --- | --- | --- | --- |
| `st.work.items` | gauge | `{item}` | `st.work.state` | Current-generation nonterminal step projection |
| `st.work.oldest_age` | gauge | `s` | `st.work.state` | Seconds since creation of the oldest item in each bucket; empty buckets are zero |
| `st.agents.with_work` | gauge | `{agent}` | none | Distinct agents with unexpired work leases |
| `st.agents.idle_with_work` | gauge | `{agent}` | none | Holders whose latest substantive replicated activity or claim start is more than 600 seconds old |

Only these four state labels are emitted:

| Projection state | Metric bucket |
| --- | --- |
| `ready` | `ready` |
| `claimed`, `working`, `verifying` | `claimed` |
| `waiting-person`, or `ready`/`pending` assigned to a person | `waiting_person` |
| `pending`, `blocked`, other nonterminal states | `blocked` |

An expired held lease becomes ready before bucketing. Terminal steps, terminal owning
runs/root runs/generations, and superseded generations are excluded. These are materialized
projection measurements, refreshed by the existing reconciler; they do not independently
re-run readiness or person-ask resolution. A blocked step with a valid agent lease counts
as a holder, even though its work bucket is blocked.

Substantive activity uses the existing last-activity evidence categories: `work.progress`,
`work.submitted`, sent/received `message.sent`, and current claim-incarnation
`harness.timeline` entries of type `message`, `content`, `tool_call`, or `tool_result`.
Renewals, heartbeats and usage do not reset the idle clock. Latest `work.claimed` acceptance
time (creation time if absent) starts that clock for a newly acquired lease. When an agent
holds several steps, its newest claim selects the timeline incarnation (ties use descending
incarnation); its newest substantive evidence wins. Owner-local timeline observations
are deliberately excluded: replicas must measure the same replicated evidence. Consequently,
unreplicated native activity is not visible to this signal; it measures replicated inactivity,
not proof that the native process is stopped.

All nodes report fleet-wide values from their replicated projection. Do not sum nodes.
After convergence `max()` yields one fleet count; during lag it is an upper envelope, not an
atomic fleet snapshot. Prometheus's standard OTLP name translation gives:

```promql
max by (st_work_state) (st_work_items)
max by (st_work_state) (st_work_oldest_age_seconds)
max(st_agents_idle_with_work) / max(st_agents_with_work)
```

Use the existing resource/`target_info` join for host attribution rather than adding a host
label. No subject, agent, work identifier, free text, process identifier, revision or PR is
a metric attribute. These four instruments publish exactly ten gauge points per daemon sample.

The SDK-owned blocking metric reader performs one reader-pool statement at most every
60 seconds, shared by every gauge callback; it never takes the writer or enters a Tokio
worker. It walks only `step_runs_open_index`, joins owners by primary key, and performs
one indexed latest-claim seek per open step plus four indexed latest-activity seeks per
distinct holder. Mission waits share this same statement and cache, adding three indexed
state/retry/carry timestamp seeks and three clock/dirty/pending projection seeks per step.
Including three owner joins, the hard point-lookup cap is `10W + 4H`, where `W` is current
open projected work and `H <= W` is distinct held agents, independent of completed history.
Snapshot memory is O(W + H); export callback output is constant size.
Failed refreshes omit samples rather than export stale values or fabricated zeros.
`flow_metrics_cost_is_independent_of_completed_history` records elapsed time and SQLite VM
work at two completed-history sizes and asserts bounded growth.

### Fleet person-attention metrics (O11Y-R18)

`canonical attention publication -> configured-person filter -> two bounded kinds -> OTLP`

| Instrument | Type | Unit | Labels |
| --- | --- | --- | --- |
| `st.attention.open` | gauge | `{item}` | `st.person`, `st.attention.kind` |
| `st.attention.oldest_age` | gauge | `s` | `st.person`, `st.attention.kind` |

The source is the same shared attention publication served by client attention windows,
not a second query or interpretation of work history. Only the daemon's configured person
subject is an eligible `st.person` value; no configured person means no attention series.
Labels are bounded by configured fleet people, with exactly `ask` and `update` kinds for
each exported person. Both buckets export zero when a valid publication has no matching
rows. Missing, withdrawn or stopped publications omit metrics; there is no full-fold fallback.

Rows must be canonical `state=open` attention for the configured `person_id`. A structured
`update` field selects `update`; every other open attention item selects `ask`. Titles,
details, subject names and urgency are never inspected or exported as attributes.
Oldest age is seconds since the earliest matching `requested_at`; empty buckets are zero.

An unread update about a cancelled mission still counts as open when its own attention
row is open. Cancelling the subject it reports on does not mean the person has read the
notice. Reading/dismissing the update, or cancellation of the update's own person-step,
removes it through the existing canonical publication filters.

Each metric callback makes one O(A) pass over the already-published open attention rows,
where A is current published attention, with O(1) additional aggregation memory and zero
SQL statements. Both gauges together make two such passes and publish four points for the
configured person. Reading keeps the existing attention refresher active; wake requests
coalesce in that refresher. No metric export walks completed work or attention history.
The focused canonical-publication test asserts zero SQLite VM work for aggregation.

Fleet replicas report the same person's fleet-wide attention rather than node-owned subsets:

```promql
max by (st_person, st_attention_kind) (st_attention_open)
max by (st_person, st_attention_kind) (st_attention_oldest_age_seconds)
```

Use the existing resource/`target_info` join for host attribution. During replication or
publication lag, max is an upper envelope; it is exact after convergence.

### Fleet mission flow metrics (O11Y-R18)

```text
current work + clean projected clocks -> shared work snapshot -> wait gauges
new local terminal receipts          -> bounded reader task  -> lead-time histogram
```

| Instrument | Type | Unit | Labels | Source |
| --- | --- | --- | --- | --- |
| `st.mission.run.lead_time` | histogram | `s` | `st.mission.run.outcome` = `succeeded`, `failed`, `cancelled` | First locally authored terminal-state receipt minus run creation |
| `st.mission.steps.waiting` | gauge | `{step}` | `st.mission.wait_reason` = `human`, `dependency`, `infrastructure` | Current open work and clean current-attempt clock projection |
| `st.mission.steps.oldest_wait` | gauge | `s` | same bounded wait reasons | Oldest start among the same waiting steps |
| `st.mission.steps.wait_unknown` | gauge | `{step}` | none | Working/claimed steps whose current clock is absent, pending, dirty, or has the wrong attempt |

Wait reasons are a projection mapping, not an interpretation of titles or arbitrary summaries:

| Evidence | Reason | Wait start |
| --- | --- | --- |
| `waiting-person` | human | Latest state transition |
| `ready` or `pending`, assigned to a person | human | Readiness activation, or state transition if not activated |
| Other `pending` steps, including scheduled backoff | dependency | Latest state/retry/carry transition |
| `blocked` on the scheduler's `step baseline` predicate | dependency | Latest state transition |
| Agentless `working` step with the scheduler's `waiting for` mission-run dependency | dependency | Latest state transition |
| Active declared work wait | dependency | Clock projection `pause_started` |
| Other `blocked` or `orphaned` steps | infrastructure | Latest state transition |

Initial state has creation time as its fallback. State-transition time is the greatest
acceptance time among the latest `step-run.state`, `step-run.retried`, and `step-run.carried`
receipts, read by indexed seeks. Lease renewals do not shorten a wait's measured age.
A declared wait remains active through `paused_until` inclusive, matching the clock reducer.
The read joins `local_step_clocks` by subject and current attempt and rejects rows with
`local_step_clock_pending` or `local_step_clock_dirty` evidence. It reads raw pause fields
without invoking the reducer or its history fallback. A missing pause start for an active
declared wait is unknown as well.

When any required clock is unknown, reason counts and oldest-wait gauges are omitted rather
than publish incomplete totals. The unlabeled unknown gauge remains visible and reports how
many steps are omitted. These gauges share the work sampler's single reader statement and
60-second cache; they add no separate export query or async-runtime read.

The lead-time sampler is a daemon-owned task on the existing runtime, with SQLite reads in
`spawn_blocking` through the reader pool only. At startup its first tick records the current
store frontier without backfilling history. Every subsequent 60-second tick reads at most
1024 new `mission-run.state` receipts through `claims_kind_index` and advances only through
the returned batch. A larger backlog continues on following ticks; no tail is skipped.
The query joins creation times by run primary key, has O(1024) maximum rows and O(1024)
temporary memory per tick, and is independent of total run history. Read failure preserves
the cursor and retries next tick; missing run projection also fails the batch rather than
drop a completion.

Only the authoring daemon records receipts with `phase=terminal`; replication copies do
not record, and later outcome corrections without that phase do not record again. Outcomes
map `completed` to `succeeded`, `failed` to `failed`, and `cancelled` to `cancelled`.
This covers ordinary completion and descendant/direct cancellation paths without adding
writer hooks or changing lifecycle behavior. Lead time is receipt acceptance minus creation,
clamped at zero for clock skew, in seconds. Explicit buckets are
60, 300, 900, 1800, 3600, 7200, 14400, 28800, 86400, and 172800 seconds, plus infinity.
These are process metrics, not a durable outcome ledger: a process restart begins a new
frontier, so a completion during its downtime is not backfilled. The task aborts when its
daemon-owned handle drops.

Wait gauges are fleet-wide replicas and use max. Lead-time buckets are owner-emitted
events, so sum per-node rates rather than max:

```promql
max by (st_mission_wait_reason) (st_mission_steps_waiting)
  unless on() (max(st_mission_steps_wait_unknown) > 0)
max by (st_mission_wait_reason) (st_mission_steps_oldest_wait_seconds)
  unless on() (max(st_mission_steps_wait_unknown) > 0)
max(st_mission_steps_wait_unknown)
histogram_quantile(0.5, sum by (le) (rate(st_mission_run_lead_time_seconds_bucket[1d])))
histogram_quantile(0.9, sum by (le) (rate(st_mission_run_lead_time_seconds_bucket[1d])))
```

The unknown guard also hides previously scraped reason values while a fresh sample is
unavailable. Add `st_mission_run_outcome` to the histogram's `sum by` for per-outcome quantiles.
No mission, run, step, agent, subject or free-text value is a metric label.
