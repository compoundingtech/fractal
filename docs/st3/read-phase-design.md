# Status reads: phase attribution before bounds

Status: phase attribution only approved by FPL; not implemented.
Final note in the series beginning with [the receive replay fence](receive-replay-design.md).

## Evidence boundary

The supplied status observations include a native 15-second read deadline and a reported outer
four-second cutoff. The outer cutoff proves only that its caller did not observe completion in that window;
it does not establish the daemon's internal phase or prove work stopped at cancellation.
The exact blocker remains unknown: fleet-cache mutex/fold work and snapshot construction/query
work are candidates, not established causes. Prior agent-card cache changes described in
[Profiling](profiling.md) do not prove the behavior of this separate status path.

Do not infer writer contention from concurrent writer stalls, or a fixed read-pool bottleneck
from historical pool behavior. Do not increase the timeout and present that as a fix.

## Proposed observation contract

Reuse the existing route operation, writer/read/SQLite accounting and named phase spans.
Attribute these boundaries on the actual status path, without adding synchronous diagnostic
store reads or status snapshots to the measured operation:

- Route admission, blocking queue and handler start.
- Fleet-cache lookup, mutex wait, cache hit/miss, fold/build, and publication.
- Read connection acquisition/open and snapshot establishment.
- Snapshot construction and its constituent query/fold work.
- Response assembly, serialization and server-side delivery.

Separate wait from work and include cache hit/miss and bounded item/byte counts. Identify the
phase still active when cancellation occurs and account for blocking work that continues after
it. A dropped profile is not evidence of successful response delivery. Reuse existing low-cost,
opt-in profiling; do not create a second tracing system or emit payloads/high-cardinality labels.
Confirm which boundaries already exist before adding spans. Nested/overlapping phase times
must not be summed as independent route work.

## Conditional bounds, not a speculative rewrite

Only after phase evidence identifies a contributor should implementation propose its bound:

| Evidence | Candidate change requiring review |
| --- | --- |
| Cache mutex held across an expensive fold/build | Build outside the mutex, publish with a generation/snapshot fence, preserve invalidation and prevent unbounded duplicate builds. |
| Repeated history fold or snapshot query dominates | Use an indexed bounded read or existing immutable snapshot/cache, preserving snapshot consistency and freshness semantics. |
| Queue or delivery dominates | Bound that wait/work at its owning layer, without labeling it a database projection fix. |

A full-status result cannot silently become a truncated or stale result to meet a deadline.
If the identified phase cannot be bounded under the existing result contract, FPL must choose
an explicit pending/unavailable outcome, a freshness contract, or a paged result before code.
Cancellation must not leave an unbounded detached build or a held snapshot/mutex behind.
This note selects no new cache, paging API, concurrency limit or timeout value.

## Approval and proof gate

FPL approved phase attribution only. A behavioral bound requires a follow-on
review grounded in complete native-request profiles; the outer cutoff alone is insufficient.

Proof should compare cold and warm reads, concurrent requests, cache invalidation, long history,
cancellation and response failure. Attribute native and outer deadlines separately, preserve
returned status/snapshot semantics, and measure remaining work after caller cancellation.
No live investigation, instrumentation or verification was performed for this note.
