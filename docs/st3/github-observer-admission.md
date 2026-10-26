# Local GitHub observer request meter and admission

The response meter previously counted only completed resource-provider HTTP
responses in memory. A failed send or restart could lose usage. This change
reserves configured daemon GitHub HTTP requests before local reqwest or the
existing authorized RPC runs. `STATE/github-admission.json` retains the last
hour's reservations; doctor reads them as
`github-requests/admitted/CALLER/RESOURCE` even before a new request after restart.

**Default: meter only, no quota enforcement.** The optional DAEMON setting
`[github].observer_requests_per_hour = 1000` enables the Factory allocation, per
resource and per caller, with a shared aggregate of1000 rather than1000 for each
observer. This is a local config setting, not a GitHub setting. Values1..1000 can narrow the Factory ceiling; raising it requires a new policy
review. No live config is changed. Existing production observer usage is UNKNOWN in this source review.
Before setting a cap, the owner must measure current per-resource request rates
including pagination at the5-minute interval and confirm quota/freshness headroom.
Meter-only I/O failures are visible gaps and do not stop polling; enforced
observer admissions fail closed on an unproven durable write. Unscoped commands
remain metered and uncapped; a broken meter must not block merge writes.

GLOBAL-VIEW: no. No other machine, distributed lock or shared-token reservation
is consulted. The existing repository observer and shared per-URL cache remain
the only poller. A local per-process mutex serializes this daemon's journal. The
caller is the reconciler's task-local `spend_as` subject, never ST_AGENT or an HTTP
header. Unenrolled direct comment/merge-gate traffic is `unattributed`; authentic
per-agent write admission remains a separate obligation.

Core, GraphQL and search use separate request counters (including GitHub
Enterprise `/api/graphql` and `/api/v3/search` path shapes without relaxing any
existing transport-domain policy). These are NOT GraphQL points, search-provider
or secondary-limit guarantees. Failed sends and conditional304s remain reserved.
Refusal adds no reservation and returns a typed `LocalObserverBudget`, displayed
as “local observer budget reached, next slot at T”, not a GitHub HTTP 429.
It persists the next local slot without raising a provider-rate-limit fault.

GitHub HTTP 429 and secondary-limit 403 responses honour `Retry-After` (seconds
or HTTP date) and `x-ratelimit-reset`, using the latest future deadline. If neither
header names a future deadline, the observer waits fifteen minutes; a stale
primary reset cannot turn a secondary limit into a one-second polling loop.
Every changed refusal deadline is persisted, including when the HTTP status and
reason have not changed, so attention evaluates the latest reset rather than
the first refusal's expired reset.

Each REST page, check/status sub-request and GraphQL query (including a smaller
query retried after a GraphQL execution error) passes admission separately.
REST read redirects are followed explicitly, up to ten hops within the same API
origin, with a new reservation and response attribution for each hop. Cross-origin
redirects are refused rather than forwarding credentials. Mutations are sent once,
without following redirects. Automatic transport retries are disabled in the
observer and authorized gateway clients; an ordinary HTTP 5xx is not retried
inside a send. A reservation can still correspond to no
completed HTTP request when a connection fails, so the ledger remains a
conservative attempt ceiling rather than proof of exact provider debits.

Provider header history is separate: `STATE/github-admission.json.budget-history.json`
retains the first actual `limit`, `used`, `remaining`, and reset sample for each
resource/reset/minute, for up to 48 hours and at most 8,192 samples. These retain
provider units: core requests and GraphQL points are never added together or
inferred from each other's request counts. Sampling adds no HTTP request and
publishes only a new minute/reset sample, not every response. The ring uses the
existing bounded private-file publication path; corrupt history is not silently
overwritten. A history I/O gap is visible in doctor and never changes admission
or the actual HTTP outcome. The enforced reservation window remains 61 minutes.

Counters are coalesced per minute/caller/resource, including unscoped traffic:
a100000-call unscoped flood in one minute occupies one row and cannot consume
the observer quota or fill its row allowance. Whole-minute expiry is conservative
by at most a minute, so doctor reports a61-minute upper window. Small backward
clock steps preserve counts and clamp future timestamps to now; timestamps more
than a day ahead remain invalid. A cold read never silently resets bad data.

