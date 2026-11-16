# Observation writer work: append and retention

Status: instrumentation-first direction accepted by FPL; limits await data. Not implemented.
This is the third note after [receive continuation](receive-deadline-design.md), not a
prerequisite to claim a measured cause for an individual slow write.

## Evidence boundary

Supplied journal writer-hold diagnostics attribute a roughly 17.4-second hold to one harness-events write;
a separate roughly 11.9-second trim hold reaches the per-subject/kind cap branch. These identify
operations, not an exact internal cause. `append_local_observation_fenced` includes transactional
runtime fencing and observation insertion. Serialization, digesting, evidence and provenance
processing can grow with input; no supplied measurement apportions their contribution.
`trim_local_observations` caps deleted rows per transaction but its cap-boundary lookup uses
`OFFSET`. A 500-row delete limit does not bound rows scanned, protected rows examined, bytes
processed, or total pass work. Do not turn either observation into a claim that SQLite, payload
size or a particular helper is the proven bottleneck.

## Harness-events append proposal

First attribute preparation, writer queue, runtime/idempotency checks, observation insertion,
evidence/provenance work, digest/serialization work and commit within existing profiling.
Record bounded counts/byte sizes, not event bodies. Preserve writer hold versus wait accounting
and distinguish caller completion from a blocking write that outlives cancellation.

Move pure preparation of immutable input off the writer only where attribution and code contracts
support it. Prepare the canonical bytes/digests once and consume that representation in the
transaction; do not introduce a second serializer or digest definition. Revalidate incarnation,
runtime fence, duplicate-event identity and any store-derived provenance inside the writer.
No reader-side result can authorize an append after its fence changes.

For input-dependent work, choose explicit accepted event/evidence/provenance byte and count
limits before promising a bounded append. Refuse oversized atomic input with a structured,
truthful outcome before writing; never truncate evidence or acknowledge an event that was lost.
Splitting one event requires proof that its existing atomicity, ordering, idempotency and native
acknowledgement semantics survive; this note does not authorize that protocol change.

## observations.trim proposal

Use bounded off-writer candidate discovery and small in-writer deletion units. Carry keyset
cursors for age order and subject/kind cap discovery; avoid repeated deep OFFSET scans and
whole-table grouping collected into memory. Bound rows examined, candidate bytes and elapsed
work per reader page and writer unit, as well as deletes and total pass work. A pass yields with
a continuation even when it finds many protected candidates and deletes nothing.

A candidate is a hint, never deletion authority. In the writer, revalidate the exact candidate
identity, current retention eligibility, newest-row preservation, and every applicable protected
consumer/fence before deleting. Concurrent append or cursor movement can change eligibility.
For caps, use keyset pages with bounded boundary preparation and in-writer revalidation;
no OFFSET. Do not move unbounded rank work to a reader and call the whole pass bounded.
Reuse an existing durable cursor only if existing retention state can express the progress;
otherwise use bounded restartable pages. New persisted boundary/protection state needs review.

Resume discovery from a stable high-water mark so continuous appends cannot prolong one pass
forever. Revisit skipped candidates in subsequent passes; a cursor must not permanently strand
rows that become eligible later. Restart may safely repeat a bounded page, but must not lose
protected/newest observations. Preserve existing age, per-subject/kind cap and protected-consumer
rules exactly. This is a work-placement change, not a retention-policy change.

## Approval and proof gate

Instrumentation before causal optimization is approved; derive event size/work and trim
page/pass limits after the data. Retention policy is unchanged; any new persisted protection
or boundary state requires separate review. Cheap observation and status instrumentation may
proceed in parallel after the receive instrumentation/capture priority.

Acceptance must preserve byte-identical canonical identity, runtime fencing, duplicate handling,
acknowledgements and evidence under concurrent incarnation changes. Retention proof must cover
newest/protected rows, concurrent append and consumer advancement, cap changes, long histories,
no-deletion pages, restart and eventual reclamation. Measure rows scanned and bytes as well as
deletes and writer holds. No implementation or verification accompanies this proposal.
