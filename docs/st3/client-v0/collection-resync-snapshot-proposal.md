# Proposal: collection resync with a snapshot and cursor

Status: revised contract proposal after Intake B's CHANGES-REQUESTED review of
`d8d7f8d64`; no implementation or wire behavior change yet. Scope is IVM-backed
collection subscriptions. The HTTP event-page `GONE` path in `api.rs` stays
unchanged.

## Current behavior and intended measure

Today `collection_ivm.rs` turns a journal gap into `Output::Resync` with
`delivered: None`, `again: true`, and `replace: true`. `deliver_ivm_collection`
sends a data-less `resync` frame, then its immediate refresh calls
`subscribe_snapshot`. That call reads the rows and boundary in one SQLite
snapshot and sends an authoritative `snapshot` on the **same subscription
ID**, without requiring client resubscription. This existing snapshot is
already internally cut-consistent. The defect addressed here is the two-frame,
two-capture recovery: a client can mark or clear the retained window on the
first frame before the second arrives, and the two captures add read and
delivery work to a gap recovery. Some clients choose to resubscribe on
`resync`, adding another first snapshot. This proposal does not claim to fix
an inconsistent existing snapshot or a measured daemon CPU regression.

The acceptance measure under a forced gap is one replacement frame and one
authorized SQLite capture for an opted-in subscription, versus today's
`resync` plus `snapshot` from two captures. Record frame count and the time
from gap detection to an applied authoritative window in a client fixture;
show no intermediate empty/stale state in that fixture. Preserve one-cut
rows, the existing ordinary snapshot byte bound, and no extra full-window
reads outside gap recovery. Do not claim a production latency, writer time,
or SLO gain until measured.

## Negotiation and wire shape

Advertise optional capability `collections` version **2** from
`client_v0::capabilities` in the client-v0 discovery response, granted under
the existing `read.projections` scope. Version 1 retains its current meaning.
A new client sends `"resync_snapshot": true` on a collection `subscribe`
only after it sees granted `collections` version >= 2. Without that
capability it sends the current command and handles the current data-less
`resync` followed by `snapshot`. An older daemon's serde command decoder
ignores an unknown subscribe field, but clients must not rely on that:
absence of version 2 means no enriched frame can be expected. Legacy clients
never opt in and see the identical sequence and fields they see today.

An opted-in journal-gap recovery sends one new, distinguishable frame:

```json
{
  "kind": "resync-snapshot",
  "id": "missions-tab",
  "collection": "missions",
  "code": "cursor-gap",
  "snapshot": { "id": "snapshot/…", "store_index": 123, "projection_version": "client-projection.v0" },
  "items": [],
  "order": [],
  "has_more": false,
  "cursor": "opaque-continuation"
}
```

`items`, `order`, `has_more`, and `snapshot` are the ordinary full-window
snapshot fields. The client atomically replaces the complete window and its
derived state, then continues on that subscription. It does not resubscribe
because of this frame. Existing data-less `resync` remains the retryable
not-ready/read-failure signal; it never carries rows. A legacy `snapshot`
remains the first reply and the replacement after a legacy gap.

For an opted-in subscription, its first `snapshot` also includes `cursor`.
Each later visible `changes` includes `from_cursor` equal to the last
client-visible cursor on that subscription generation and a new `cursor` for
the delivered continuation. The new client uses this chain to reject a
duplicate, reordered, or cross-generation change and request a fresh
subscription. The server keeps the last **wire** cursor across silent internal
cursor advances, so the next visible `from_cursor` still matches what the
client holds. `resync-snapshot` starts a new chain and replaces both rows and
the wire cursor. The token is opaque, unique to a subscription generation and
delivery, bound to its authority, collection, and filters. It identifies the
server-held IVM continuation at that delivery's cut; it is not a graph index,
pagination cursor, or credential, and clients never echo it as authority.
This makes the field useful to client consumers without exposing view-global
keys or raw provider state.

## Capture and delivery across layers

The change begins in `graph_watch_ivm.rs`: the `page_with_rows` gap branch
currently returns `ViewPage::Resync { reason, boundary }` with no rows. Extend
that branch to invoke a full-window callback **inside its existing SQLite
snapshot**, after authorization, and return the resulting window with the
boundary. Use the same `collection_ivm.rs` adapter row function as ordinary
`subscribe_snapshot`; do not introduce a second row reader or a second
independent read after gap detection. `collection_ivm.rs` packages that
window and a new `Delivered` built from the same boundary, then
`client_v0.rs::deliver_ivm_collection` sends the new frame and acknowledges
both together. The source cut in its `ClientSnapshot` must equal the
boundary's admitted and projected store index. If projection admission is
behind the store, fail the capture with the existing pending/refusal path;
never pair rows from a later store cut with an older boundary.

