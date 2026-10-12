# Git change reads

`st changes list`, `show` and explicit `sync` use configured Git transport and the
node-local graph index. They use no GitHub API. The existing PR workflow remains
available when Git or its designated reader is unavailable.

The feature is off by default. An operator supplies one controlled transport
repository and one reader node per repository/fleet. Requests accept the configured
name only: they cannot supply a checkout, destination or credential command. A
configured destination must match the transport's Git remote before network I/O.
The reader uses its service OS account. Diagnostics show its actual account, reader
node and safe credential-source label; command bodies and tokens are not shown.
This does not establish isolation between seats sharing that account.

```toml
[changes]
enabled = true

[[changes.repositories]]
name = "fixture/garden"
reader_node = "fixture-reader"
transport_directory = "/var/lib/fixture/git/garden.git"
remote = "origin"
remote_url = "https://github.com/fixture/garden.git"
credential_helper = "gh auth git-credential"
credential_source_name = "operator/git-reader"
pilot_started_at = "2026-01-01T00:00:00Z"
```

The example is illustrative, not a service activation. The configured clone and
credentials are operator-owned and outside an untrusted workspace. Authentication
failure retains the old observation and reports its age; it never becomes an empty
successful inventory. A configuration owned by a different node reports the named
owner and the PR fallback instead of starting a second Git reader.

```text
st changes list --repository fixture/garden
st changes show I0123456789abcdef0123456789abcdef01234567 --repository fixture/garden
st changes sync --repository fixture/garden
st changes list --repository fixture/garden --offline
```

A new shared observation performs one patterned `ls-remote`. Concurrent requests
join it; the minimum interval is ten seconds and explicit fetch shares the limit of
six remote reads a minute. `sync` reports in-flight or deferred admission rather
than bypassing the budget. After a completed fetch, a finite importer validates
otherwise unknown retained metadata chains, ancestor first, with a 10,000-record
job limit. It holds no database snapshot or writer during Git operations. A canceled client
does not release the import slot while blocking validation is still running. A larger
history requires a later resumable importer; the PR path stays available.

List pages contain at most 50 changes. Their cursor pins the original observation
for five minutes, is scoped to repository and offline mode, and expires on restart
or bounded eviction. Source/index/evidence freshness remain distinct. An unknown
current object says `sync_needed`; a missing previously indexed change says
`remote_missing`; inconsistent markers say `marker_conflict`. Indexed metadata is
cooperative scheduling data and does not prove a real landing or a live work lease.
Evidence freshness is unknown until separately indexed and verified.

List/show use the existing `read.projections` scope and client envelope. Explicit
sync requires `control.work` because it exercises configured Git credentials and
writes the local derived index. The CLI carries the configured person or the
current `ST_AGENT` over the trusted local Unix socket for sync; list/show keep
their read-only client. Sync accepts no actor selector and refuses an HTTP
endpoint instead of forwarding local identity. An unavailable or non-owner sync is a typed
`unavailable` result naming its reader node and the PR fallback. Imports bind each
current ref's canonical ID to the ID of every loaded or already indexed record;
a mismatch is skipped and reported in a bounded issue list, and cannot set another
change's latest history pointer. A canceled client between fetch completion and
import start can leave fetched objects waiting until another explicit sync.

This importer has a finite 10,000-object job bound, with full unknown lineage
validation before publication. It has no minute-scale wall budget or durable
checkpoint for an unfinished lineage. Very deep cold histories require a separate
resumable importer or the PR path; repeatedly running sync is not promised to
complete a lineage beyond the bound. This read
slice does not add claim/state/source mutation commands, a landing observer,
production pruning, a new claim kind or a binding-contract field. Pilot creation
and stage actions remain off until their separately reviewed authority and bound
checks are available.

## Explicit stage publication

