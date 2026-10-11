# CPU and additional condition adapters

This stacked follow-up adds the remaining adapters after the disk-first daemon
change. It inherits the same local quorum-free retention dependency, OTLP history
and copied notification outbox. It cannot queue until disk retention is complete,
independently approved and deployed. Deploy every daemon before declaring these
metrics; the disk-only parser refuses them.

| Scope | Metric | Reading |
| --- | --- | --- |
| host | `memory.available-percent` | available native memory percent |
| process | `process.cpu-cores` | CPU seconds per elapsed second, summed by executable name |
| process | `process.rss-bytes` | resident bytes, summed by executable name |
| member | `daemon.cpu-cores` | existing daemon CPU SLO windows |
| member | `cost.usd-per-day` | existing recorded model spend over 24 hours |
| route | `slo.p99-ms` | existing latency window p99 milliseconds |
| route | `slo.burn-rate` | fraction over the target divided by its one-percent error budget |

A process declaration supplies `process "collector"`. A route declaration
supplies `route "person-read"` or a measured path. CPU and route metrics accept
`window "1m"`, `"5m"` or `"1h"`; route defaults to one hour and daemon CPU to five
minutes. No new endpoint, collector, SLO window or performance budget is added.
Native process enumeration is bounded and can omit processes beyond its cap;
Executable names longer than the 15 bytes kept by `/proc/PID/comm` are
rejected. CPU requires two samples of the same PID/start time; children that
start and exit between samples are undercounted. The per-PID scan cost is
unmeasured in production. Process names removed from
declarations are pruned from the sampler. Native memory/process readings may be
unavailable on platforms without the supported kernel facts. Missing readings
interrupt a hold and preserve an established breach.

`db.authored-bytes-per-day` retains the previous informational attribution
reading: bytes authored by this member, not physical database growth. It is not
the first-user growth condition. First-user declarations must not set a condition
on this attribution metric. Physical net database-plus-WAL growth per replica
needs the shared hourly sampler and Operations' last-week baseline; it remains a
follow-up. `db.size-bytes` keeps the disk PR's hourly background cache.

Spend is reused for five minutes. The authored-byte counter catches up in bounded
indexed pages and prunes hourly buckets after two days; it can be unavailable
while catching up or with less than an hour of coverage. Precise readings and
hold phases remain local. Only entry/recovery transitions replicate, and every
locally authored transition is copied atomically for the existing OTLP exporter.

The finite Operations [production node protocol](conditions-measurement.md) measures the
whole daemon plus evaluator SQL. No local test proves zero production overhead.
The [retention contract](condition-retention-contract.md) separates retained
state from permanent negatives, messages, pending deliveries and daily transport
writes; the same unresolved payload/transport dependency applies to this PR.
