# Objectives

An objective declares something Fractal measures, with an explicit target and optional response
when it is missed, or an explicitly measured-only native indicator. There is one concept and one KDL kind, `objective`. Fractal ships its
own objectives (request latency, the store, CPU) and a fleet declares more with `fractal apply`,
like agents, missions and accounts. Both are read the same way: `fractal slo ls`, `fractal slo
explain ID` and `fractal slo chart ID`.

## The `objective` kind

```kdl
version 2

objective "ops/change-rollout" {
  about "A change, from its author marking it ready to every member that serves it"
  indicator "interval" {
    subject-prefix "custom/acme/change/"
    start "custom.acme.change-ready" at="ready_at"
    expect "custom.acme.change-merged" set="hosts" at="merged_at"
    stop "custom.acme.change-applied" member="host" at="applied_at"
    cancel "custom.acme.change-abandoned"
    label "class"
    exclude "exclusion" "deferred-launch" "deferred-runtime" "deferred-workspace"
  }
  window "7d" min-samples=20
  target "p50" within="60s"
  target "p95" within="5m"
  on-breach "fault" owner="agent/ops/release-manager" backup="agent/ops/observability" condition="change-rollout-slo-missed"
}
```

- The name is the objective's id and its subject is `objective/NAME`. It is lowercase words joined
  by `-`, in segments joined by `/`. A declared objective is named `NAMESPACE/NAME` or deeper, like
  an agent; the objectives Fractal ships are one segment each, so the two never collide.
- `about` says what is measured, in one sentence.
- `indicator` says how a sample is measured. Each kind is below.
- `target STAT BOUND` bounds one statistic. `STAT` is what the indicator reports (`p50`, `p99`,
  `max`, `cores`, `per-hour`, ...) and the bound is `within=DURATION`, `max=NUMBER`, `min=NUMBER`
  or `min-percent=NUMBER`. Each indicator names its rows (table below). A percentile over claims or
  CI is the row `NAME/pNN` (`ops/change-rollout/p95`), one per target.
- `window` is the span an interval or count is measured over, and `min-samples` the samples an
  interval needs before it is met or missed; with fewer it is `no-data`.
- `budget` declares the bad-event or bad-covered-time allowance, horizon, evidence gates and
  paired burn windows for ratio and gauge indicators. Built-in telemetry may carry this policy
  alongside its existing diagnostic targets.
- `on-breach` is what happens when a row is missed. It is optional; without it a miss is shown and
  nothing is delivered.

A missing, unknown, repeated or nonsensical child is refused, never defaulted. An objective is a root
declaration; it cannot sit inside a mission.

Fault owners can be declared without a harness or command. Their declared `host` selects the
single member that appends the durable `message.sent`; delivery does not require a running
agent. Host-block placement, default placement and `host "local"` are resolved when the
declaration is published. Reapply older no-launch declarations that omitted an explicit host
to persist that resolved placement. Repeated reconciliation retains one message per episode;
a recovered objective that burns again opens a new episode and message.

```kdl
agent "ops/budget-owner" {
  host "monitor"
  name "Budget owner"
  description "Own service budget episodes without starting a runtime"
}
```

This inert top-level agent needs no task children. A run-owned agent still needs a launch;
invalid or conflicting explicitly declared launches are rejected as before.
Its agent card has no runtime or current session. Changing a previously launched root to an
inert declaration does not reuse historical runtime or harness observations as current state.

## Indicators

### Native telemetry

Fractal measures these itself, in memory, on every member. They are the objectives Fractal ships, in
[`slo/objectives.kdl`](../../slo/objectives.kdl), which is generated from
[`slo/objectives.kdl.genie.ts`](../../slo/objectives.kdl.genie.ts). A fleet cannot declare them:
each path, store series and CPU reading counts toward exactly one objective, and that is the one
the daemon was built with.

| Indicator | Measures | Targets | Rows | Windows |
|---|---|---|---|---|
| `latency` | Each `path`: a daemon route (`GET /v1/client/agents`), a stream's first snapshot (`stream agents`), a refresher's publication (`publish missions`) or a paired phone's report (`client/ios/connect`) | `p99 within=`; again with `scope="remote"` for the same reads served from another member | `NAME`, `NAME/remote` | 1m, 5m, 1h diagnostics; declared operational budget over 30d |
| `share` | A phone's live foreground time over its foreground time, at `path` | `live-share min-percent=` | `NAME` | 1m, 5m, 1h |
| `statement` | One SQL statement | `p99 within=`, `max within=` | `NAME` (the p99) | 1m, 5m, 1h |
| `transaction` | Read and write transactions and writer holds, with independent slow-event ratios | `max within=` diagnostic; declared operational allowance | `NAME/read`, `NAME/write`, `NAME/writer_hold` | 1m, 5m, 1h diagnostics; operational budget over 30d |
| `writer-hold` | Each writer hold and long read, by holder | `max within=` | `NAME` | 1m, 5m, 1h |
| `cpu` | The daemon and its replication workers over closed UTC minutes; children `idle-min-minutes` and `load-max-cores` | `cores max=`, `idle-cores max=` | `NAME`, `NAME/idle` | the hour |
| `database` | Covered time over either live-size or daily-growth bound | `live-gb max=`, `growth-mb-per-day max=`; declared operational allowance | `NAME` | hourly measurements; covered-time budget over 30d |
| `divergence` | Open projection divergence faults | `open max=` | `NAME`, once a divergence check has run | now |

The statement, transaction, writer-hold, CPU, database and divergence objectives are one each,
named as the table's rows say (`sql-statement`, `transaction`, `writer-hold`, `cpu`, `database`,
`projection-divergence`): `fractal doctor` and the OTLP export read them by these names.

Diagnostic aspirations retain their strict meanings: a p99 is missed in a window when more than
1% of its samples are over, and a max when any one is. An operational budget row's status instead
uses its slow-event or bad-covered-time allowance, retained coverage and paired burns. A `met`
operational row does not claim that its diagnostic p99 or max aspiration was met.

#### Organic terminal attachment and sparse reads

`terminal-attach` counts successful server-side first-screen observations, not a terminal
listing or viewer click-to-paint. Its exact keys are `GET /v1/client/terminals/{id}/screen`
and **singular** `stream terminal`. The collections socket records the latter only after
successfully sending a terminal screen. **Plural** `stream terminals` measures the terminal
list's initial snapshot and must never count as attachment evidence.

The dedicated local and remote terminal screen streams do not currently record that
first-screen latency. Their successful send boundaries are in `terminal_stream_socket`
and `remote_terminal_stream_socket` in `crates/fractal-api-client-v0/src/api/client_v0.rs`.
Consequently, absent attachment samples cannot establish that no viewer attached. Until
those producers are instrumented, the native objective covers only its instrumented organic
paths; the existing exact key mapping is not a substitute for a missing producer.

When no organic observations qualify in the last hour, `terminal-attach` and
`person-read-fresh` explicitly display **No qualifying events**. With no retained qualifying
events they remain `no-data`, with no remaining-budget verdict, even when observation-time
coverage is complete. Native operational rows expose `coverage` as the observed fraction
over their accounting horizon, or `null` when unavailable; their explanation states the
same coverage explicitly. Coverage of observation time does not prove every viewer or
attempt was instrumented. Missing coverage is unknown, never an invented zero or success.
An hour without new events does not erase a verdict supported by retained eligible history.
After a restart, an empty process-local diagnostic histogram is not evidence of an empty
hour: retained hourly events remain visible, with the diagnostic p99 explicitly unavailable
in the new process rather than mislabeled as no qualifying events.

Synthetic probes are not organic attachment or fresh-read traffic. Any future availability
probe must have its own separately named objective rather than filling these sparse rows.

### CPU populations and commissioning

Production `cpu` remains ≤1 core over **all 60 closed UTC minutes**, including daemon and
replication-worker process CPU. Unknown minutes never become zero or a shorter denominator.
The sampler requires the exact expected fresh worker cohort at both boundaries. In production's
singleton-worker topology, the newest reported process birth supersedes older births; the changed
minute stays unknown, but the following stable minute recovers without waiting for the old report
to age out. Equal-birth competing identities remain ambiguous, and a delayed older report cannot
revive a superseded worker. Multi-worker topologies retain the entire exact cohort check.
The CPU target in `/v1/client/request-latency` includes at most one hour of `minute_evidence`:
CPU/wall nanoseconds, expected worker count, idle/seat evidence,
busy reasons and a bounded `missing_reason` (`worker-missing`, `worker-stale`,
`worker-cohort-extra`, `worker-topology-changed`, `worker-incarnation-changed`,
`worker-counter-regressed`, `daemon-counter-regressed`). An omitted minute is an unobserved
boundary, not a known zero. These are current local diagnostics, not durable historical
attribution: an older receipt without them cannot prove which condition caused its unknown CPU.

