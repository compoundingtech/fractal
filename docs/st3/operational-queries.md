# Operational queries

## Bulk desired declarations

Use `fractal subject desired --json [--all | SUBJECT...] [--kind K]` to read desired
declarations and selected declaration tokens with cheap actual summaries, without
reducing status or folding cards. The existing CLI noun is `subject`, singular.
An empty subject selection means all declarations; explicit names are exact IDs.
The API is `GET /v1/desired?subject=ID&subject=ID&kind=K&limit=N&cursor=TOKEN`.
Repeated URL-encoded `subject` parameters select exact names; omitting them selects all.
`kind` and `cursor` are optional. At most 1,000 supplied names are accepted
(`desired-batch-limit` otherwise). Page limit defaults to 1,000 and cannot exceed 1,000;
zero or an over-limit value returns HTTP 422 `desired-page-limit`.
Each page is `{"items":[...],"next_cursor":null,"store_index":123}`: `items` contains
flattened `DesiredSubject` entries plus `desired_token` and optional extensible `actual`,
`next_cursor` is an opaque string or `null`, and `store_index` is a `u64`.
All sources for a page are read in one store snapshot.

The cursor binds the store origin, pinned claim read cut, shared graph projection progress,
desired row-write generation, selection filters, limit and last subject. The graph projection
frontier covers both selected desired declarations and mission definitions; backlog projection
can advance it without appending a claim or changing `store_index`. A durable per-store
generation advances transactionally on every desired row insert, update or delete, including
repair that leaves the claim cut and graph health unchanged. Continue with unchanged subjects,
kind and limit. Each continuation validates the claim cut, graph health/frontier and row-write
generation with bounded reads in the same pinned snapshot; any change returns typed
`desired-cursor-expired`, never a silent
restart or retry. No historical snapshot or pagination cache is retained. The CLI follows
every cursor with limit 1,000 and unchanged
filters, then prints one final JSON array. Any page failure, including cursor invalidation,
fails the command without partial stdout.