The stage adapter is disabled by default. An operator can separately set
`mutations_enabled: true` for the named owner repository and its explicit pilot
start. It permits draft creation, draft/ready/next marker changes, and cooperative
build/review claims. Source commits must already contain exactly one canonical
Change-Id; absence requires explicit recovery in the owned workspace. Creation
never amends a commit or invents an association. It validates at most 100 selected
commits and rejects incompatible IDs in that range. A previously indexed identity
whose remote refs disappeared requires explicit owner repair, never fresh creation.

```text
st changes create --repository fixture/garden --step STEP --source FULL_SOURCE --base FULL_BASE
st changes ready ID --repository fixture/garden --step STEP --expected-current RECORD --next build
st changes claim ID --repository fixture/garden --step STEP --expected-current RECORD --lane build
st changes release ID --repository fixture/garden --step STEP --expected-current CLAIMED_RECORD --nonce NONCE --next review
st changes draft ID --repository fixture/garden --step STEP --expected-current RECORD
```

An agent seat supplies its exact incarnation through `ST3_INCARNATION` (or the
explicit `--incarnation` flag). Before publication, a bounded owner-runtime read
obtains the exact durable work-claim reference; `--work-claim` can supply it
explicitly. The API derives the actor from the authenticated local Unix session,
requires `control.work`, and checks the current generation, attempt, worker,
incarnation, original local claim, ready runtime and actual locally renewed lease.
It repeats the proof immediately before push and requires at least 30 seconds of
lease remaining. A Git timestamp or a replica's remembered lease is insufficient.
Another work claim or runtime incarnation invalidates the fence.

One designated owner serializes publication. Canceling a requester leaves the
owner operation running and holding its publication slot; another requester gets
an explicit in-flight result. Every publication capacity check shares discovery's
six-read/minute and ten-second admission budget. It reads change and immutable
evidence refs together, then preserves a normal change-only observation for reads.
Budget refusal reports an explicit retry delay or PR fallback, with no hidden wait
or polling. An increase must fit 5,000 managed refs. Creation counts both remotely
visible and previously indexed distinct IDs toward 100, and the pilot lasts at
most 14 days. Voluntary release remains available after that window.

Claims always advance current and create the lane claim ref in one atomic leased
push. Release checks the worker, originating work binding and nonce, advances
current and removes the old claim. No claim is inferred from an uncertain push
result; inspect the remote record and binding before retrying. Runtime cancellation
can race a network operation, so a newly published scheduling claim must still
pass live authority checks for every subsequent transition. It grants no merge
permission.

This adapter refuses landing lanes and dependency-bearing claims: independent
landing/composition receipts remain with the existing PR and curator path. It does
not steal an expired claim, prune records, recover deleted refs, or amend sources.
Those actions require their separate verified adapters and reviews. A failure can
always continue through the existing PR path.

State and next-lane publication is restricted to the actor in the current metadata
record. Generation 1 assigns that actor to the authenticated creator; state edits
preserve it. An accepted claim transfers scheduling ownership to its claimant,
and voluntary release keeps that actor. Other live agents cannot flip the state
or next marker using an unrelated work lease. Any live local agent may claim a
ready change in build/review, independently of its step's role in this initial
adapter; release remains restricted to its exact claimant/work/nonce. Lane-to-step
role policy is a later adapter.

Failures have typed outcomes. Refusal uses `git_change_refused` (400), busy uses
`git_change_busy` (409, admission time unknown), and deferred uses
`git_change_deferred` (429 with retry_after_ms). An unconfirmed attempted push
returns change_id, candidate record_oid and the generated claim_nonce, with no
ownership assumption. Confirmed publication whose local index write failed uses
`git_published_not_indexed` and carries the same reconciliation coordinates. The
CLI preserves these details, including in JSON output, while short client messages
exclude operator paths and Git stderr; diagnostic details stay in daemon logs.

A malformed, non-self-pointing evidence ref makes capacity admission fail closed;
a shared repository writer can therefore stop scheduling in the pilot. Each
mutation consumes a capacity read from the same six-per-minute budget as list/show
and sync, so aggregate pilot throughput is at most six admitted remote operations
a minute. Git metadata and the shared credential do not authorize a main update.