Production `cpu/idle` remains conditional: ≤0.5 core over at least ten confirmed idle minutes
in a complete hour. `not eligible: busy` means productive seat activity alone rules out ten
idle minutes; the row stays no-data with no error-budget credit. Missing CPU/seat evidence
remains distinct and visible, even when reasons overlap. A live session's same-incarnation
runtime heartbeat is not productive work: matching harness `ready`, `idle`, `blocked` or
`ended` with no background jobs can prove it quiet only while the source `observed_at_ms`
is fresh under the existing harness stale/future-skew bounds. Starts, new incarnations,
work, and missing or expired matching harness observations cannot. Receiving an old report
again does not refresh its source evidence.
Freshness is checked before each admitted replacement as well as at minute boundaries;
a later fresh observation cannot erase an earlier intra-minute gap of expired evidence.

The separately labeled **representative isolated controlled quiet reference** is commissioned
with the real daemon/worker sampler, never synthetic CPU values and never by pausing live seats.
Prepare a network/state-isolated member using the same binary and configuration as the reference
member, with instance-specific isolation supplied as launch overrides. Retain the launch,
network/state isolation and configuration provenance in an operator receipt. Do not connect its
worker to production peers. Then run:

```sh
python3 scripts/commission-cpu-quiet-reference \
  --socket /isolated/member.sock --reference-socket /reference/member.sock \
  --config /isolated/config.toml --reference-config /reference/config.toml \
  --binary /isolated/fractal --reference-binary /reference/fractal \
  --expected-workers 1 --isolation-evidence '<launch-and-isolation-receipt>'
```

The command only reads both members; it requires equal nonempty `build.commit` strings in
both health responses, checks byte-identical
binary/configuration inputs and reference worker topology, then collects exactly ten **new consecutive**
closed minutes and waits for seat settlement. The operator's isolation receipt, not the socket
name, establishes that launch overrides really isolate the member and that these are the files
actually used. Unknown CPU, missing boundaries, busy/unsettled seats, topology changes or a
changed build cannot pass. The measured verdict is published only after the final health read
confirms the same nonempty build commit; a failed final health check leaves the receipt `no-data`.
Its JSON stdout carries binary/configuration SHA-256, build, worker topology, isolation provenance,
measured CPU/wall totals and per-minute evidence. Save that stdout as the
commissioning artifact; a miss/no-data exits nonzero. This is `cpu/quiet-reference`, explicitly
`production_objective=false`, not a third production objective or evidence of the busy host's
idle behavior. Production `cpu` and conditional `cpu/idle` are unchanged by commissioning.

Regression commands (not substitutes for a measured commissioning receipt):
`cargo test -p smallclaims windows::cpu_minutes`,
`cargo test -p fractal-store runtime_heartbeats_do_not_make_a_quiet_live_session_productive`,
`cargo test -p fractal-api-core the_cpu_rows_are_met_missed_or_no_data_never_met_without_the_facts`,
and `python3 scripts/commission-cpu-quiet-reference-test`.

### Source-backed GitHub measurements

`github-merged-count` and `github-ready-to-merged` read the canonical committed GitHub projection,
not an independent acquisition loop or the generic earliest-claim-wins count/interval evaluator.
Their TypeScript constructors are `Fractal.indicator.githubMergedCount` and
`Fractal.indicator.githubReadyToMerged`. Each requires authored `source` and `roster` subject
references, an IANA `timezone`, explicit `cut: 'now' | 'watermark'`, and
`measurement: 'only' | 'targeted'`. There are no default hosts, repository lists or target values.

Merged-count requires `PullRequest` and `Merge` coverage; ready-to-merged additionally
requires `Lifecycle` coverage. The finite evidence horizon is the latest authoritative
required-family coverage endpoint intersecting effective roster membership. Retired
members contribute only up to their authored membership end and never pin that horizon.
`cut: 'watermark'` evaluates at this horizon (bounded by Now), not an invented claim of
complete coverage. Each rolling-hour or civil-day window independently requires every
member effective in that window to have all required-family coverage for its membership
intersection. Missing coverage makes that aggregate window unknown, while fully covered
historical civil-day bars remain available. A missing lifecycle family cannot erase
otherwise covered merge-count bars, and an uncovered new member cannot erase days
before that member joined.

Root acquisition and membership declarations use `Fractal.githubSource` and
`Fractal.githubScope` (also named package exports). They return KDL nodes for `emit` or
`fractalKdl`, using the native schema declaration reader's fields:

- `githubSource({ name, host, generation, authorityGrant?, repositories, enabled,
  checkpointEveryMs, replicatePrivate })`: repositories are unique `owner/name` locators;
  the cadence is at least 60,000 ms. Both booleans are explicit. A disabled source
  can omit `authorityGrant` and publish before authority admission; enabling requires
  a nonempty admitted grant reference in both the TypeScript type and native validation.
  This declaration creates neither an observer nor a grant.
  Each repository still needs an observer on the admitted acquisition member.
- `githubScope({ name, revision, repositories: [{ repositoryId, fromMs, untilMs? }] })`:
  repository IDs are stable GitHub node IDs, not locators. Membership is half-open
  `[fromMs, untilMs)`, with an explicit version and authored Unix-millisecond start.
  Same-repository intervals may touch but cannot overlap. An authored activation start
  establishes prospective membership, not historical coverage. Builders reject unsafe
  JavaScript integers rather than rounding native timestamps.

Private policy belongs in its owning declaration source, not the built-in daemon objectives.
It supplies source identity, effective-dated versioned roster and timezone before
declaring source-backed objectives, and selects an admitted authority before enabling
acquisition. A throughput policy of at least twenty
merges per rolling hour is `target "per-hour" min=20`; true ready-to-merged remains
`measurement "only"` without targets. Neither repository discovery nor another workflow's
roster establishes that policy or its historical effective dates.

```kdl
objective "delivery/ready-to-merged" {
  about "Final ready to merge, measured without a target"
  indicator "github-ready-to-merged" {
    source "github-source/test"
    roster "github-scope/test"
    timezone "America/New_York"
    cut "watermark"
    measurement "only"
  }
}
```

Merged count is distinct repository/PR identities across **all authors**, in exactly
`[cut - 1 hour, cut)`, under the versioned roster's effective membership at each merge.
`measurement "targeted"` requires one policy-authored `target "per-hour" min=NUMBER`.
Ready-to-merged is measured-only: the sample begins at the final proven draft-to-ready
transition before merge, or at creation only with explicit created-ready proof. Complete
lifecycle coverage must extend back to creation. A draft transition resets the cycle;
queue entry, head push and the first-ever ready event are not proxies.

`measurement "only"` requires no targets and forbids `on-breach`; it never implies a target
or green verdict. The native SLO row exposes `verdict: "measured-only"` while retaining
`status: "no-data"` for existing verdict consumers. Missing coverage, members, PR images
or lifecycle proof are unknown, never met. A completely covered empty count is a real zero.
Corrections and retractions replace previous projection images and therefore revise previous
counts and cycles.

In TypeScript, either omit `targets` or provide `targets: []` for both measured-only
GitHub indicators. Targeted merged-count still requires its explicit per-hour minimum.

The native SLO measurement evidence contains fourteen civil-day bars, including partial today,
with timezone, measured bounds and full calendar end bounds. Day lengths honor DST. It also
carries source identity/generation, projection and roster revisions, the coverage frontier and
source lag. `cut "now"` preserves the current cut and marks lagging acquisition unknown while
retaining observed partial samples. `cut "watermark"` explicitly measures as of the trustworthy
source frontier; it does not disguise that older cut as now. These indicators own their rolling
hour/calendar windows and take no generic `window`.

Source-backed rows are composed before native owner bindings are evaluated. A binding
may reference an existing GitHub measurement row and inherits its evidence and status
without overriding its target or inventing an error budget. Fully covered throughput
targets retain count semantics (`met`/`missed`); ratio/gauge budgets retain the shared
`burning`/`exhausted` states. Both use the existing owner-episode dispatcher. Unknown
coverage remains `no-data`, never a recovery of an open episode. Measured-only
ready-to-merged has no success verdict and cannot acquire one through ownership.

