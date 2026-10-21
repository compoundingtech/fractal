# Replication guard accounting

The recent-inbound guard asks its local daemon for a full replication summary and compares
only its authority digest. Its export, digest comparison, error handling and scheduling are
unchanged. The `authority_guard` body marker classifies only that local request. It is omitted
from ordinary requests, ignored by old daemons, and never enters a signed peer exchange.

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

Existing aggregate export counters do not distinguish guard calls. Private incident
measurements are retained outside the repository. Compact-mode implementation waits for a
bounded read on a reachable real node after the
normal Operations rollout. This baseline alone establishes neither a CPU saving nor an idle
floor, and does not qualify the four-member, ten-minute idle goal.
