# Shared Git discovery and local record index

This slice provides the daemon reader's components. It adds no configured reader,
CLI command, route, polling loop, stage authority, or pilot activation. The daemon
adapter must install one discovery owner per configured repository/fleet and name
its service account and operator-managed credentials.

The owner performs one patterned discovery for a new observation. Concurrent
requests join it, and cancelling a client leaves the owner task alive. Later
requests reuse its original timestamp during a minimum ten-second interval.
Discovery and explicit fetch share a sliding limit of six reads a minute. Offline
reads perform no transport operation. A failed observation retains the prior
successful inventory and reports the failed attempt separately; it cannot become
an empty successful inventory. A denied fetch reports its next admission delay.
There is no per-seat reader or timer loop. Successful inventories and their ID
reduction are validated once and shared through immutable Arc allocations; joined,
reused, offline and paginated reads do not copy or rebuild the inventory.

The node-local graph database holds immutable metadata-object receipts and a
latest indexed pointer per repository and change ID. Tables are empty at creation;
there is no first-open history scan, new claim kind, replicated binding field, or
replicated digest contribution. An explicit importer validates Git objects,
source/tree/trailer/parents outside the writer before publishing a receipt in one
short transaction. Reindexing an existing object preserves its original indexed
time. Conflicting bodies for the same object fail. A late old generation does not
regress the latest pointer. Record bodies are bounded at 16 KiB. Growth is one immutable row per metadata
object; pruning is a later slice and is not enabled here.

List/show join the exact advertised current object with that local index. An
unknown object reports `sync_needed`, with old indexed data explicitly labelled as
history. Absent previously indexed IDs report `remote_missing`; absence is never
interpreted as a landing or abandonment. An inconsistent marker set reports
`marker_conflict`. A never-observed ID absent from a successful inventory reports
`not_found`. Offline data and observation errors retain their actual timestamps.
Consumers must read the page-level observation before displaying an item: a failed
refresh can leave an exact indexed receipt alongside an aged inventory and error.
Even `indexed` means cooperative Git scheduling data, not an independent landing
receipt or runtime-lease proof. Evidence freshness stays unknown because evidence
refs are outside the discovery pattern.

Pages contain at most 50 changes and use full-ID seeks across live and previously
indexed IDs. Metadata lookups are bounded primary-key reads; history and unrelated
repositories are not scanned. Responses include inventory count and freshness,
not the full ref map on every page. A future route must pin the observation in its
cursor and enforce the existing client authorization/response budgets. A future
mutation adapter must validate authoritative runtime leases, dependencies and
signed evidence, and implement the approved pilot limits and PR fallback.

Tests cover cancellation/concurrent joining, shared discovery/fetch limits, zero
offline reads, failed-observation retention, exact-object joins, missing/conflicting
markers, pagination, immutable index conflicts, and a fixed lookup with 100,000
unrelated indexed records. The cost check measures real store SQL work; it is not
a proof of whole-route latency or production writer-hold time.