### Fleet roll completion

`fleet/roll-completion` measures **whole rolls**, not host-roll pairs: 90% must have
every immutable target host verified within 24 hours of declaration. The 30-day
budget allows 10% bad rolls and uses the common paired burns (14.4× over 1h+5m,
6× over 6h+30m, at least one known roll in each window). Holds, admission waits,
retries and rollback do not pause or reset the declaration clock. There is no
public fleet roster or personal owner embedded in this generic source.

The deployment authority is an actual bound agent with this existing resource edge:

```kdl
resource "roll-authority" subject="resource/fleet/roll-completion" reason="Declare immutable rolls"
```

Only that agent's bound execution may publish declaration/resolution claims through
`fractal claim`. An agentless mission-run credential, `--actor` text, or an unbound
local client cannot impersonate the authority. Signed `resource.observed` claims
carry `fields.kind = "custom.fractal.fleet-roll-receipt"` using the existing open custom
resource schema, and the version-1 receipt in `fields.facts`.

Authority receipts also require a currently usable delegated signing chain.
Historical signature authentication alone does not preserve authority after a key
in that chain is revoked: admission and replay remove that receipt's trusted
contribution, rather than retaining cached healthy adoption. Existing source
enrollment can recover only from a surviving authenticated candidate.

For an enrolled native rollout, resolve the onset and pinned revision with
`fractal slo roll-intent mission-run/RUN_ID --json`. It returns `roll_id`,
`declared_at_ms`, `dotfiles_revision` and `resource_claim_id`, authenticated against
the original native creation envelope and immutable resource input. The onset is
the creation claim's canonical native acceptance time, **not** a deadline-derived
mission view timestamp or a caller's current clock. Missing proof is an error.

For example, declaration facts are:

```json
{
  "version": 1,
  "kind": "roll-declared",
  "roll_id": "example-roll-42",
  "declared_at_ms": 1795000000000,
  "targets": {
    "host-a": {
      "dotfiles_revision": "1111111111111111111111111111111111111111",
      "daemon_revision": "3333333333333333333333333333333333333333"
    }
  }
}
```

Declare the complete nonempty cohort **before the first build/admission, drain,
hold or activation effect**. Dotfiles and daemon revisions are nonempty exact pinned
identities. This rollout activates dotfiles, system, home and daemon only;
`factory_revision` is not a supported target or verification field and is
rejected, including `null`. Autonomous factory-applier provenance is not a
declared host activation target.
Optional `system_path` and `home_path` constrain already known realized paths; do not
delay declaration until evaluation/build produces paths.
A held host remains in the cohort. Same roll ID/time/targets is an idempotent
retry; changing the time or cohort is refused, never treated as a new attempt.

The declaration clock and all target/resolution/observation references stay
immutable. Same-target native runs can be discovered out of order across hosts:
each host's own collector also signs `intent-observed` proof binding the source
enrollment, immutable declaration ID and original `mission-run.created` claim ID.
It verifies the creation's original signed host envelope and pinned resource
revision, never caller evidence or a deadline-derived projection clock.
The effective SLO onset is the minimum of declaration time and these canonical
native acceptance times. A later-discovered earlier intent can only move the
deadline **earlier**, never restart it or rewrite the signed declaration.
Every own intent needs an admitted proof before the source can close past it.
Frontier deltas carry those proofs and mirrors wait for their original creation
and pinned input dependencies. Generic public intent-onset signing is forbidden
for every principal, including the real declaration authority.

Publish under `resource/fleet/roll-completion/ROLL/declaration` using the actual
bound actor, never a fabricated identity:

```sh
fractal claim resource/fleet/roll-completion/ROLL/declaration resource.observed \
  --actor "$ST_AGENT" --field kind=custom.fractal.fleet-roll-receipt --field state=declared \
  --field "facts=$DECLARATION_JSON" --idempotency-key "roll/ROLL/declaration" --json
fractal slo ingest-roll CLAIM_ID --json
```

`POST /v1/slo` accepts only `{"claim_id":"..."}`, not receipt JSON or a local file.
It resolves the claim from the graph and requires verified whole-record, direct
signature authority before reserving a roll ID. The signer must have the declared
resource binding; unsigned, delegated, partial, and unauthorized claims cannot poison
the cohort. Origin strings in imported JSON are not proof.

After resolving the actual controller and home candidates, the **same signed
declaration authority** publishes one immutable `host-target-resolved` receipt per
host under `resource/fleet/roll-completion/ROLL/HOST/resolution`. Its fields are
`version`, `roll_id`, `declared_at_ms`, `host`, `target` (the immutable declared
identity), `expected_system_path`, `expected_home_path`, and
`declaration_claim_id` (the admitted declaration claim). Publish it with the same
`fractal claim` form and `state=resolved`, then admit its claim reference.
Conflicting resolved paths are refused; retries do not change the original clock.

The producer calls the target daemon's `POST /v1/slo/observe` with only
`{"claim_id":"SIGNED_RESOLUTION_CLAIM_ID"}`. The daemon independently reads its own
active system/home profiles, running compiled revision and readiness. It rejects
another host's resolution and never accepts caller-provided adoption, paths, health,
or revision evidence. Missing supported observations remain unknown. The removed
`factory_revision` field is rejected rather than promising an unavailable reader.

The internal collector publishes a signed `host-observed` resource observation
under `resource/fleet/roll-completion/ROLL/HOST/OBSERVED_AT_MS`; only a verified
direct `host/HOST` signature with no actor attribution may assert that host's
adoption. Generic public claim publication forbids **all** host-observed receipts,
even for a bound roll authority, so the daemon cannot be used as a signing oracle.
The collector returns its signed observation `claim_id` with the admission result.
The observation time is sampled only after resolution authentication, graph
readiness, daemon health and active-profile reads finish. It is floored by the
native acceptance clock, with source-census/publication serialization held until
the signed observation is published. A measurement that crosses the deadline
cannot count as timely by retaining its start time.
These existing generic claims carry their signature proof through ordinary graph
replication and replay; there are no new schema claim kinds.

Adoption is `verified`, `not-adopted` or `unknown`. Only unknown observations may
omit evidence. Known negative observations require the same complete, nonempty
independent evidence as verification and at least one actual mismatch or
not-ready daemon; a missing probe is unknown, not a negative proof, and an
all-matching negative label is refused. Missing receipt, unknown adoption and predeadline nonadoption
remain unknown even after the deadline; only an observed **post-deadline**
nonadoption or eventual late verification proves a miss. Completion is the last
target host's earliest exact verification, not inferred installation time. A
timely completed roll remains known despite later unknown readings. A mixed
verified/unknown cohort is not a completed roll. Timely events account at final
verification; late events account at the original deadline, so late ingestion
does not move failures into the current accounting window. Historical evidence
can revise earlier windows without creating another denominator event.

Signed graph claims are source authority. The local
`state_dir/roll-signed-receipts.sqlite3` ledger retains admitted claim references,
immutable receipt keys and admission times; restart revalidates graph signatures
and authority before rebuilding the in-memory source. Backups must retain the
graph and its signature/provenance records. The obsolete unsigned
`roll-receipts.sqlite3` remains diagnostic only and is deliberately never replayed
as authority; re-publish legitimate source receipts through the signed producer.
No unsigned JSON import path remains.

The commit observer subscribes before its initial graph scan and admits signed
declarations, resolutions and observations from normal replication. Missing
dependencies remain pending and retry only on graph commits. Source admission wakes
native projections; there is no file poll. SLO reads use only the maintained
in-memory source, never ledger I/O or its writer lock.

Population eligibility requires **source-complete native intent coverage**, not
just whichever declarations have arrived locally. Before declaration effects,
the actual bound authority enrolls its source once at
`resource/fleet/roll-completion/sources/ACTOR/enabled`, with state `enabled` and facts:

```json
{"version":1,"kind":"source-enabled","producer":"agent/example/deployer","enabled_at_ms":1234567890000,"mission_prefix":"example/roll/","resource_input":"main","revision_fact":"head","roll_id_prefix":"example/"}
```

