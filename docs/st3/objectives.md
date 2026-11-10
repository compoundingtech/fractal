# Objectives

An objective is a target for something Fractal measures, with an error budget and an optional
response when it is missed. There is one concept and one KDL kind, `objective`. Fractal ships its
own objectives (request latency, the store, CPU, CI) and a fleet declares more with `fractal apply`,
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
  `max`, `cores`, ...) and the bound is `within=DURATION`, `max=NUMBER` or `min-percent=NUMBER`.
  Each indicator names its rows (table below). A percentile over claims or CI is the row
  `NAME/pNN` (`ops/change-rollout/p95`), one per target.
- `window` is the span the budget is measured over, and `min-samples` the samples it needs before it
  is met or missed; with fewer it is `no-data`. Only an indicator over claims takes a window.
  Native telemetry keeps its own windows, below.
- `on-breach` is what happens when a row is missed. It is optional; without it a miss is shown and
  nothing is delivered.

A missing, unknown, repeated or nonsensical child is refused, never defaulted. An objective is a root
declaration; it cannot sit inside a mission.

## Indicators

### Native telemetry

Fractal measures these itself, in memory, on every member. They are the objectives Fractal ships, in
[`slo/objectives.kdl`](../../slo/objectives.kdl), which is generated from
[`slo/objectives.kdl.genie.ts`](../../slo/objectives.kdl.genie.ts). A fleet cannot declare them:
each path, store series and CPU reading counts toward exactly one objective, and that is the one
the daemon was built with.

| Indicator | Measures | Targets | Rows | Windows |
|---|---|---|---|---|
| `latency` | Each `path`: a daemon route (`GET /v1/client/agents`), a stream's first snapshot (`stream agents`), a refresher's publication (`publish missions`) or a paired phone's report (`client/ios/connect`) | `p99 within=`; again with `scope="remote"` for the same reads served from another member | `NAME`, `NAME/remote` | 1m, 5m, 1h; budget over 1h |
| `share` | A phone's live foreground time over its foreground time, at `path` | `live-share min-percent=` | `NAME` | 1m, 5m, 1h |
| `statement` | One SQL statement | `p99 within=`, `max within=` | `NAME` (the p99) | 1m, 5m, 1h |
| `transaction` | Read and write transactions and writer holds | `max within=` | `NAME/read`, `NAME/write`, `NAME/writer_hold` | 1m, 5m, 1h |
| `writer-hold` | Each writer hold and long read, by holder | `max within=` | `NAME` | 1m, 5m, 1h |
| `cpu` | The daemon and its replication workers over closed UTC minutes; children `idle-min-minutes` and `load-max-cores` | `cores max=`, `idle-cores max=` | `NAME`, `NAME/idle` | the hour |
| `database` | The store's pages in use and their daily growth | `live-gb max=`, `growth-mb-per-day max=` | `NAME` | the newest hourly measurement |
| `divergence` | Open projection divergence faults | `open max=` | `NAME`, once a divergence check has run | now |
| `ci-duration` | A CI `series` (`ci/merge-queue`) read by the repository observer | `pNN within=` | `NAME/pNN` | 1d |
| `ci-count` | Events in a CI `series` | `per-hour max=` | `NAME` | 1h |

The statement, transaction, writer-hold, CPU, database and divergence objectives are one each,
named as the table's rows say (`sql-statement`, `transaction`, `writer-hold`, `cpu`, `database`,
`projection-divergence`): `fractal doctor` and the OTLP export read them by these names.

These keep the meanings they had as built-in targets: a p99 is missed in a window when more than
1% of its samples are over, a max when any one is, and CI rows are no-data on a member that reads no
Workspace CI runs.

### Interval over claims

`interval` measures work that starts and stops in different processes and on different members,
from claims those processes write. Each subject under `subject-prefix` is one sample.

| Child | Meaning |
|---|---|
| `subject-prefix "P"` | The sample subjects: every subject that starts with `P`. Custom claims (`custom/NS/...`) need no registration and replicate to every member. |
| `start KIND at=FIELD` | The sample starts at the earliest such claim. |
| `expect KIND set=FIELD` | The members a sample must stop on, a list of strings in the newest such claim (by its time, then its claim id, so every member picks the same one). An empty list stops the sample at the expect claim's time. Without `expect`, one stop claim ends the sample. |
| `stop KIND member=FIELD` | One member has stopped: the earliest such claim per value of `FIELD`. The sample stops when every expected member has, at the latest of their times. |
| `cancel KIND` | An open sample with such a claim is cancelled. A sample that stopped before the cancel was accepted keeps its value. |
| `pause KIND resume=KIND` | The clock stops from each pause to the next resume, and a sample still paused is not aging. Pauses and resumes are timed when they were accepted. |
| `label "FIELD"` | Also one row per value of `FIELD` on the start or expect claim: `ops/change-rollout/p95{class=routine}`. A sample without the field is only in the unlabelled row. `label "member"` splits by member, each sample's value being that member's stop: the expected members, or without `expect` the members that stopped. |
| `exclude "FIELD" "A" "B"` | A sample whose start or expect claim has one of these values is excluded. |

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
A row with no data neither opens nor ends one. The member that delivers it is the one the owner
agent is declared on: every member evaluates the objective from the same replicated claims, and one
delivers. An owner that is not a live declaration gets nothing, and the explanation says so.

`backup` is held: it is recorded and shown, and never becomes the fault's reviewer. It differs from
the owner.

Objectives Fractal ships respond as they always have: a missed CI row is a fault for the fleet fault
agent.

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
A declaration this build cannot read, as one replicated from a newer build, is the one row `NAME`,
`no-data`, whose explanation names its revision and why.

## Listing claims by kind or subject prefix

`GET /v1/claims` filters by `kind`, by `subject_prefix` or by an exact `subject`, and pages by store
index. `fractal claims ls --kind KIND --subject-prefix PREFIX` prints them. Both read through the
claims' kind and subject indexes. The interval indicator reads its claims the same way, keeping each
objective's samples in memory and reading only claims newer than the last it saw.
