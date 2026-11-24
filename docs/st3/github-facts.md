# Durable GitHub facts: proposed plugin contract

Status: design proposal; no new claim kind, capture mode or endpoint is enabled by this document.
Implementation and activation require the acceptance gates below. This deliberately extends
[the plugin store contract](plugins.md): bounded GitHub history becomes durable claim evidence,
while full source content and rebuildable indexes remain outside the replicated fact authority.

## Ownership and scope

Fractal owns acquisition, normalized identities, lifecycle evidence, coverage, corrections,
replay, watch delivery and native objective evaluation. Its existing GitHub HTTP/auth/admission
path is shared by operational reads and historical capture; no second credential subsystem,
standing repository poller or authoritative plugin database is introduced.

Deployment configuration owns credentials, admitted source scopes, private-data replication,
the designated capture member and resource budgets. Policy declarations own versioned,
effective-dated repository membership, target values, timezone and response owners. None of
those deployment-specific defaults belongs in the public GitHub plugin.

The first fact families are repositories, PRs, ready/draft transitions and merges. Workflow
attempts, checks, jobs, review/GO evidence and artifact completeness need separate contracts;
a successful PR crawl cannot certify those populations. Existing GitHub writes and post
attribution remain supported, including command-time authoritative precondition reads.

## Stable identity and bounded records

Canonical identities are `(GitHub host, repository node ID)`, `(host, PR node ID)` and
`(host, upstream timeline event ID)`. Owner/name/number are mutable routing aliases.
A rename or transfer does not create a new PR or merge. Observed time is separate from
upstream occurred time; a mirror difference is not a precise upstream transition timestamp.

Proposed kinds, subject families and field ceilings require registry/manifest review:

| Kind | Subject identity | Fields ceiling |
| --- | --- | ---: |
| `github.fact.observed` | host, source generation, entity/event ID, revision digest | 1 KiB |
| `github.capture.committed` | logical source, generation, authority epoch, source sequence | 2 KiB |

One fact carries schema version, source/generation, stable identity, normalized image or
retraction, digest, observed time, upstream revision/time where known, and explicit supersession.
PR images contain the lifecycle fields needed by current state and measurement: repository/PR
IDs, number, state, draft/created-as-draft evidence, created/closed/merged times and author identity.
Each ready/draft event is its own fact; never append an ever-growing timeline array to a PR.
Merge identity is the PR identity, not the capture or observation ID. A corrected merge revises
that entity and does not create another merge. Null/unknown creation-draft evidence stays unknown.

Bodies, titles, review prose, labels, head history, request metrics, logs and artifact bytes are
not part of these first normalized families. Large optional content needs separate retention
and must not become an external-only dependency of retained decision evidence. Reject an
oversized required identity or record explicitly and leave coverage incomplete; never truncate
identity, silently skip evidence or raise the generic plugin byte ceiling to fit raw snapshots.

## Capture is the visibility boundary

A bounded capture commit contains source/generation, admitted authority epoch, monotonic source
sequence, predecessor commit, referenced fact IDs/digests, a scan checkpoint, audit totals and
coverage deltas. The sequence belongs to the source chain, not a member's store index or clock.

1. Stage bounded normalized fact claims using ordinary idempotent claim appends.
2. Append a bounded commit marker referring to those exact facts and the prior committed frontier.
3. Publish facts, cursor and coverage together in projections only when the marker, its predecessor
   and every dependency are present, valid and admitted for that source/epoch.

This is a logical commit protocol, not a claim that the current append API supports atomic
multi-append. A crash before the marker leaves inert staging. A crash after it is replay-safe.
Replica reordering cannot expose coverage before evidence. Missing or invalid dependencies mean
projection-not-ready, never successful capture or an empty complete answer.

Split pages by encoded byte size, not just row count. A planning budget of four fact references
per marker is conservative; coverage and checkpoint bytes also count against the 2 KiB ceiling.
Split a large coverage delta or timeline across markers. Keep the relevant pass incomplete until
all its dependencies are committed; page completion cannot stand in for population completion.
Use existing append/idempotency machinery, not a new generic transaction service.