Publish with the same authenticated `fractal claim` interface as declarations,
but **omit `--idempotency-key` for source enrollment**. Generic operation keys
are not signing-key scoped and can otherwise replay a revoked enrollment forever.
Republish the unchanged journal facts from the actual bound authority when
declaring a roll. Native admission retains equivalent signed candidates without
resetting the earliest still-valid enrollment epoch. After key rotation/revocation,
replay selects the earliest surviving authenticated candidate and starts coverage
no earlier than its own durable acceptance; no journal or rollout clock is reset.
The journal timestamp does not authorize backdated coverage: native admission and
membership epochs clamp the coverage start. Each current fleet member then closes
its **own** durable native mission-intent stream, including held, pending and
terminal runs. Pinned immutable resource inputs determine the roll ID; unresolved
or missing declarations bound the cut before the earliest uncovered intent.
Foreign-origin intent copies never substitute for their original member's proof.

Native daemons publish signed source frontiers on source changes and at most once
per minute while quiet. Generic frontier publication is forbidden, even for a
bound authority. Each frontier pins the producer membership, current host keys,
source enrollment, preceding frontier and only the new own-origin receipt IDs
since its native snapshot cursor. These bounded deltas include declaration,
resolution, **host observation and native intent-onset** receipts. The mirror waits for the entire
chain and every delta receipt, so a frontier arriving ahead of a positive adoption
claim cannot make an older negative observation falsely complete.

Each incarnation versions one stable frontier resource subject; heartbeat time
does not create another resource card. A delta contains at most 512 receipt IDs.
After downtime, native commit-driven catch-up advances through bounded chunks,
closing before the earliest excluded receipt's canonical acceptance time. Several
chunks can share a timestamp but have strictly increasing snapshot cursors. An
empty initial interval remains unknown until the entire timestamp bucket arrives;
no truncation certifies a larger cut. Atomic native publication rejects an
intervening backdated source write and fences subsequent own writes beyond the
cut. Observation timestamps use that same native monotonic floor inside the
shared collector/census guard.

The cursor is bound to the existing native durable database identity and source
epoch (`ivm_event_state.database_id`, `ivm_source.epoch`), captured in the same
snapshot as the census. Ordinary reopen preserves this identity. Native
restore/clone identity rotation starts a new source interval at the new native
cut, never reuses an old arrival cursor, and cannot inherit prior coverage.

The roll runtime explicitly initializes this existing canonical source lifecycle
before retaining signed receipt references. Initialization does not attach
optional materialized views or enable their claim-capture hooks, and preserves an
existing source epoch, event-retention choice and unavailable state. Missing or
unavailable lifecycle proof is never replaced with a fabricated identity.

The in-memory reducer keeps only the latest closed frontier per producer/host;
signed graph and ledger history retain predecessor proof for replay.
Authenticated retries of the retained frontier are idempotent even after its
predecessor leaves the hot reducer. Older native writer positions are terminal,
not missing dependencies; the commit watcher drops them instead of retaining
one pending candidate per heartbeat. A new frontier still waits for an
unadmitted predecessor and every receipt in its delta.

Evaluation intersects all configured producers × all current host incarnations
and evaluates ratios and paired burn windows at that common proven cut, not
wall-clock `now`. The uncovered tail stays explicit. A missing member proof,
reincarnation, source conflict, or a frontier older than 120 seconds is unknown.
Each window also needs source history covering its width plus the 24-hour
declaration-to-deadline lookback; new enrollment cannot fabricate thirty days of
history. Without that proof, status is `no-data` and budget remaining is absent.
Unknown open rolls stay visible separately and never erase independently known
completed outcome counts or become good/bad denominator events. In each budget
window, overdue unknown rolls with deadlines in that window reduce coverage:
known outcomes / (known outcomes + relevant overdue unknowns). Each window
requires at least 90% coverage and one known outcome. Not-yet-due open rolls are
pending, not coverage failures. Insufficient coverage is `no-data`, not healthy
recovery; known good/bad counts remain visible. No known outcomes means
`no-data`, not green. `measurement` exposes the frontier, receipt time,
population, open unknowns and each budget window's coverage and unknown count.

Bind ownership with the existing declaration mechanism; no new owner route:

```kdl
objective "example/roll-completion" {
  about "Deployment outcome owned by the deployer"
  indicator "native" { objective "fleet/roll-completion"; }
  on-breach "fault" owner="agent/example/deployer" condition="roll-completion"
}
```

The binding inherits measurement and opens the existing member-local owner
episode for burning/exhausted budgets. Unknown data never recovers an episode.

### Native ratios, covered-time gauges and ownership

The native budget contract measures failures directly rather than turning a percentile miss
into an event count. Ratio denominator events are deduplicated by subject and event timestamp;
the numeric field is bad only when it is strictly above `bad-above`. Missing or malformed
numeric values and timestamps are not successful events. Times use the same RFC 3339 or Unix
millisecond field representation as other claim indicators, but `at` is required.

Mixed TypeScript and KDL objective declarations load in canonical declaration-source path order,
not objective-ID order.

```kdl
objective "acme/request-budget" {
  about "Interactive requests completing within 100ms"
  indicator "ratio" {
    subject-prefix "custom/acme/request/"
    event "custom.acme.request" at="observed_at"
    value "duration_ms" bad-above=100
    label "route"
    only "class" "interactive"
  }
  budget allowance=0.05 horizon="30d" min-coverage=0.9 min-samples=20 {
    fast burn-rate=14.4 long="1h" short="5m"
    slow burn-rate=6 long="6h" short="30m"
  }
  on-breach "fault" owner="agent/acme/ops" condition="request-budget"
}
```

`allowance` and `min-coverage` are fractions, not percentages. Burn is observed bad fraction
divided by allowance. Each burn pair requires **both** its long and short windows above the
declared rate, with enough evidence; fast and slow are alternative breach conditions. The
30-day horizon is the budget accounting period, not a claim that a newly started member already
has 30 days of evidence. Coverage and unknown time remain visible. Shipped allowances are 5% for
latency, 1% for transactions and 5% for database capacity. Existing max and p99 diagnostics stay
visible and independent rather than becoming a substitute denominator.

| Native rows | Slow / bad condition | Operational allowance | Diagnostic aspiration |
|---|---|---|---|
| `person-read`, `person-read/remote`, `write-ack` | completed work strictly over 300ms | 5% of eligible events | p99 ≤300ms |
| `projection-fresh` | eligible publication strictly over 1500ms | 5% of eligible publications | p99 ≤1500ms |
| `transaction/read`, `transaction/write`, `transaction/writer_hold` | transaction or hold strictly over 300ms | 1% of that row's eligible events | max ≤300ms |
| `database` | live size >8GB **or** growth >300MB/day | 5% of covered capacity-hours | both capacity bounds |

These rows account over 30d with fast 14.4× and slow 6× paired burns. Ratio pairs are
1h+5m and 6h+30m; database expands them to its actual hourly cadence as described below.
The transaction family keeps three separate populations; request completion, publication and
capacity time are never averaged together. The `writer-hold` holder-attribution row is a separate
diagnostic, not a second operational transaction budget.

Dedicated bulk exports have separate diagnostic operation classes in the native
request-latency `routes` array: select `scope=export-operation` and
`operation_class=backup-export`, `authority-proof-export`, or `replication-export`.
These count completed response envelopes, overlap the original route totals, and remain
available when the arbitrary-route table fills. Do not sum scopes. They do not change any
interactive objective or invent an export latency target. A shared mailbox page can serve
either an ordinary listing or an export, so its route alone does not classify it as an export;
ordinary session opens likewise remain in `person-read`, regardless of duration.

For example, a 0.45% transaction/write slow share is below its 1% allowance even if one long
transaction misses the max aspiration. A 35.24% person-read slow share burns at 7.05× its 5%
allowance: it cannot establish a 14.4× fast burn. It establishes a slow burn only if **both**
6h and 30m have eligible evidence above 30% slow. An hour's distribution cannot stand in for
either missing window or for 30d remaining budget.

Native telemetry persists independent closed UTC-minute receipts for 30 days in
`state_dir/slo-budgets.sqlite3`. The durable ledger preserves closed evidence across restarts;
restart partial minutes and collection gaps are unknown rather than inferred good. This is
local observation history, not a reconstruction from diagnostic percentiles.
Request and SQL completions append only to an independent in-memory minute buffer. The
checkpoint swaps that buffer under its short recorder lock, releases it, then merges and
persists receipts under a separate ledger lock. Checkpoint and gauge SQLite work never hold
the recorder lock; a blocked disk operation cannot delay completion recording.
Failed checkpoint snapshots remain available for retry while new completions collect separately.

