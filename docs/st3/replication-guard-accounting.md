# Replication guard accounting

Current workers compare the daemon's store-scoped authority revision snapshot with
the authenticated inbound authority witness. They do not fetch an authority guard
on every wake. Membership has an independent subscription.

The daemon still accepts the `authority_guard` body marker from older workers.
It classifies only that local request, is ignored by old daemons, and never enters
a signed peer exchange. This request-decoding compatibility does not restore the
old worker polling path.

The existing `/v1/performance` meter records these fixed labels:

- `replication/authority-guard`: completed legacy guard store calls, including errors.
- `replication/summary`: other completed summary store calls, including errors.
- `replication/root-summary`: root-only summary constructions.
- `replication/full-summary`: full bucket summary constructions, including difference exports.

The second label includes every ordinary peer-summary export served by this handler. It is
a visible addition to the report, not a new route. The CLI renderer prints arbitrary kinds;
the cost tests measure their declared route workloads rather than rejecting unknown meter
kinds. Fractal TUI has no consumer of the performance report's request-kind table.

Each label records count and elapsed wall time of the synchronous store call. These times
overlap the enclosing export request; never add them to the route's wall time. The recorder
does not measure CPU. Its `cpu_ms: 0` means no CPU was recorded, not zero CPU use. Wall time
across the worker's await is also not CPU. JSON serialization, client decode, hashing and
compression are outside these labels; their CPU shares remain unknown. Existing route and
client accounting continues unchanged, apart from the bounded recorder overhead.

The meter is a rolling, at-most-300-second top-20 report. A missing label is unknown,
not zero. Deterministic counting tests establish the zero-work contract; a missing
row alone does not.
Calls that fail before entering the summary handler or never complete are not in the label.
These labels require no profiler enablement, stored payload or sampling service.

Aggregate export counters alone do not establish how much full-summary work was
avoided. Compare the summary-construction labels and the enclosing export routes
over the same window. Work-count reductions do not by themselves establish a
measured CPU saving or an idle CPU floor.