Publication writes an exclusive private temporary, fsyncs, renames atomically and
syncs the parent before send. A directory-sync failure invalidates memory so the
next attempt rereads any committed reservation. Configuration removes stale
private regular temporaries in a bounded scan, without following symlinks; UUID
names avoid pid-reuse collisions. Doctor try-locks rather than waiting for disk
sync, and limits output to127caller/resource groups plus an aggregated other row
and any fault line. Expected fsync latency has NOT been measured; minute buckets
reduce file size, but there is still a durable publication per admitted request.

Conditional REST behavior is unchanged: valid retained ETags produce
If-None-Match on revalidation and a304 preserves full bodies/page links. First
reads and endpoints without ETags cannot supply a valid conditional header.
GraphQL has no ETag revalidation. Its shared repository minimum is now5minutes
(was60seconds), settled remains15minutes, explicit refresh remains explicit.
This visibly affects st gh watch wakes and observer-backed PR/check/queue UI
freshness, up to5minutes while settling. Curator's source page says live merge
and ci-passed gates fetch current data and enqueue/merge are direct writes; this
patch does not replace those live gates with cached PR facts. Consumer coverage
must still be qualified on the deployed build. No automatic skip or stale-head
approval follows from the cache or this meter.

Rust controls are written and have not been run locally:
restart/expiry and aggregate limits, separate resource counting, refusal without
spending, persistence without a response, corrupt/gross-future state, symlink
refusal, unscoped flood, tolerated skew, meter-only policy, enforced failure vs
unscoped availability, stale temporary cleanup, and doctor contention. The new
module parses with rustfmt and the diff passes whitespace checks. Existing
resource tests cover shared lists and200/304/head-change behavior. Exact-head
Intake review, ordinary hosted qualification and later deployment evidence are
required; no live admission or savings claim is made here.

## Repository aliases and response accounting

Standing repository observers with the same observed numeric `repository_id`
share one poll across the fleet declaration union. The lexicographically first
observer supplies the poll. An observed rename target with the same numeric ID
supplies the shared locator and resource; otherwise the elected observer's
resource is shared. That target remains shared when its directly named watches
end, preserving the surviving aliases' item history. A pending explicit refresh
temporarily selects the addressed alias to complete that refresh on the same
resource. Each alias keeps its existing watch subjects, deadlines and recipients.
The elected observation reads the union of their fields and fans out to every
subscription, including those on a different spelling of the repository.
Custom non-standing observers are not coalesced. An unknown identity must first
be observed, so startup discovery can still read both aliases before their IDs
are known and replicated. Deduplication is not a distributed admission lock.

`st.github.responses` is a monotonic OpenTelemetry counter with
`github.repository`, `github.resource` (`core`, `graphql` or the provider's
reported resource) and `github.response.kind` (`charged`, `not_modified`).
The daemon's `host.name` resource attribute supplies the host dimension.
`charged` means a completed non-304 response, including refused responses; it
is not proof of an exact provider debit and never represents GraphQL points.
`not_modified` counts completed conditional 304 responses. Failed sends appear
only in admission reservations, not this response counter. Requests satisfied
entirely by the local cache appear in neither counter. Compare rates of the
two response kinds with the account's provider rate-limit observations rather
than treating `github-admission.json` reservations as charged requests.

The existing conditional cache retains JSON and pagination links on a 304 and
authenticates every revalidation. Standing observers with no live watch or
mission subscription stop; stopping a seat's runtime alone does not end its
watches. These behaviors have dedicated regression coverage.

## Stable inclusive listing boundaries

Repository item and comment cursors advance only to returned `updated_at`
timestamps, not the polling clock, so idle polls keep stable request URLs.
GitHub documents `since` as exclusive: requests subtract one second from the
stored watermark and deduplicate overlap by identity, update time and content
digest, so a second edit within one timestamp second is not dropped. The cursor
retains boundary versions during ordinary polls. When a read exceeds ten pages,
it freezes the starting watermark, retains sweep versions and saves exact Link
continuations. Before advancing the watermark it revalidates from that starting
point until two complete ordered page-version sequences agree. This recovers
rows shifted into already-read pages; repeated shifts require further sweeps.
Each poll still reads at most ten pages per listing. After two nonmatching completed
sweeps, immediate reconciliation is exhausted: preserve both scan states and frozen
watermarks, return the ordinary provider deadline, and let the observer use normal
cadence instead of one-second continuation. Further pagination also remains at
normal cadence until the backed-off scan converges. Thus continuous churn cannot
keep reconciliation one-second-hot or force an unsafe watermark commit. Continuous
churn can defer watermark advancement: REST offset pagination cannot give an atomic
snapshot. Sweep bookkeeping uses fixed-size digests/counters; deduplication keeps
the active sweep identities and prunes only after commit.
Optional cursor fields default on old records; no storage version change,
migration or backfill is required.