Installation, checkpoints and gauge mutations publish a maintained immutable native evidence
projection through a short in-memory Arc swap. Its explicit as-of accounting cut and covered/unknown
time describe the evidence actually published. Every reader, including `current_slo_rows` inside
an existing caller-owned read snapshot, reads only that projection and performs no ledger I/O.
There is no reader-triggered refresh or TTL cache. If the current closed-minute cut has advanced
beyond the published projection, readers expose stale/unknown evidence instead of an old green
verdict. A failed publication cannot advance freshness or retain a pre-mutation healthy projection.

All ledger mutation paths run through one publish/invalidate boundary, including their prerequisite
SQL reads. A gauge SELECT or INSERT failure therefore invalidates a same-cut healthy projection
just like a checkpoint or summary-rebuild failure; no database error can bypass invalidation.

Reporting also takes an owned snapshot of request, client and CPU diagnostics under the Meter
lock and releases that guard before attaching native budgets and rendering store diagnostics.
Neither a caller-owned PinnedRead nor a completion diagnostic lock spans native ledger I/O.

Database capacity uses hourly readings with a two-hour maximum age. A violation of either
capacity bound is known bad; known good requires both bounds to be readable and within limits.
Missing evidence for either bound therefore cannot turn a partial reading into healthy time.

A `gauge` uses the same event, value, labels and filters, but measures **covered duration**
instead of event count. Each subject is one series. A valid reading holds forward from its
timestamp until the next reading or `max-age`, whichever comes first; it never fills time before
the first reading or bridges a stale gap. Its bad covered milliseconds divided by covered
milliseconds is the observed bad fraction; uncovered time is unknown, not good.

```kdl
indicator "gauge" {
  subject-prefix "custom/acme/capacity/"
  event "custom.acme.capacity" at="observed_at"
  value "free_bytes" bad-below=1000000000
  max-age "2h"
  cadence "1h"
}
```

Gauge thresholds take exactly one of `bad-above` or `bad-below`; equality is good. `cadence` is
the source's measurement interval, not an interpolation permission. `max-age` must be at least
one cadence. Effective burn windows expand to `short=max(configured short,cadence)` and
`long=max(configured long,2*cadence)`: hourly gauges use fast 2h+1h and slow 6h+1h, not fabricated
five-minute evidence. Both actual windows still require coverage and sample gates. Gauge and
ratio declarations require `budget` and take neither `window` nor diagnostic `target` children.
The evaluator retains the longest effective burn or accounting window plus the held-value
predecessor; cadence expansion never truncates the evidence needed to establish coverage.
Thresholds must be finite and nonnegative; allowance is in (0,1), coverage in (0,1], and minimum samples positive.
Each pair requires `short < long <= horizon`, and fast has shorter windows and a higher rate
than slow. Unknown, repeated and missing fields are errors.

The TypeScript equivalents are `Fractal.indicator.ratio({ subjectPrefix, event: { kind, at },
field, badAbove })` and `Fractal.indicator.gauge({ subjectPrefix, event: { kind, at }, field,
badBelow, maxAge, cadence })`. `Fractal.objective` takes `budget: { allowance, horizon,
minCoverage, minSamples, fast: { burnRate, long, short }, slow: { burnRate, long, short } }`.
Gauge inputs allow exactly one of `badAbove` and `badBelow`. Targets may be omitted for these
indicators. The DSL checks these same rules and lowers to the KDL above.

Fleet ownership of a built-in row is a separate, namespaced declaration, not a redefinition of
native measurement or an embedded private owner in public defaults:

```kdl
objective "acme/person-read" {
  about "Fleet ownership of native latency"
  indicator "native" {
    objective "person-read/remote"
  }
  on-breach "fault" owner="agent/acme/ops" condition="person-read-budget"
}
```

Use `Fractal.indicator.native({ objective: 'person-read/remote' })` in TypeScript. The reference
may identify a built-in subrow such as `transaction/read` or `transaction/writer_hold`. References
allow underscore-joined words within a path segment; declaration names retain their hyphen-only
word grammar. A native binding inherits the source
row's measurements and budget and forbids `target`, `window` and `budget` overrides. The parser
checks reference syntax; evaluation resolves it against actual built-in rows, so an unknown
reference is explicit no-data, never a healthy fallback. Responses use the declaration's owner
and condition and retain a separate episode on each observing member. Delivery can route to a
live declared owner agent hosted on another member; ownership is not restricted to a local
process and public built-in definitions contain no private fleet IDs. Existing interval and
count declarations keep their original syntax and semantics.
Native episode failures persist the exact declaring objective name. Cleanup reads that ownership
from the claims, so changing the bound native family or restarting the member still recovers
obsolete children without prefix-matching an independent nested declaration. A wholly unavailable
source keeps its previously open owned rows unknown, including after restart.

### GO-to-merged completion ratios

[`slo/dotfiles-merge-latency.kdl`](../../slo/dotfiles-merge-latency.kdl) declares
`landing/merge-latency/dotfiles/p50` and `landing/merge-latency/dotfiles/p95` through
the existing native `ratio` evaluator. These are ratio row IDs, not diagnostic percentile
estimates: at least 50% of qualifying completions must take at most 600 seconds, and at
least 95% at most 1800 seconds. Both use the half-open merged-completion cohort
`[now - 30d, now)`. Admission or GO before that horizon does not exclude a merge inside it.
Equality at either duration is good; equality at the bad-event allowance satisfies the
rendered `≤` target with zero remaining budget. Exhaustion requires exceeding that
allowance. This inclusive boundary applies to the shared native budget classifier;
unknown evidence and paired-burn decisions are unchanged.

The source is an explicitly published declaration, not an automatically activated observer
or a second configuration reader. Include it in the owning fleet's governed declaration
source and publish through that source's existing `fractal apply --set` lane. Preview with
`fractal apply --as "$FRACTAL_AGENT" --dry-run slo/dotfiles-merge-latency.kdl`.
This file neither starts acquisition nor changes the merge queue. It declares no breach
owner; fleet policy may author `on-breach` through its normal ownership lane.

The queue-derived producer ingests existing custom claims through `fractal claim` /
`POST /v1/claims`, not a new API or schema kind:

- Kind: `custom.landing.dotfiles-merged`.
- Subject: `custom/landing/dotfiles/merged/DIGEST`, where `DIGEST` is SHA-256 of the
  UTF-8 compact JSON array `[repo,pr,head,admission,merge]`. Preserve producer-normalized
  tuple values on retries; do not derive identity from ingestion time.
- Fields: `schema: "fractal.merge-latency/v1"`, `repo`, `pr`, `head`, `admission`,
  `merge`, `admittedAt`, `mergedAt`, `goAt`, `goSource`, `goEvidence`, `durationSeconds`.
  `head` is the admitted full head; `merge` is the completed merge commit. Times are
  RFC 3339 or Unix milliseconds. `durationSeconds = (mergedAt - goAt) / 1000`
  must be finite and nonnegative, without rounding at the threshold.
- `goSource: "route-a-exact-head"` requires an unedited exact-full-head
  `q19-go v1 head=<40hex> verdict=GO` comment with one valid v3 footer identifying
  a reviewer different from the PR body's owner; `goEvidence` is its durable URL.
  These are self-asserted footer semantics, not independent authentication.
  Preparation/candidate timestamps and free-form review prose are not promoted to GO.
  Without qualifying evidence, `goSource: "admission-proxy"` uses admission time and
  `goEvidence` holds the admission ID; both objectives explicitly exclude it.
- Pass each field as `--field KEY=JSON`, plus `--actor "$FRACTAL_AGENT"` and a stable
  `--idempotency-key`. The command parses JSON before its raw-string fallback, so encode
  strings as JSON too. Retry the identical accepted fact, including its original
  completion time and provenance, rather than inventing a new event timestamp.

The producer owns validating durable completed admission/merge pairs and GO evidence;
custom fields are not a new registered claim schema. The native reader incrementally
reconstructs rows from replicated claims after restart or declaration revision and
deduplicates repeated claims by subject and event timestamp. A missing or malformed
duration is unknown, never good. Producer output/journal remains the replay source.
No new schema registry, generated schema, projection, reconstruction kind, route,
or acquisition cost probe is introduced.
The integration tests compile this exact KDL source rather than duplicating its policy.
It is therefore an explicit `fractal-api-core` extra in `.st/rust-test-inputs.json`
and an input to both `linux-clippy` and `linux-fleet-compat` in the evidence policy
and lane manifest, alongside the other compile-time SLO sources.
The included KDL is also registered as source site `lv-ha5695af111aa` in the existing
SQLite source inventory, with its adjacent owner/action marker. This accounts for
test-source provenance only; it does not assert a SQLite value-size proof or add a
runtime database path.

