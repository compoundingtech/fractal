# Objectives

An objective is a target for something Fractal measures, with an error budget and an optional
response when it is missed. There is one concept and one KDL kind, `objective`. Fractal ships its
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
- `window` is the span the budget is measured over, and `min-samples` the samples an interval needs
  before it is met or missed; with fewer it is `no-data`. Only an indicator over claims takes a
  window.
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

The statement, transaction, writer-hold, CPU, database and divergence objectives are one each,
named as the table's rows say (`sql-statement`, `transaction`, `writer-hold`, `cpu`, `database`,
`projection-divergence`): `fractal doctor` and the OTLP export read them by these names.

These keep the meanings they had as built-in targets: a p99 is missed in a window when more than
1% of its samples are over, and a max when any one is.

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

### Fractal's own CI

Fractal's Workspace CI objectives are declared, not shipped, in
[`slo/fractal-ci.kdl`](../../slo/fractal-ci.kdl), over the `github.workflow-run` claims the
repository's poller writes, one per completed run ([plugins](plugins.md#the-graph-kinds)). Every
member evaluates them from the replicated claims, so each reads the same rows:

- `fractal/merge-latency/p50` and `/p95`: an interval over successful first-attempt merge group
  runs, from `created_at` (the group entered the queue) to `completed_at` (its run went green), at
  most 10 minutes over a day. It is a proxy for entering the queue until merging: the wait before
  GitHub creates the group's run and the merge after it are not counted.
- `fractal/ci-critical-path/p95`: an interval over successful first-attempt pull request runs,
  from `started_at` to `completed_at`, at most 13 minutes over a day. A rerun restarts its start
  and reruns only some jobs, so only a first attempt measures the run.
- `fractal/flakes`: a count of runs whose `flakes` is not 0 (a test nextest passed only on a retry,
  or a merge group ejected from the queue) over the last hour, at most 0 an hour. With `max=0` the
  row is missed exactly when any flake is seen; it counts runs, not flaky tests. Pull request runs
  of every conclusion are recorded, so a flaky test in a failed or cancelled pull request run counts
  too (the shipped `flakes` row it replaces read only successful pull request runs).

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
A declaration this build cannot read, as one replicated from a newer build, is the one row `NAME`,
`no-data`, whose explanation names its revision and why.

## Listing claims by kind or subject prefix

`GET /v1/claims` filters by `kind`, by `subject_prefix` or by an exact `subject`, and pages by store
index. `fractal claims ls --kind KIND --subject-prefix PREFIX` prints them. Both read through the
claims' kind and subject indexes. The interval and count indicators read their claims the same way,
keeping each objective's samples or events in memory and reading only claims newer than the last it
saw.