Cold caches, changed URLs and endpoints without ETags still need ordinary
responses. Authenticated idle revalidations return 304 and preserve full bodies
and pagination links. Cache warmup after restart is not quota-free.

## Bounded admission attribution

`STATE/github-admission.json` keeps the original strict reservation shape:
`at`, `caller`, `resource`, `observer`, and `count`. Closed endpoint and response
classes live only in the separate, bounded
`STATE/github-admission.json.attribution.json` diagnostic sidecar, never in the
admission ledger and never as raw URLs or query strings. Repository, issues and
issue-comments are separate classes. Responses distinguish 200, conditional 304,
other success, redirect, client/server errors and transport errors. Reservations
without sidecar attribution remain unknown without losing spent admission.
Doctor reports endpoint/status counts, conditional 304 hits and conservatively
possible quota-bearing reservations. The local cap still includes 304s: provider
quota and local admission are not the same meter. Only the original ledger
controls admission safety. Pending and completed sends publish attribution
separately; sidecar read/write failures expose a diagnostic gap without blocking
HTTP or replacing its response. Rollback binaries never read the sidecar and can
still decode every reservation written by this version; no ledger migration or
deletion is needed, and requests admitted by older binaries appear as unknown
attribution after upgrading again.

## Shared-token fleet budget: design only

Two complementary options remain outside this local change:

* **Single observer owner per repository across the fleet:** use a lease/election
  and replicated observations, defining failover/freshness before suppressing
  duplicate owners. Alias grouping uses the existing ownership model; it is not
  a shared-token reservation service.
* **Backoff from `X-RateLimit-Remaining` and `X-RateLimit-Reset`:** defer observer
  reads below an agreed shared reserve while foreground merges and agent `gh`
  calls retain headroom. Scope metadata to credential and quota resource, handle
  concurrent out-of-order responses conservatively, and spread reset wakeups.
  Headers observe shared quota; they do not atomically reserve it.

No header backoff is implemented: reserve sizing and starvation/freshness policy
need fleet ownership and operational evidence. Desired-stop watches remain a
follow-up: suppressing them without catch-up can permanently miss a terminal
transition if another subscription advances the shared repository facts. Retired
seat declarations already end their watches in normal reconciliation. No stored
watch backfill or live-daemon change is part of this patch.
## Adaptive standing-watch cadence

The single standing `github.repository` observer shared by a repository's watches
now starts at three minutes. Successive unchanged observations back off to six
then ten minutes (the ceiling); any changed observed fact resets the next poll
to three minutes, even when that fact does not wake a particular subscription.
There are still no per-thread pollers. Existing standing declarations with
`every "30s"` adapt by standing-observer identity, without live-state migration.
New standing declarations use `every "3m"`. User-authored non-standing observers
retain their explicit intervals, including faster thirty-second polling.

Cadence is local scheduling state keyed by observer/revision, independent of the
retained provider cursor. After a new revision or daemon restart, the observer polls
once immediately unless a provider rate-limit retry deadline is active, then follows
three-minute active cadence and quiet backoff. Explicit refresh still runs immediately
and resets cadence. Normal scheduling deadlines
are not persisted in receipts (only their hashes are recorded); effective
six/ten-minute deadlines live in the scheduler. Backoff uses the writer's canonical
observation outcome across all fields, not raw delta JSON or receipt writes.
An identity-only quiet delta after an item change advances to six minutes;
re-emitting the same item does not count as another change.
Pagination still continues at one second without advancing quiet backoff, and
provider rate-limit errors retain their retry deadlines, including older receipts
without a revision and after desired revision changes. This changes standing
watch freshness to three minutes while active and up to ten minutes while quiet;
it does not change direct merge/check gates or claim measured live savings.
