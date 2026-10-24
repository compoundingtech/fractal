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
Refusal adds no reservation and returns the existing typed ProviderRateLimit.

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
retained provider cursor, ETags and response bodies. A new revision or process
starts active cadence; explicit refresh runs immediately and resets cadence.
The persisted successful-observation deadline is the conservative three-minute
restart bound; effective six/ten-minute deadlines live in the scheduler, not in
the receipt. Backoff uses the writer's canonical observation outcome across all
fields, not raw delta JSON or whether a health/refresh receipt was written.
An identity-only quiet delta after an item change advances to six minutes;
re-emitting the same item does not count as another change.
Pagination still continues at one second without advancing quiet backoff, and
provider rate-limit errors retain their retry deadlines. This changes standing
watch freshness to three minutes while active and up to ten minutes while quiet;
it does not change direct merge/check gates or claim measured live savings.

## Operator-supplied installation read source

The optional daemon `[github].app_token_root = "/absolute/operator/supplied/path"`
or explicit `ST3_GITHUB_APP_TOKEN_ROOT` environment input selects installation
tokens for reads. The environment input overrides the config field. Relative
roots are rejected. st3 never computes a platform root: Agent Policy's host facts
are Linux `$XDG_RUNTIME_DIR/github-app-token` and macOS
`$(getconf DARWIN_USER_TEMP_DIR)github-app-token`; the operator supplies the
already-resolved absolute result. The external minter rollout depends on
dotfiles#7401 and is not performed by this code change.

For a repository's owner, read `ROOT/gh-ci/OWNER.token` and `OWNER.json` for
metadata, branches/contents, PRs/reviews, checks/actions and open-PR GraphQL.
Use `ROOT/github-ledger/OWNER.token` and `OWNER.json` for issue lists, issue
comments/excerpts, issue watch preflight and Issue identity GraphQL. PR watch
preflight and review excerpts use gh-ci. Remembered opaque node IDs are partitioned
by their recorded `ItemIdentity.pull`, never guessed from their bytes, and each
partition gets its own minimal identity query and app authorization.

Token files must be owner-owned regular mode0600 files, containing only the
bare token with no whitespace or newline. Sidecars contain numeric
`installation_id`, RFC3339 `expires_at`, a `permissions` object and
`repository_selection` (`all` or `selected`). The minter owns atomic publication
and refresh when less than15minutes remain from a60-minute lifetime. st3 reopens
the supplied files per read batch, not once per daemon lifetime. A401 permits
one bounded reload/read replay, then a transition PAT read if still rejected.
No network retry is made for a failed transport or for a mutation.

The existing Local/Authorized PAT transport remains the explicit route for
every REST or GraphQL mutation; app availability never gates writes. Unconfigured
roots keep existing PAT behavior. Read-only transition fallback increments the
OTel COUNTER `st.github.auth.fallback`, labeled `github.owner`, `github.app` and
`github.auth.fallback.reason`, and emits a warning. Reasons are `missing`
(unconfigured/absent root or one missing file), `no-installation` (both owner
files absent under an existing supplied root), `expired`, `unreadable`
(including invalid sidecars/token format/permissions), and `401`. Operators must
use this explicit absence convention when publishing an owner's installation.
These count source-selection fallback events, not GitHub charges; conditional
cache hits can still encounter a missing source. Remove transition fallback only
after one week with zero unintentional fallback across the enrolled owners.

**Legacy status qualification:** neither app has statuses:read. An app GraphQL
`statusCheckRollup` returns check runs without legacy `StatusContext` data, so
repository PR detail reads retain a PAT-only combined commit-status GET, once
per distinct head SHA in a read, through the existing ETag cache. Its source
exception is observable as fallback reason `statuses`. Legacy status contexts
are merged with app check runs before deriving check facts and required-check
outcomes. Truncated status responses fail visibly rather than asserting missing
contexts passed. This intentional status exception must be excluded from the
one-week unintentional-fallback gate until installation permissions are separately
approved and qualified. No statuses permission is assumed or added here.

REST cache keys include the stable installation principal, permissions and
repository-selection scope, not token bytes; rotation preserves cache identity.
PAT and app bodies/ETags are separate. A401 PAT fallback drops the app's ETag
and does not populate the app cache with a PAT representation.

Local fixture tests cover supplied-root reads, endpoint selection, no-root PAT,
rotation, expiry, invalid/unreadable/missing sources and reason counts,401 reload
and fallback, separately authorized mixed identity pages, PAT-only writes and
legacy-status merging. They are written but not run in this slice; no build,
test, lint, formatter, credential inspection or live migration was performed.