Only exact-GO completions form the denominator. A proxy-only cohort reads
`no-qualifying-events`, not met; unobserved or proxy merges cannot prove fleet-wide
compliance. One qualifying completion makes the 30d budget eligible, but recent paired
burn windows retain their independent evidence gates: an eligible historical cohort
without recent evidence can read `not-eligible`, not an invented healthy burn verdict.
Inspect `fractal slo explain landing/merge-latency/dotfiles/p50` (and `/p95`) for
qualifying/excluded/unknown counts and remaining budget; use
`fractal claims ls --kind custom.landing.dotfiles-merged` for provenance and producer
coverage. These native ratios do not assert that the producer has observed every merge.

### Interval over claims

`interval` measures work that starts and stops in different processes and on different members,
from claims those processes write. Each subject under `subject-prefix` is one sample.

| Child | Meaning |
|---|---|
| `subject-prefix "P"` | The sample subjects: every subject that starts with `P`. Custom claims (`custom/NS/...`) need no registration and replicate to every member. |
| `start KIND at=FIELD` | The sample starts at the earliest such claim. |
| `expect KIND set=FIELD` | The members a sample must stop on, a list of strings in the newest such claim (by its time, then its claim id, so every member picks the same one). An empty list stops the sample at the expect claim's time. Without `expect`, one stop claim ends the sample. `expect` may name the `start` kind, so one claim starts the sample and names its members (merge to live). |
| `stop KIND member=FIELD` | One member has stopped: the earliest such claim per value of `FIELD`. The sample stops when every expected member has, at the latest of their times. `stop` may name the `start` kind when both read their time from different `at=` fields and there is no `expect` or `member=`: one claim is then the whole sample, from its start field to its stop field (a CI run from `started_at` to `completed_at`). Every other kind has one role. |
| `cancel KIND` | An open sample with such a claim is cancelled. A sample that stopped before the cancel was accepted keeps its value. |
| `pause KIND resume=KIND` | The clock stops from each pause to the next resume, and a sample still paused is not aging. Pauses and resumes are timed when they were accepted. |
| `label "FIELD"` | Also one row per value of `FIELD` on the start or expect claim: `ops/change-rollout/p95{class=routine}`. A sample without the field is only in the unlabelled row. `label "member"` splits by member, each sample's value being that member's stop: the expected members, or without `expect` the members that stopped. |
| `exclude "FIELD" "A" "B"` | A sample whose start or expect claim has one of these values is excluded. |
| `only "FIELD" "A" "B"` | A sample is kept only when its start or expect claim has one of these values; one without the field is left out too. Several `only` must all hold, each on its own field. A left-out sample is counted with the excluded ones. |

`at=FIELD` reads a time from the claim's fields: an RFC 3339 string or Unix milliseconds, either
also as a JSON-encoded string. Without `at`, the time is when the claim's origin accepted it, which
every member sees the same: a replicated claim keeps its origin's acceptance time. A list field may
also be a JSON-encoded string. A claim whose time, list or member cannot be read is counted as
unreadable and changes no sample.

A sample is **closed** when it stopped, **open** while it has started and not stopped or been
cancelled, **cancelled**, or **excluded**. A closed sample belongs to the window by its stop time, a
cancelled one by its cancel and an excluded one by its newest claim; an open sample is always
current. A subject with claims but no start yet is counted apart and in no class.

**Open samples are censored, not dropped.** For each target, an open sample older than the target's
`within` enters that target's percentile at its current age: it is certain to be over, so a stuck
change spends budget instead of hiding. A younger open sample is left out of the percentile, since
it may still finish within. The row counts every class, so nothing is silently dropped:

```
ops/change-rollout/p95  missed  p95 ≤ 5 min  p95 41 min over 23 samples (1 open over)  7d  0%
```

The value is the nearest-rank percentile. The budget is the share over `within` against the share
the percentile allows: p95 allows 5 in 100. With fewer than `min-samples` in the percentile the row
is `no-data` and spends no budget; with none it reads `no samples`.

Each evaluation reads only the claims newer than the last it read, keeping each objective's samples
in memory; a sample closed, cancelled or excluded before the window is dropped from memory.

### Count over claims

`count` counts events that processes write as claims, such as pull requests merged or flaky tests
seen, per hour over the window. Each subject under `subject-prefix` is at most one event.

```kdl
objective "factory/work-throughput" {
  about "Factory-authored pull requests merged into the roster's default branches"
  indicator "count" {
    subject-prefix "custom/factory/merged-pr/"
    event "custom.factory.pr-merged" at="merged_at"
    label "repository"
    exclude "author" "renovate"
  }
  window "24h"
  target "per-hour" min=10
}
```

| Child | Meaning |
|---|---|
| `subject-prefix "P"` | The event subjects, as for an interval. |
| `event KIND at=FIELD` | A subject's event is at its earliest such claim, its time read from `FIELD` as an interval's `at`. `at` is required: the event's own time, which every retry and every producer of the subject repeats, so a retry inside the window is the same event and one after it is before the window and ignored. Retries and a second producer never count twice. |
| `label "FIELD"` | Also one row per value of `FIELD` on the event claims the revision has read: `factory/work-throughput{repository=fractal}`. A label row is a part of the count, shown without a verdict (below). |
| `exclude "FIELD" "A" "B"` | An event whose claim has one of these values is excluded: counted apart, never in the rate. |
| `only "FIELD" "A" "B"` | An event is counted only when its claim has one of these values; one without the field is left out and counted with the excluded ones. Several `only` must all hold, each on its own field. |

The objective takes one target, `per-hour` with `max=N` (at most N an hour, N ≥ 0) or `min=N` (at
least N an hour, N > 0), and a `window` without `min-samples`. The row `NAME` reports the events in
the window and the rate, events ÷ window hours:

```
factory/work-throughput  met  ≥ 10 an hour  312 in the last 24h (13.00 an hour)  24h  23%
```

- `max=N` is met when the rate is at most N, and the budget left is `100 × (1 − rate ÷ N)`: with
  `max=0` it is 100% while met and 0% once missed.
- `min=N` is met when the rate is at least N, and the budget left is `100 × (1 − N ÷ rate)`, the
  share of the rate above the minimum, as a `share`'s budget is the share of its allowed remainder:
  0% at the minimum or below, nearing 100% as the rate grows.

Only the row `NAME` is judged. The bound is the whole count's, so a label row is a part of it: it
reports its events and rate with the target `part of NAME`, is `no-data` with no budget, and never
opens a fault. With `min=10` over five repositories, 13 an hour in all is met though no repository
alone reaches 10. A label value keeps its row once the revision has read it, reading 0 when its
events have left the window, so a repository that stopped merging still shows; the values are kept
per revision, at most the distinct values the event claims carry.

```
factory/work-throughput{repository=fractal}  no-data  part of factory/work-throughput  0 in the last 24h (0.00 an hour)  24h
```

The row `NAME` is `no-data` until the objective has read one event claim for its revision; from
then on an empty window is a real 0, so a producer that stopped shows as a missed minimum rather
than hiding.
A claim whose time cannot be read is counted as unreadable and changes nothing. `fractal slo explain`
says the events counted and excluded in the window, the rate, the unreadable claims, and the
subject and revision.

Each evaluation reads only the event claims newer than the last it read, and keeps only the events
in the window and the label values read: an event before it is dropped from memory, and a claim
timed before it is not kept, though its label value is.

### Claim bucket series

Use `fractal series --objective ID --from MS --to MS [--bucket-seconds N]`, or
`fractal series --kind KIND [--kind KIND...] [--subject-prefix PREFIX] [--group-by FIELD]
--from MS --to MS [--bucket-seconds N]`. The CLI defaults to 86400-second buckets;
global `--json` prints the response as JSON.

`GET /v1/series` and `GET /v1/client/series` require `from` and `to` (unsigned Unix
milliseconds), `bucket_seconds` (positive integer seconds), and one selection mode:

- One or more repeatable `kind=KIND` parameters select stored claims of those kinds.
  Optional `subject_prefix=PREFIX` restricts subjects by a literal prefix. Each selected
  stored claim counts once, including distinct claims for the same subject; repeating a kind
  does not count a claim twice. The first present timestamp field in `at`,
  `occurred_at_unix_ms`, `occurred_at` determines the claim's event time. A present but
  unreadable timestamp excludes that claim, even if a lower-priority field is readable.
  Generic-read visibility applies before timestamps, counts or groups: reserved notice subjects
  and reserved notice kinds contribute nothing, including when explicitly named in a selector.
