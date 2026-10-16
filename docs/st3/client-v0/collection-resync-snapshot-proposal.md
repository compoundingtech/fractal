# Proposal: collection resync with a snapshot and cursor

Status: source contract proposal; no implementation or protocol change yet.
Source seam: `deliver_ivm_collection`, `Output::Resync`, in
`crates/st3/src/api/client_v0.rs`. This concerns only IVM-backed collection
subscriptions. The HTTP event-page `GONE` path in `api.rs` is unchanged.

## Wire contract

Add an optional `resync_snapshot: true` field to a collection `subscribe`
command, advertised by a new optional collections capability version. The
default is false. A client that does not opt in receives the existing
`resync` frame and subsequent fresh `snapshot` behavior, including a client
that resubscribes with the same ID. No existing frame or required field is
removed or reinterpreted.

For an opted-in subscription whose delivered IVM cursor has a journal gap,
send exactly one replacement frame for that capture:

```json
{
  "kind": "resync",
  "id": "missions-tab",
  "collection": "missions",
  "code": "cursor-gap",
  "retryable": true,
  "snapshot": { "id": "snapshot/…", "store_index": 123, "projection_version": "client-projection.v0" },
  "items": [],
  "order": [],
  "has_more": false,
  "cursor": "opaque-continuation"
}
```

`items`, `order`, `has_more`, and `snapshot` have exactly the ordinary
collection snapshot meanings. `cursor` is an opaque identifier for the
continuation the server retains for this subscription; it is not a graph
index, a pagination cursor, or authority for a later request. The client
atomically **replaces** its entire window and derived state with this frame,
then applies later `changes` on the same subscription. It must not combine
the replacement with prior rows or request another resync merely because this
frame says `kind: resync`. A client that cannot recognize the complete
replacement still uses the existing resubscribe path. The server does not
require the client to echo `cursor`; after delivery it continues from the
same retained cursor. The token must not expose raw view-global keys or
provider state, and must be bound to this subscription generation, requested
collection and filters, and current authority. Its exact encoding remains
private to the server.

## Capture and acknowledgment invariants

On `ViewPage::Resync`, read the full bounded window through the collection
adapter within the **same** SQLite snapshot that checked the journal gap and
captured the new `Boundary`. Do not construct the frame from the old cached
rows, and do not take a second independent read after receiving the gap.
Revalidate the session and `read.projections` scope before capture. Apply the
same person, actor, status, subject, row visibility, limit, and byte bounds
as an ordinary collection snapshot; collection-specific scopes and ownership
checks still apply. A changed or revoked authority cannot inherit a prior
window. The public `snapshot.store_index` equals that boundary's admitted
and projected source cut; the delivered key and availability cursors, provider
identity, snapshot version, and any continuation page version come from that
boundary. Thus rows and cursor name one cut.

The replacement is a full current window even if the gap occurred halfway
through a key or availability page. Discard partial page results and the old
continuation. Set the new cursor at the captured boundary; the first read
after successful delivery starts there, with no `again` solely because of
the gap. A concurrent commit is seen in this snapshot or wakes a subsequent
read through the already installed socket notice receiver. The client-visible
`has_more` continues to describe the bounded collection window, not journal
page continuation. If a later page changes snapshot version, use the existing
page restart rule; do not splice pages from different versions into one
replacement. An expired journal floor or provider replacement triggers this
fresh capture, never repeated paging from the expired cursor.

Advance the held cursor and replace `previous`, `order`, and `has_more` only
after the whole replacement frame is successfully sent. A closed socket,
serialization failure, or response-size refusal acknowledges nothing. A
capture or authorization error sends the existing `resync` or permanent
`error` classification and acknowledges no candidate; it must not send a
partial replacement. A not-ready view retains the current `Unavailable`
behavior until it is ready, without claiming a fresh snapshot. A fresh
capture cannot immediately revisit the same gap in a quiet store: its next
read starts at the new boundary. A genuinely new retention gap under ongoing
write pressure may require another replacement, but the server must not
schedule an unconditional gap/retry loop.

## Review and verification before implementation

- Test a forced expired journal and provider replacement: each opted-in
  frame contains a newly authorized full window and a cursor from its cut;
  the next quiet read emits no second resync.
- Test authorization revocation and each supported filter at capture time;
  no previously visible row or view-global key leaks into the replacement.
- Test a gap at the last key page and at the last availability page, with a
  concurrent commit and a changed page version. The next frame must follow
  the captured cursor without skipping a visible change.
- Test send failure and read refusal: no cursor or cached-window advancement.
  Keep non-opted clients' current resync then snapshot behavior, and confirm
  `api.rs` event-page `GONE` responses are unchanged.