Revalidate the session and `read.projections` before capture. Apply the same
person, actor, status, subject, row visibility, limit and collection-specific
scope/ownership checks as an ordinary snapshot. Changed or revoked authority
cannot inherit rows from the previous window. The full window discards
partial key/availability pages and old continuation state, including an
expired journal floor or provider replacement. Set both internal cursors at
the replacement boundary, and set `again: false` for the gap alone. The
already-installed notice receiver catches a concurrent commit either in
the capture or in a following read. `has_more` describes bounded collection
rows, never journal page continuation. A later ordinary page whose snapshot
version changed follows the existing page-restart rule; it cannot splice a
different page version into the replacement.

Only after a full frame is sent may the socket replace `previous`, `order`,
`has_more`, its wire cursor, and `Delivered`. A row/read/authorization error
uses the existing retryable `resync` or terminal `error` classification and
acknowledges no candidate; no partial replacement is sent. A not-ready view
keeps the existing `Unavailable` behavior, not an invented snapshot. Split
frame encoding/size validation from transport send for this branch. If
serialization fails, send one small terminal `error` with code `internal`;
if the encoded replacement exceeds `CLIENT_MAX_RESPONSE_BYTES` (the same
1 MiB bound as an ordinary snapshot), send one small terminal `error` with
code `collection-snapshot-too-large`. Both have `retryable: false`, end the
subscription, and acknowledge no candidate. If even that error cannot be
sent because the socket closed, close it without acknowledgment. Do not
retry the identical oversize or serialization failure.

One subscription generation may deliver at most **one** gap replacement
without an intervening successfully delivered non-gap journal page that
advances the new cursor (a silent page counts if it advances). A second gap
before such progress sends one terminal `error` with code `cursor-gap-loop`,
`retryable: false`, then drops that subscription without acknowledging the
second candidate. This is a visible failure, not a polling/retry delay.
Successful non-gap progress rearms the bound for a later genuine retention
gap. A quiet store therefore produces no second resync; rapid expiry can
produce a visible error instead of an unbounded replacement loop.

## Contract surface and consumer coordination

Implementation changes the subscribe and frame definitions in
`docs/st3/client-v0/schemas/client-v0.schema.json`, the collection stream
entry in `docs/st3/client-v0/schemas/operations.json`, examples and
compatibility text in `docs/st3/client-v0/collections.md`, contract fixtures
and `docs/st3/client-v0/fixtures/manifest.json`, generated TypeScript,
Swift, and Rust client models/schemas (including
`crates/st3-client/src/generated.rs`), and a `release-notes/*.json` entry. Regenerate
and check generated clients rather than hand-editing generated files. Queue
the implementation PR alone with schema/codegen and old-client compatibility
proofs because it changes the client-v0 wire contract.

Consumers that recognize the new frame include the TypeScript client, Swift
client, `st3-client` Rust collection follower, `st3-feed`/stui, and the iOS
and web collection adapters that use them. Their handling must be updated or
explicitly remain on v1. Coordinate `st3-feed`/stui with concurrent #3166;
this isolated worktree does not edit those files. A new client talking to an
old daemon follows the v1 path. An old client talking to a v2 daemon never
opts in and keeps the current path.

## Verification required for implementation

- Negotiate capability v2 and v1/absent, including a new client against an
  old daemon whose decoder ignores the unknown field; only v2 sends opt-in.
- Force a journal gap: opted-in receives exactly one `resync-snapshot`; v1
  receives the existing `resync` then `snapshot` on the same ID. Check both
  cursor chains and atomic client window replacement.
- Pause projection admission behind the store while producing a gap: no
  mixed-cut frame or cursor acknowledgment; after catch-up, rows and boundary
  use one cut. Exercise scoped filters and authority revocation at capture.
- Expire a key page and an availability page, replace the provider, change
  snapshot version between pages, and commit during capture. Verify no
  skipped visible change and no partial-window splice.
- Force a second gap without non-gap cursor progress: exactly one typed
  `cursor-gap-loop` error and no further reads for that subscription. Check
  that successful page progress rearms the bound.
- Force oversize encoding, serialization refusal, and socket-send failure:
  one visible terminal error when transport allows, no cursor/cache
  acknowledgment, and no retry of the same invalid frame.
- Check schema fixtures, generated TypeScript/Swift and Rust client parsing,
  old-client behavior, and unchanged `api.rs` event-page `GONE` tests.
