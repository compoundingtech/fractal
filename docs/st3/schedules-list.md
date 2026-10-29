# Listing schedule declarations

`st schedules ls` (also `st schedules ls`) reads one bounded page of schedule declarations
already applied on the selected local member. It includes declarations from other hosts that
this member has received through replication. It never contacts those hosts to answer the list.

```sh
st schedules ls --as person/operator
st schedules ls --limit 20 --json --as person/operator
st schedules ls --cursor 'TOKEN' --as person/operator
st schedules ls --unheard-after 10m --as person/operator
```

The CLI uses `--as`, the seat's `ST_AGENT` (including the current Fractal alias), or the configured
local person. This is the existing `read.declarations` authority, also required to read
publication definitions. Anonymous local sessions and limited observation pairings do not gain
that scope. Existing tokens and devices retain their recorded scopes. A remote client uses its
existing paired Fabric credential; this adds no listener or transport.

Rows show the recorded owner run, generation and step when present; legacy ownership is explicitly
not recorded. PRIMARY resolves `host "local"` against the declaration's origin, never against the
member serving the list. STANDBY NODE reports only whether a raw `standby` node is present. The
JSON keeps that node and unfamiliar declaration fields; it assigns no takeover meaning and adds
no run-twice warning in this declaration-only read. Opaque, unfamiliar,
stopped and oversized declarations remain visible rows rather than disappearing.

NEXT DECLARED is a nominal tick computed from the declaration at the page's `as_of`. It does not
predict admission, actual execution, completion, catch-up, current ownership or whether a prior
occurrence ran. A past one-shot has no future declared tick; stopped declarations stay listed
without a tick. A declaration the scheduler rejects, including invalid standby settings, has an
unknown next tick while its raw fields remain visible. Intervals need an explicit anchor,
otherwise the tick is unknown. Calendar ticks
use the scheduler's existing timezone calculation: the first real minute after a DST gap and the
first instant of a repeated local minute. An exact tick is included. This read changes no scheduler,
arming, catch-up, role, heartbeat or occurrence identity behavior.

KNOWLEDGE AGE is the age of the last successful **direct exchange with the declaration origin**
recorded on this member, even if the exchange was partial. It is not declaration freshness or proof
of complete applied history. A relay exchange does not establish contact with the origin. Missing,
unreadable or future timestamps are unknown. A local declaration has age zero at the local page
cut; it says nothing about any remote member. Completeness is always unknown.

Every page, including an empty one, ends with “No successful direct exchange recorded in …” and
names covered hosts whose receipts are older than the display threshold or unknown. The default
is five minutes; equality with the threshold is still within it. Normal successful idle exchanges
record receipts even without changed claims and recur at a nominal 30 seconds. Five minutes is ten
such intervals, not a promise under backoff, suspension, overload or a network partition.
**This is local knowledge; it does not say they are down or that the list is complete.**

Coverage is limited to the configured peers and this page's declaration-origin/primary hosts.
It is not the active fleet directory. At most 200 configured entries and two host references per
returned declaration feed at most 601 deduplicated point lookups including this member. The footer
covers at most 200 names; omitted configured entries and covered names are counted explicitly.
A 64 KiB footer byte budget can also omit names and marks the coverage partial. A configured host
is not thereby known to be an active member. Unknown membership stays unknown.

`--unheard-after` must be positive and no larger than 9,007,199,254,740,991 milliseconds,
the exact-integer ceiling shared by JSON and the generated clients.

The default page has 50 rows; `--limit` accepts 1 through 200. There is no `--all` or automatic page
traversal. Follow the printed cursor explicitly. It binds the serving instance/process, authenticated
session, page limit and display threshold, and expires after 15 minutes or a restart. Omit limit and
threshold on continuation to retain them, or repeat the same values. Each page uses a new local
read cut, so edits between pages can change what remains; this is not a frozen cross-page snapshot
or a fleet-wide inventory. A cursor cannot be moved to another host/session.

The SQL reader seeks the applied `desired` schedule prefix and joins only selected claim IDs for
origin metadata. It does not fold history, inspect runs or rebuild membership. Raw declaration JSON
is materialized up to 64 KiB per row; larger declarations have a visible `read_problem`, byte count
and unknown display fields. This is a read limit, not a new stored payload limit or a physical-cost
claim about SQLite. Pages stay within the existing 1 MiB client response ceiling and continue after
the actual last returned row. Metadata too large for a row/cursor returns a visible size error.

The additive client read is `schedules.list`, `GET /v1/client/schedules`, returning
`Envelope<SchedulesPage>` under `st3.client.v0`. Query fields are `limit`, `cursor` and
`unheard_after_ms`. Rust `schedules_list`, TypeScript `schedulesList` and Swift `schedulesList`
preserve unfamiliar state strings and raw declaration fields. Old servers return unsupported;
the CLI asks for a server upgrade and never falls back to a history scan. Old clients are unchanged.
