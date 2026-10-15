# Replication guard accounting

The recent-inbound guard compares only its local authority digest with the last inbound offer.
`Backend::authority_digest` returns only that digest, so its result cannot enter a peer exchange.
MainBackend sends a default-false `authority_only` body flag together with `authority_guard`
and `summary_only` to the existing local export handler. An old daemon ignores the new flag,
returns its full summary, and the new worker extracts the digest. An old worker omits the flag
and still gets the full summary from a new daemon. Ordinary request bytes and peer exports
remain unchanged; no compact flag enters a signed peer exchange.

The compact helper calls the same `replication_snapshot` as the full summary, retaining sealing,
judging, freshness and failures. It omits signature-request and advertised-checkpoint queries,
bucket/projection copies and their serialization. The comparison, push-on-error behavior,
notifications and scheduling are unchanged. No persistent cache or additional writer capacity
is introduced. Invalid or undecodable digest replies are errors, not a reason to suppress a push.

The existing `/v1/performance` meter records two fixed labels:

- `replication/authority-guard`: completed summary store calls marked by the guard, including errors.
- `replication/summary`: other completed summary store calls, including errors.

The second label includes every ordinary peer-summary export served by this handler. It is
a visible addition to the report, not a new route. The CLI renderer prints arbitrary kinds;
the cost tests measure their declared route workloads rather than rejecting unknown meter
kinds. stui has no consumer of the performance report's request-kind table.

Each label records count and elapsed wall time of the synchronous store call. These times
overlap the enclosing export request; never add them to the route's wall time. The recorder
does not measure CPU. Its `cpu_ms: 0` means no CPU was recorded, not zero CPU use. Wall time
across the worker's await is also not CPU. JSON serialization, client decode, hashing and
compression are outside these labels; their CPU shares remain unknown. Existing route and
client accounting continues unchanged, apart from the bounded recorder overhead.

The meter is a rolling, at-most-300-second top-20 report. A missing label is unknown, not zero;
use only a report that includes both labels and the export route for a call-share comparison.
Calls that fail before entering the summary handler or never complete are not in the label.
No new endpoint, profiler enablement, stored payload or sampling service is introduced.

One bounded real-node report had 289 guard calls among 1,064 exports. The guard's full store
path was 16.21% of export route wall in that one 293-second sampled report. This change removes
unnecessary full-summary assembly and clone work from those calls; the shared snapshot work
stays. The removable fraction and CPU saving are UNKNOWN. These figures are not a CPU share
or a prediction of the deployed effect.

The controls compare warmed full, digest and shared-snapshot paths on the same store, count
SQL and logical bucket/projection entries actually copied by summary assembly, and vary bucket
count. The clone counter is test-only and does not measure allocated bytes or dirty snapshot
rebuild work. Full/compact digest parity, local writes, actual payload trim, reopen, mixed-build
requests and real worker push liveness have separate controls. They are UNRUN in the private
source packet. Hosted results and a separate finite Operations read after normal rollout are
required before any effect claim. No four-member ten-minute idle interval is established.