The idempotency key binds source, generation, epoch, predecessor and capture digest. Re-fetching
uncommitted work is permitted. Duplicate poll, webhook and historical conversion observations fold
by upstream entity/event identity and normalized revision. Changed prose alone writes no new
normalized fact. Source access or schema/population changes are explicit generation changes,
not accidental cursor resets after a member move.

## Single acquisition authority and fencing

One explicitly designated member captures each logical source, sharing credential budget and
live/backfill scheduling for watched and scoped repositories. Other members read projections or
relay reads; a slow or missing authority does not start a replacement crawler. `gh watch` adds a
subscription/acquisition request to this source rather than a second standing poll loop.

Authority admission binds source, member and monotonically increasing epoch. Selected desired
state or eventual last-writer selection is not a distributed mutex. Transfer requires an explicit
stopped/fenced-writer receipt, its final committed frontier and an admitted successor epoch.
No automatic failover or promotion during partition is part of this contract.

The successor continues the source chain without changing source identity or generation. Old-epoch
claims remain valid only within the predecessor epoch's admitted final frontier. Late replication
beyond that cutoff is rejected, not selected by arrival order. Projection validates the transfer
chain before exposing successor coverage. A partition may cause unavailability rather than
split-brain acquisition. A grant without proof of old-writer fencing is insufficient.

## Coverage and corrections

Coverage is derived from committed capture evidence, never a separately writable truth. Expose
source/generation, scope revision, repository/family, requested history boundary, complete and
authoritative flags, observed-through, last trustworthy success and explicit gaps/errors.
Repository discovery completeness is distinct from PR enumeration and timeline completeness.
An inaccessible roster entry stays in the population as unknown. Search-derived external-own
results cannot assert authoritative exhaustive history.

Moving listings require restartable pagination, overlap and bounded full reconciliation. A
rate/budget stop commits progress but does not advance a complete watermark past unscanned work.
Reuse existing authentication/refusal classification and retry deadlines; permission refusal is
not a rate-limit pause. Prioritize live activity without starving historical repair.

A correction or retraction appends a revision with provenance and supersession. A 404 does not
prove historical deletion. Canonical folds select the latest valid committed revision in source
order, not the first observation or replica arrival. Recompute affected measurements/series while
pinning past evaluation evidence to the frontier actually used. Never fabricate precise ready
history from legacy mirror differences.

## Retention, capacity and projections

Normalized lifecycle identities/revisions, corrections, manifests, capture chain and coverage/audit
history are durable in claims and normal backups. The mirror's 30-day transition retention does
not apply. Local projections/content caches are disposable; deleting them rebuilds from committed
claims, not a new GitHub scan that silently loses unrepeatable observed history.

Initial useful history is PRs merged in the previous 30 days plus all open PRs and each selected
PR's complete ready/draft history, including events before that window. Filtering by mergedAt
alone cannot prove final-ready history or older open age. A full repository scan remains necessary
where the provider cannot otherwise prove the requested population. Fourteen daily bars require
coverage for their complete calendar boundaries, not merely fourteen preceding 24-hour intervals.

Admission requires a measured incremental backfill/capacity gate. Count normalized revisions,
event records, markers and coverage, not raw prose bytes; include claim/envelope/index overhead,
replication and backup growth, checkpoint cadence and projection rebuild cost. A preexisting
store-size objective miss is separate from whether this incremental workload is material.

A representative retained-history measurement found 91,529 normalized revisions, 22,700,229 payload
bytes and a 33,771,520-byte disposable projection. A conservative planning allowance of 4 KiB per
fact plus 4 KiB per four-reference marker yielded about 502 MB including that projection; this
is an estimate, not measured production write amplification or a writer-throughput benchmark.
The much smaller initial history window is not permission to discard selected PRs' earlier cycles.

Write changed normalized images only. Coalesce no-change coverage checkpoints at an explicit,
budgeted source cadence and report the resulting freshness; do not append a durable record per
request or no-change poll. The source declaration must bound this ongoing cost before admission.
Do not reclaim authoritative facts merely because a disposable projection no longer needs them.

Candidate rebuildable indexes use these bounded keyset reads:

| Read | Projection key / demonstrated SQLite plan | Bound |
| --- | --- | --- |
| Merge window / series page | covering `(repository_id, merged_ms, pr_id, ready_ms)`; `SEARCH ... repository_id=? AND merged_ms>? AND merged_ms<?` | 1,001 rows, resume or explicit incomplete |
| Last ready/draft cycle at merge | primary key `(repository_id, pr_id, occurred_ms, event_id)`; `SEARCH ... repository_id=? AND pr_id=? AND occurred_ms<?` | reverse order, `LIMIT 1` |
| Replay page | source sequence primary key; `SEARCH ... rowid>?` | 128 scanned records |
| Coverage at frontier | `(repository_id, kind)` inside pinned source/frontier projection | one row per requested family/member |

These are candidate projection query plans, not an assertion that the current daemon implements
them. The last lifecycle event at merge must prove ready; a last draft event is not bypassed by
searching backwards for any earlier ready event. A created-ready fallback requires complete
history and explicit non-draft creation evidence. Unknown coverage never becomes zero latency.
Pagination caps are work bounds, not permission to report an incomplete aggregate as complete.

## Replay, native objectives and watch behavior

A versioned authenticated plugin fact feed exposes one committed frontier with snapshot-consistent
coverage. Cursors bind source/generation, kind, scope revision and canonical source sequence, not
machine-local store index. Scan a bounded number of observations, advance across empty filtered
pages, and reject mismatched/future/invalid cursors explicitly. Do not silently restart replay.

Preserve existing plugin read/CLI shapes, post attribution, watch deadlines, own-post filtering,
delivery deduplication, closure and mission evidence. Historical capture cannot wake subscribers
for old activity or reset their live delivery baseline. Keep a distinct eligible-live delivery
frontier and durable receipts. Narrow command-time precondition reads remain allowed, but do not
turn them into a second durable scheduled source writer.

Policy uses the native TypeScript declaration DSL lowered to KDL and Fractal's evaluator, never
a second consumer-side evaluator. Rolling throughput counts distinct merged PRs in `[cut-1h, cut)`
across effective-dated repository membership and all authors. Calendar bars carry timezone,
boundaries, partial-today flag and policy revision. Ready-to-merged latency starts at the final
covered ready cycle and ends at upstream mergedAt; head pushes do not reset it.

Evaluations/series expose definition revision, evaluation cut, source frontier, observed-through,
lag, known/unknown population, units, target status, verdict and evidence. Lagging wall-clock-now
answers are partial; an explicit as-of-watermark answer must not be relabeled now. Consumers
render these results instead of independently counting events or calculating another ready clock.

The first replay extension stays outside client-v0 schema/codegen. A new typed relay variant that
would touch a frozen client contract remains held pending explicit owner ruling; do not infer
that a native SLO exemption permits unrelated client, thread or composer changes.

## Activation and acceptance gates

- Obtain plugin-contract ownership acceptance and the applicable independent design review.
- Deploy reader capability to all admitted members before enabling unfamiliar durable kinds;
  explicitly handle absent members rather than allowing mixed readers to silently ignore facts.
- Author effective-dated policy membership without invented historical dates. Reconcile retained
  ledger/source revisions and every live consumer before converting history or retiring writers.
- Admit private-data replication and the designated authority through the normal approval path.
  Stop/fence the old writer, preserve its final frontier and retained recovery authority, and
  convert observations with original source sequence, times, corrections and coverage provenance.
- Activate both throughput and true ready-to-merged measurements with coverage before switching
  their consumers. Retire superseded merge writers and proxy/completed-hour policies at cutover.
- Complete remaining fact-consumer parity before deleting the old ledger package/service; a live
  reader cannot be abandoned or left on a second authoritative historical writer.

Required integration evidence: crash before/after marker; missing/reordered replica dependencies;
duplicate poll/webhook/conversion; corrected/retracted merge; stale epoch and stopped-writer
transfer; moving-list backfill and budget stops; multi-page ready histories; unavailable member;
rename/transfer identity; multiple ready/draft cycles and created-ready cases; exact rolling and
DST calendar boundaries; stale-now versus as-of-watermark; watch replay/takeover/own-post behavior;
and recovery solely from backed-up claims after deleting all plugin projections.

Before activation, rollback removes inactive code/declarations. After capture begins, freeze the
new writer and recover/replay its claim chain with a capable reader/writer release, or explicitly
stop capture. Never restart the old ledger from a stale cursor as a second authority. Historical
read arms/conversion preserve original evidence; they do not authorize parallel live writers.
