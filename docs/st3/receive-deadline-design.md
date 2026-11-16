# Live receive: whole-route deadline and durable continuation

Status: contract accepted by FPL; not implemented. Limits await instrumentation data.
Prerequisite: [the live-receive replay fence](receive-replay-design.md).

## Gap

A ten-second cooperative admission budget checked only after the first commit is not a
whole-route deadline. It leaves ingest, sealing/judging, mutex and writer waits, payload and
membership preparation, repairs, projection, tail bookkeeping and response work outside its
coverage. An expired admission budget followed by single-chunk projection still does not bound
the route when that chunk can enter full replay. This note addresses that reviewed budget
contract, not a claim that all source revisions already implement it.

## Proposed contract

Create one monotonic deadline at route entry, before body collection/decoding and blocking-work
queueing; propagate its remaining allowance through every phase. Never reset it after a commit,
lock acquisition or helper call. Reserve time for bounded completion bookkeeping and response
serialization/write. The target is ten seconds total, including that reserve; choose the reserve
and per-phase row/byte/work limits from instrumentation data, not assumptions. A dropped caller is not cancellation of already committed work.

| Phase | Required bound or continuation point |
| --- | --- |
| Body, payload, membership and signature preparation | Bound bytes, records, nesting and per-record work before allocation/folding; page membership work. Reject an oversized indivisible unit explicitly, without truncating it. |
| Blocking queue, projection mutex, writer queue and SQLite waits | Deadline-aware admission and waits; a queued expired job must not start expensive work. Recheck at acquisition. |
| Seal/judge, ingest and admission | Bound each transactional unit by both records and bytes; check before the first unit and between units. Commit identity and resume position together. |
| Repairs and projection | Same remaining deadline and bounded work units; no full replay. Persist independent progress when admission is ahead of projection. |
| Tail, notifications and response | Bounded summaries and response size; do not scan all committed history to construct the receipt. Bound server-side response delivery and account for its overhead. |

Checks between transactions are insufficient if one transaction, SQL statement, signature unit,
set dependency fold or commit can exceed the allowance. Before starting such a unit, establish
an enforced size/work bound and available time, or defer it without starting. Use cancellable
waits and supported query interruption only where rollback and connection reuse are safe.
Do not abort a committing transaction and then report it as uncommitted. Strict wall-time proof
must state the storage/OS scheduling assumptions and maximum non-interruptible overshoot;
cooperative checks alone cannot guarantee a hard real-time deadline on arbitrary storage.

## Durable continuation

Prefer existing durable envelopes, admission receipts and projection frontier as resume authority;
add only state they cannot express. Keep received, admitted and projected positions separate.
A continuation identifies the stable input/envelope identity, bounded next position, target
frontier and pending phase; it is not an in-memory iterator or a caller-owned retry token alone.
Persist each progress marker in the transaction that commits its corresponding work.

On expiry, roll back only the uncommitted unit. Already committed delivery remains committed;
return a bounded outcome distinguishing complete, pending and recovery-required. Never force a
full replay, reprocess a completed prefix, discard remaining envelope bytes, or roll back a
previous commit to manufacture an all-or-nothing timeout. Persist or retain the full verified
input before acknowledging its durable receipt; otherwise acknowledge only the durable prefix.
The wire outcome must not let a peer mistake a partial receipt for complete delivery.

Restart resumes from committed positions. Idempotent peer retries and overlapping receives
must not duplicate admission or regress the frontier. Use the existing projection serialization
and transactional fences to arbitrate the next unit, not a second scheduler. The existing daemon projection catch-up
worker owns continuation and releases the writer between units; replay-required work follows the explicit
recovery policy in the first note, not ordinary retries. Backlog and continuation failure remain
visible even when no further exchange arrives.

## Approval and proof gate

The ten-second total target and existing catch-up worker ownership are approved. Propose the
response reserve, per-phase limits and exact receipt/error wire representation after instrumentation.
Reject oversized indivisible units explicitly with a typed recovery-required outcome and the
operator path, never a silent retry loop; existing envelope limits remain unchanged.
If that representation changes persisted or wire contracts, review its compatibility separately;
this proposal does not authorize a new schema or alter accepted envelope limits silently.

Acceptance must cover expiry before the first commit, each wait/prelude/repair/projection/tail
phase, caller disconnect, response failure after commit, crash between units, restart without a
new receive, duplicate retries and concurrent receives. Check durable receipts/frontiers and
final graph equivalence, not only handler latency. Measure queue, execution and response time
separately; demonstrate stated overshoot and work bounds with large inputs and protected history.
No verification was run for this design.