- `objective=NAME` selects the exact declared count or interval name, not a percentile or
  label row. It cannot be combined with `kind`, `subject_prefix` or `group_by`.

The range is half-open `[from,to)`, with `from < to`, at most 31 days and at most 1000 buckets
(`ceil((to-from)/(1000*bucket_seconds))`). Buckets begin at `from`; the last ends at `to`
even when shorter than the requested width. Empty buckets have count zero.

The response is `{from,to,bucket_seconds,buckets:[{start,end,count}]}`. With explicit kinds,
optional `group_by=FIELD` selects an exact top-level key in the original claim's `fields`
object. The response then also carries `group_by`, and every bucket has
`groups:[{value,count}]`, including an empty array for empty buckets. Values retain JSON types:
the string `"1"` is different from the number `1`, and arrays and objects remain structured.
A missing field joins the `null` group. Groups are ordered lexically by canonical JSON
serialization, with recursively sorted object keys. Without `group_by`, neither `group_by`
nor `groups` is emitted.

These are **counts, not percentiles, rates, durations, budget readings or historical SLO
verdicts**. In objective mode:

- A count contributes one event per subject, at the earliest readable event's timestamp
  (claim id breaks ties), after the same `only` and `exclude` selection as the objective row.
  Retries do not add events or move one into a later bucket.
- An interval contributes one closed sample per subject, at its completion timestamp, using
  the same earliest start, latest expected set, earliest eligible stops, cancellation and
  exclusion rules as the evaluator. Expected members complete one sample, not one sample
  per member. An empty expected set closes at the later of start and expect. Open, cancelled,
  excluded and unstarted subjects contribute nothing, including censored open samples that
  enter the live percentile. Pauses affect duration, not the completion bucket.

The current declaration and all currently recorded context determine objective selection;
this is not an as-of replay. Starts and other context may precede `from`, and later retries or
expected-set changes can affect selection. Original numeric, RFC 3339 and JSON-encoded
timestamps are preserved and decoded by the shared timestamp reader.

Unknown objective names return `unknown-objective` (404). Shipped/native objectives and
declarations that are not readable declared counts or intervals return
`unsupported-objective-series` (422). Invalid ranges, bucket sizes or selector combinations
return `invalid-series` (422).

A rebuildable local timestamp projection indexes acceptance time and every readable timestamp
field by kind, field and unsigned timestamp. Its first startup backfills existing claims once;
insert/update/delete triggers capture changes, and append/projection transactions drain only
pending claims before publication, including replication. Each new claim parses its fields once;
storage is one acceptance row plus one row per readable timestamp field. The claim body is never
rewritten. Explicit-kind requests seek bounded timestamp candidates and fetch each candidate's
claim by its primary key, never the rest of its subject's history. Objective requests seek the
bounded kind/field/time range, then read only relevant kinds for candidate subjects through the
subject/kind index. Full candidate-subject history is necessary for objective pre-range starts
and retry deduplication. The range and bucket caps do not cap event density or an objective
candidate's history. No series request bootstraps or scans the claim log.

### Fractal's own CI

Fractal's Workspace CI objectives are declared, not shipped, in
[`slo/fractal-ci.kdl`](../../slo/fractal-ci.kdl), over the `github.workflow-run` claims the
repository's poller writes, one per completed run ([plugins](plugins.md#the-graph-kinds)). Every
member evaluates them from the replicated claims, so each reads the same rows:

- `fractal/ci-critical-path/p95`: an interval over successful first-attempt pull request runs,
  from `started_at` to `completed_at`, at most 13 minutes over a day. A rerun restarts its start
  and reruns only some jobs, so only a first attempt measures the run.
- `fractal/flakes`: a count of runs whose `flakes` is not 0 (a test nextest passed only on a retry,
  or a merge group ejected from the queue) over the last hour, at most 0 an hour. With `max=0` the
  row is missed exactly when any flake is seen; it counts runs, not flaky tests. Pull request runs
  of every conclusion are recorded, so a flaky test in a failed or cancelled pull request run counts
  too (the shipped `flakes` row it replaces read only successful pull request runs).

The obsolete `fractal/merge-latency` workflow-run proxy is no longer declared. It did not
measure ready-to-merged latency; source-backed lifecycle measurement above is the replacement,
and requires an authored source and effective-dated roster rather than inferred history.

Each names its owner in `on-breach`. Publish the file with `fractal apply slo/fractal-ci.kdl`
once every member runs a build that knows `only` and the `github.workflow-run` kind.

### Ratio over Prometheus metrics

There is none: Fractal does not read Prometheus. It exports its native readings as OTLP gauges
(`st_slo_*`), and Grafana computes ratios from them and from other exporters. An objective whose good
and total events exist only as Prometheus series stays in Grafana until its events reach Fractal as
claims or native telemetry.

## Breach response

```kdl
on-breach "fault" owner="agent/NS/NAME" backup="agent/NS/OTHER" condition="NAME-slo-missed"
```

A missed row opens one `operational.failure` on `daemon/MEMBER` addressed to `owner`, with the
`condition`. It stays one episode until the row is met again, when `operational.recovered` ends it.
A row with no data neither opens nor ends one. A row the objective no longer judges ends its fault:
an interval's label row whose samples have all left the window, a row a new revision no longer
has, or a count's label row, which is never met or missed. The member that delivers it is the one
the owner agent is declared on: every member evaluates the objective from the same replicated
claims, and one delivers. An owner that is not a live declaration gets nothing, and the
explanation says so.

`backup` is held: it is recorded and shown, and never becomes the fault's reviewer. It differs from
the owner.

Objectives Fractal ships open no fault.

## Publishing, revisions and replication

An objective is published with `fractal apply`, like an account. It is stored as the subject's
`intent.desired` claim, and its revision is the hash of the canonical declaration, which `fractal
apply --dry-run` prints as the old and new revision. The claim replicates to every member, so every
member evaluates the same declaration. A revision restarts the objective's evaluation from the claims;
nothing about past samples is stored apart from them. Retiring the subject removes the objective and
its rows.

A daemon that does not know the `objective` family refuses the replicated claim, so publish an
objective only after every member runs a build that knows it.

## Reading objectives

`fractal slo ls` lists every row, shipped and declared alike, from `GET /v1/slo`. `fractal slo explain
ID` adds, for a declared objective, its subject and revision, the sample counts by class, each open
sample over target with its age, and its response. `fractal slo chart ID` draws the budget left a
minute at a time from the local seven-day history, which records declared rows like any other.

`fractal doctor` presents every objective and account budget from the same native SLO
projection, including declared objectives, as `slo/ID`. It retains the native status, tone,
current reading, target and window. SLO verdicts describe, they do not gate: every row is
informational, so a red or quiet objective never fails `fractal doctor --strict`, and no SLO
row is presented as a successful check. Doctor does not re-evaluate SLO thresholds.

A declaration this build cannot read, as one replicated from a newer build, is the one row `NAME`,
`no-data`, whose explanation names its revision and why.

## Listing claims by kind or subject prefix

`GET /v1/claims` filters by `kind`, by `subject_prefix` or by an exact `subject`, and pages by store
index. `fractal claims ls --kind KIND --subject-prefix PREFIX` prints them. Both read through the
claims' kind and subject indexes. The interval and count indicators read their claims the same way,
keeping each objective's samples or events in memory and reading only claims newer than the last it
saw.

## Build admission lifecycle

`build/admission/HOST/CLASS` measures accepted eligible finite requests granted capacity
within two hours: 90% good over 30 days, with paired burns of 14.4× over 1h and 5m,
or 6× over 6h and 30m. The capacity grant is the stop, not payload launch or completion.
The denominator is resolved requests plus overdue open requests, not queue polls.
An open request fails once in its original deadline bucket; a later grant cannot move
that failure. Early refusal and withdrawal are failed admissions, not successful stops.
The native withdrawal interface has no typed intentional-supersession proof and therefore
does not silently exclude cancellation. Requests without a finite profile are separately
excluded. Impossible finite profiles are failures.