This GET inherits the single desired GET's `read.projections` authorization, deadline and
read admission, including refusal of paired devices without that scope. It remains a read
during fleet drain and performs no status reduction or cold card folds.
See the [CLI reference](../site/reference/cli.md#bulk-desired-declarations) for the
selection and response contract. Missing actual fields are not negative observations:
fetch the card with `fractal subject show SUBJECT` or `fractal agents show AGENT`.

The kind filter matches the literal desired declaration kind. Stopped and terminally
retired agents have kind `stop`, not `agent`; unfiltered all includes their retained
declarations. Agent revision summaries also apply to those retained declarations.
Synthesized published mission entries have `desired_token: null`, matching mission
cards, because mission publications are not desired-table rows. Their publication
claim IDs are not replacement desired tokens; compare `actual.revision` and
`actual.digest` for declaration changes.
For agents, `actual` contains only the maintained published declaration `revision`
when available, matching the card's desired content revision. It omits runtime
`status`, `operation_status`, `state`, `host`, `incarnation_id`, `suspension`, `hold`,
`kind`, and `repo`, and card-level `actual_origin` and `projection`
(`layer`, `runtime_incarnation`). Those have no cheap, correct maintained source
for this snapshot path: fetch the card. For bulk roster status, use
`fractal agents ls --all --json`; roster fields such as `state`, `host_id`, and
`operational` are not interchangeable with card state, host, or projection fields.

A retained member containing an unsupported enum variant anywhere in its schema,
or an unknown field in a structure that rejects unknown fields, produces
`actual: {"unavailable":{"code":"member-version-unproven"}}` for that item,
instead of failing the batch. Its declaration and selected token remain in the page;
the current daemon omits `member` when it cannot decode it. The unavailable marker
replaces the normal actual summary, so it carries no revision. Other items keep their
normal summaries. The authority is the full `MemberSpec` serde schema, including
nested discriminators and `deny_unknown_fields` structures, not a numeric wire version.
Malformed JSON, wrong types, missing required fields, database, mission,
declaration-body and other unrelated failures still fail the page.
The CLI retains raw extensible item JSON from newer servers, including member
payloads it cannot itself decode, and preserves unavailable markers.

Non-agent actual summaries contain the published declaration `revision` and `digest`,
plus `state` when cheaply maintained (`draft`, `ready`, or `retired` for missions).
They never contain the published body. `digest` is `sha256:` followed by lowercase
hexadecimal SHA-256 of the UTF-8 canonical JSON text of the published declaration:
recursively sort object keys, preserve array order, and use compact JSON with no
whitespace. The input is the declaration itself, not the surrounding
`PublicationDefinition` metadata, revision, or selected desired token.

The raw paged desired API is not a client-v0 operation. The committed Rust, Swift and
TypeScript clients are generated from `docs/st3/client-v0/schemas/operations.json`
and `client-v0.schema.json`, so this route does not change those inputs or outputs.
Their regeneration command is `cargo run --locked -p fractal-client-codegen`;
it also invokes `rustfmt` internally. No generator execution is needed for this route.

## Single-subject status reads

`GET /v1/status?subject=SUBJECT` and the CLI's shared single-subject status helper retain
the full canonical claim ID vector. Subject `show`, `inspect`, and successful non-verdict
`trace wait` output therefore retain claim provenance; `claims=[]` still proves an empty
history, not a skipped reduction. Bounded `recent_claims` reads are unchanged. There is no
`omit_claims` HTTP mode.

Owned-set preview and resource/member reads use an internal `Fields` reduction instead.
It skips claim IDs, counts and session boundaries, then returns only actual state across
the store boundary. No `SubjectStatus` or unenumerated `claims=[]` is serialized on that path;
owned-set wire shapes remain unchanged. This avoids provenance work under those snapshots,
but actual-state and harness folding can still depend on history.
A desired-only caller should use `GET /v1/desired/SUBJECT`,
which reads the desired record without reducing actual state or claims.

## Published operational lists

Mission lists page mission IDs and return compact cards with at most three recent runs and
twenty step previews per run. Progress counts cover every step; a larger total signals an
incomplete step preview.
`active_runs`, `total_runs`, and `run_counts` count the entire history. `runs_truncated` says when
the preview is incomplete. A page also respects a byte budget; its cursor resumes after the last
returned mission. Full step details remain available by exact run subject.

The default `GET /v1/client/missions` page serves the background-published current list,
including its already-folded attention, run counts and step previews. Polling does not fold
missions again. The snapshot reports the publication's graph cut and `published_at`; a bounded
TTL cache keeps continuations on that publication across refreshes. Projection replacement,
expiry or cache eviction requires restarting pagination. Before a first complete publication,
or while none can be served, the API returns retryable HTTP 503 `mission-list-not-ready`.
If its refresher stops, HTTP 503 `mission-list-ended` is not retryable and requires a daemon
restart. History (`history=true`) and exact detail reads remain direct. Explicitly disabling
published lists with `ST3_PUBLISHED_LISTS=off` retains direct reads.

With metrics enabled, `st.mission_list.publication.age` is a seconds gauge of the newest served
publication's age (absent before publication); `st.mission_list.rebuild.duration` is a seconds
histogram of rebuild attempts, including failed attempts. The existing single-flight refresher
coalesces commits, reads large folds in chunks, and pauses between attempts rather than letting
polling increase rebuild concurrency.

Current `/v1/client/runtimes` pages serve the latest complete background publication,
under that publication's snapshot and `published_at` time. Reads clone only the requested
page: they do not reduce runtime status or hash the complete collection. The shared
collection refresher coalesces commits, runs one rebuild at a time, and pauses at least
one second and as long as the preceding rebuild between attempts. Before the first
publication, reads return HTTP 503 `runtime-list-not-ready` with `retryable=true`; a
stopped refresher returns `runtime-list-ended` with `retryable=false` and requires a restart.
Continuations retain the first page's frozen collection until its existing cursor expiry,
including across republishes. Repeated first-page reads reuse that collection's cache entry
and original expiry. Terminal permissions are applied to
each returned page for its authenticated caller. `history=true` and exact runtime detail
reads keep their existing snapshot reductions.

Machine inventories and summaries reuse these runtime rows when the publication is
servable and its graph cut exactly equals their already-pinned snapshot. They apply
the caller's terminal permissions to private copies, never the shared rows. They
retain direct snapshot reductions for missing, unavailable, older or newer
publications, history and exact-owner reads. An older runtime publication cannot
be joined to newer work/fleet membership while advertising one inventory snapshot;
the inventory does not silently move its cut to the publication's.

OTel metrics `st.runtime_list.publication.age` (gauge, seconds) and
`st.runtime_list.rebuild.duration` (histogram, seconds) distinguish stale publications
from slow rebuild attempts. Publication age is absent until a complete list is available;
rebuild duration includes failed attempts. The background rebuild still reduces the full
runtime collection; publication removes that work from ordinary list requests.

```sh
fractal missions ls --all
fractal missions ls --all --cursor 'CURSOR'
fractal missions show mission/example/build
```

Showing a mission with multiple runs prints counts by state, and two windows: the ten
newest runs and the ten newest failed runs with reasons. Each window says so, and names
the full-history read: `fractal missions ls --all --mission mission/example/build`.
Showing a mission with one run opens that run. `--follow` requires choosing an exact
run when the mission has multiple runs. The published missions list is the same kind of
window: `active_runs`, `total_runs`, and `run_counts` count the entire history, while the
card previews at most three runs and `runs_truncated` says when the preview is incomplete.

To investigate outcomes in a time window, or every terminal transition of one mission:

```sh
fractal missions ls --since 6h --status failed
fractal missions ls --all --mission mission/example/build
fractal missions ls --all --mission mission/example/build --status failed
fractal work ls --since 6h --status timed-out
fractal work ls --since 2026-10-01T00:00:00Z --until 2026-10-01T06:00:00Z --status cancelled
```

`--since` and `--until` accept RFC3339 timestamps or durations in milliseconds, seconds,
minutes, or hours. Supplying either time filter, `--status`, or `--mission` selects terminal
transition history. Status accepts `failed`, `cancelled`, `timed-out`, and `completed`; without a
status filter, all terminal transitions are returned. A timeout is a failed transition whose
recorded reason names a timeout. Cleanup sometimes omits the reason in the terminal claim;
the reader recovers it from an earlier transition for that outcome. `--mission` keeps one
mission's terminal transitions across every page (`/v1/outcome-history?collection=missions&mission=...`);
other missions' runs never appear. Running and waiting runs are not terminal transitions and
never appear in this history: read `/v1/mission-runs?mission=...` for the active runs.

These are events, so retries preserve earlier failures and a run can appear more than once.
Times are the graph claims' accepted timestamps. `--as` on work also filters by the recorded
actor or step assignment. Pages contain reasons and their claim IDs; the continuation printed
by the command fixes the original time window and `--mission`. Resume using that command without
repeating relative time filters. JSON includes `next_cursor`, and an exhausted page has `has_more=false`.
History cursors and page order are local to the serving daemon, like raw claim-history cursors;
they are not shared state folds. Reason selection uses canonical claim order, including recovery
from an earlier transition, so replicated arrival order cannot choose a different reason.
A daemon that predates `--mission` answers the unfiltered page; the command refuses it with
"this daemon does not support --mission; upgrade the daemon" instead of mislabelling it.

Native lease polling uses `GET /v1/work?actor=...&view=renewal`: only the actor's
currently held claimed/working/verifying/blocked steps, with owner and lease validity applied. This view
does not populate execution timing, queue, wake, summaries or agent hierarchy. The
default work list remains fully enriched. Performance tasks distinguish `work/renewal`
from `work/list`; native diagnostics, timeline binding, wait interruption and subagent
attribution have fixed caller labels too. Labels are never actor-derived.

Conversation routing resolves an exact agent's runtime and incarnation at the request's captured
claim frontier. Sparse identity fields use newest-first partial-index seeks, including explicit
nulls and state-transition resets, rather than replaying every heartbeat or harness report.
Runtime authority selects the newest canonical observation per origin; rival origins retain
the desired-host and checkpoint-aware ancestry rules. Resource identity fields use the same
bounded seeks; attribution-only opener receipts do not replace identity. These reads introduce
no routing cache or wire format change.

Transport routes likewise seek the newest canonical observation per observer/host pair instead
of ranking the complete transport history. Original success timestamps, clock-skew limits,
expiry, and dial-out membership exclusions still decide whether that evidence is usable.

```sh
fractal doctor
fractal doctor --performance
fractal doctor --performance --json
```

The daemon always keeps bounded request, background task, and SQL timing aggregates for the
last five minutes, in ten-second buckets. Doctor reports the twenty request/task kinds and
twenty query templates with the most total wall time, including counts, mean time, maximum
time, and recorded thread CPU for requests/tasks. Request CPU includes store work dispatched
to other blocking workers. SQL literals and comments are removed before aggregation. SQLite
statement wall time includes row processing, and concurrent statement times overlap.

Doctor also counts requests by client over the same window: the total request count and rate,
the twenty clients that sent the most requests, and the twenty client and request-kind pairs with
the most requests, each with total wall time and CPU. A local client is named by its first command
words under the agent it is bound to, such as `agent/example · st3 driver claude`; requests from
fabric or TCP peers count as `(tcp)`. Idle load is usually many cheap requests, so these rows are
sorted by count.

It also counts what woke the reconciler: each request that changed the graph, by client (and
claim kind for `/v1/claims`), replication receives, the reconciler's own timers and deadlines.
Every member runs a reconcile pass for each change in the fleet, so this table names the writes
that keep an idle daemon busy. Wakes that arrive during a pass share the next one.

The fault snapshot that delivers faults to their owners keeps each operational failure's answer
and the keys its reading noted (its source, its targets, its run and the runs above it, their
steps and declarations). A snapshot reads a failure again only when the change feed names one of
those keys, when its time comes, or when its answer rests on a read the feed does not carry (a
when-idle restart's release, a deferred stop's repairs, a provider login's person, a missing
agent's eligibility). Every five minutes it reads every failure again and compares: a different
answer is counted under `incremental_corrections` as `fault-snapshot` and means a read is missing
its key. After a day with no correction the full reading comes every half hour, and any correction
brings it back to every five minutes. The cache is in memory, so a daemon start reads everything.

Native mailbox streams report their store work as `task mailbox-change-check` and
`task mailbox-snapshot`, including CPU time. These tasks continue after the subscription request
upgrades to a WebSocket, so their cost appears separately from that initial request. They count
as background tasks, leaving the request count and per-client request rates unchanged.

This accounting writes no graph claims and requires no file profiling configuration. Samples
are local to the daemon and disappear on restart. `--performance` requests the timing report
without running dependency checks. File profiling remains available for deeper investigation.

The local API serves `/v1/mission-overview?mission=...`, `/v1/outcome-history?collection=...`,
and `/v1/performance`. Outcome API times are epoch milliseconds; `before` resumes by immutable
claim index and `limit` is between 1 and 200. `mission` on outcome history keeps one mission's
terminal transitions; `status`, time window, and cursor paging compose with it unchanged.