The host authority commits `resource.admission.observed` claims atomically with membership
changes, carrying accepted onset, grant timestamp, retirement/refusal facts, attempt owner,
original authority epoch and logical/enforced coverage mode. Replayable grants and runtime
custody are never copied into this stream. Authority acquisition replays existing membership
records once before committing its source cut. SLO reads incrementally consume only the
durable claim stream; they neither inspect the membership table nor create observations.
Child and inherited contexts do not create another membership sample. Repeated requests
retain their original identity and onset.
The native registry admits these system-only claims on `resource-admission/HOST/ID`;
`resource.admission.coverage` uses `resource-admission/HOST` for each acquired source cut.
Both kinds remain durable replicated evidence, not launch capabilities. Ordinary public
claim writers cannot publish either kind.
Evidence retains newer class names and the `recovery-not-allowed` refusal losslessly.
These facts do not extend the execution host's supported admission classes.
The daemon subscribes to claim-change notifications and arms one timer for the nearest
open deadline or 5m/30m/1h/6h/30d accounting boundary. The wake autonomously evaluates
the full native objective set and existing owner bindings even without an SLO reader.
Timers do not sample membership or manufacture lifecycle facts; an empty source has no
timer. There is no periodic admission-source poll.

Local native coverage is admitted only after this process acquires the host authority and
completes replay of supported membership classes. Retained unsupported-class custody is
still replayed as evidence without changing its authoritative bytes or grants, but the
source cut records `complete: false` and process-local coverage remains unknown. Supported
work may still acquire capacity under the existing custody and authority fences.
Unsupported queued rows never enter executable FIFO ordering, and background DSL cleanup
defers unsupported memberships instead of aborting supported reconciliation. Any held
custody still counts against capacity until a reader with authority can retire it.
A persisted marker cannot certify an absent producer after restart.
Remote evidence remains unknown: replication alone does not certify a remote complete cut.
Unreadable or uncertain clocks also leave the source unknown, never green. Coverage modes
remain visible in `coverage_modes`; a logical grant does not claim physical enforcement.

### Transitional gate producer

Gate admission receipts use immutable `gate-slot.admission/v1` accepted, granted, failed
and typed superseded edges, not payload completion receipts. Configure the gate producer's
`GATE_SLOT_ADMISSION_SPOOL` and the daemon's `FRACTAL_GATE_ADMISSION_SPOOL` to the same directory.
The producer defaults to `$XDG_STATE_HOME/gate-slot/admission` (or the standard user state
directory); the daemon requires an explicit source directory. `GATE_SLOT_ADMISSION_HOST`
must agree with the daemon's host identity when the machine's short hostname does not.
The reactive importer retains source files, replays idempotently on restart and publishes
`gate.admission.observed`; no periodic directory sampler or parallel SLO policy is involved.
The system-only `gate.admission.observed` kind uses `gate-admission/HOST/ID/OUTCOME`.
Each outcome receipt is immutable and retained durably; its schema admits the producer's
logical coverage only, without granting native execution custody.
Supersession before the deadline is visibly excluded; after the deadline it remains bad.
Unconfirmed wrapper death leaves an open sample, not an invented successful grant.

Gate evidence deliberately remains unknown until the deployment owner proves prospective
complete adoption: every authorized gate writer uses the instrumented revision, the complete
host/gate-root population is known, and pre-existing uninstrumented waiters have drained or
been reconciled. A receipt directory, one successful completion or a snapshot cannot prove
that older writers cannot introduce new unobserved requests. No historical coverage is
invented. Remove this transitional producer/importer when callers migrate to native resource
membership; do not remove admission safety as part of measurement.

## Agent mail delivery lifecycle

`agents/mail-delivery/<host>` measures durable agent messages accepted by a native harness
within **15 minutes**, with **95% good over 30 days**. A miss is counted once in its original
deadline bucket even while the message remains open, or after late acceptance. A terminal
close or expiry without input acceptance is bad, not successful delivery. The shared native
budget evaluator uses paired **14.4x (1h + 5m)** and **6x (6h + 30m)** burns, with one sample
as the minimum; sample counts remain visible. The 24h p95 and oldest open age are diagnostics,
not substitutes for the failed-message budget.

The producer joins graph sends, messages-shard sends and local lifecycle marks through
transaction-fed retained metadata. Shard lifecycle timestamps come from the source shard,
not from the mirror's later wall time; its dedicated replay cursor catches up existing
history. Retry and alias spellings collapse to one `(message, canonical agent recipient)`
sample. Current declared host ownership determines the host row. People are outside this
family; missing agent ownership is unknown, never a successful delivery.

The producer reads retained source metadata in indexed 128-row keyset pages and releases
each SQLite reader before processing another page or message's lifecycle. No read
transaction spans the 30-day scan, so WAL RESTART checkpoints can proceed during capture.
Local change bursts coalesce until at least 60 seconds after the preceding capture ends
(not a quiet-period debounce). Identity references are resolved once per distinct spelling
per capture. Pending changes and captures overlapping graph writes remain unknown rather
than certifying stale coverage or a mixed cut; idle heartbeat and boundary timers remain active.

`POST /v1/mailbox/receipts` accepts `lifecycle: "accepted"` with
`input_acceptance: { "accepted_at_unix_ms": ... }`. The native peer, physical provider
authority, recipient and current mailbox incarnation/epoch must all match. Acceptance
commits under the same transactional fence and preserves its first source timestamp.
It does **not** advance generic delivered/read state. Ordinary `message.delivered` or
`message.read` assertions cannot certify input acceptance. Historical settled mail without
a native acceptance certificate remains unknown rather than being manufactured as success.
Acceptance committed in a valid recipient incarnation remains historical evidence after
a later restart; a replacement cannot submit an earlier incarnation's unbound receipt.

Explicit silent/next-turn mail is shown in held diagnostics until its first durable staged
eligibility edge. It is never called delivered while held. Once eligible, its latency still
starts at the **original durable send**; hold time is visible but is not subtracted or reset.
Terminal undelivered held mail remains bad. Busy delivery-capable seats are not excluded.
An eligible open message remains visible after its failure bucket leaves the budget horizon.

`POST /v1/mailbox/acceptance-frontier` accepts a flat current mailbox fence and
`observed_through_unix_ms`. A native producer first seals/drains its source and durably posts
every acceptance through that cut. An unresolved handoff bounds the frontier before that
attempt; missing original handoff clocks cannot certify a frontier. This is not a daemon
heartbeat or receipt-pump wall-time assertion. The source joins each current recipient
incarnation to its frontier and evaluates budgets at the **minimum proven accounting cut**.
A source clock behind a message onset does not manufacture a deadline failure; that message
remains unjudged until the source cut reaches it. Frontier freshness is 120s.

Ledger attempt generations survive pruning, binding changes and reopen. A rejected RPC
resolves only its own generation; abandoned earlier attempts, including unknown source
clocks, continue to bound coverage until authoritative native absence resolves them.
Claude retains native acceptance timestamps independently of optional TODO journaling.
Frontier publication failures retain those bounds and produce the protocol-visible
`claude-acceptance-frontier-unsettled` waiting diagnostic, including after daemon recovery
acknowledges a previously unavailable publication; they do not write into the seat terminal.

The daemon starts one reactive source publication beside its roster producer. Source events
invalidate the previous cut immediately; capture failure, startup, missing shard mirror
coverage, missing ownership, missing/stale driver frontiers and malformed original clocks
produce unknown/no-data and cannot recover an owner episode. An idle source subscription
can renew its own capture heartbeat without querying mail, but cannot advance a native
producer frontier. One-shot lifecycle/budget/freshness boundaries wake evaluation; no source
polling or request-time sampling is used. Native ownership/response declarations bind this
host row through the existing native indicator mechanism, without a separate alert evaluator.

The local acceptance and lifecycle ledgers retain metadata across restarts and source-mark
retention. They contain recipient identity, source timestamps, sender/tags and lifecycle
provenance, not message content or model output. They currently retain this audit metadata
without automatic deletion; 30d applies to budget evaluation, not a destructive ledger TTL.

Capture does not aggregate these indefinitely retained ledgers on each event. Producer
transactions maintain a rebuildable pending-message key projection (unresolved or invalid
original send clocks), and an indexed send-time range supplies the current window. Capture
unions those populations, then reads lifecycle and certificate primary-key ranges only for
relevant messages assigned to this host. Completed history outside the window therefore
causes no historical lifecycle/certificate reads on unrelated daemon events. Startup replay
rebuilds this projection from durable metadata; no timer refresh or cached-success fallback
substitutes for producer facts.

