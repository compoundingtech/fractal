# fractal-client

Reusable typed Rust client for `st3.client.v0`. It consumes versioned JSON projections and actions;
it never parses Fractal CLI output, KDL, Markdown, raw claim envelopes, or harness transcripts.

```rust
let client = fractal_client::Client::unix("/path/to/st3.sock");
let capabilities = client.capabilities().await?;
let work = client.work_list(None, Some(100), false).await?;
```

Use `Client::unix_gateway` with a pairing credential for a paired Unix gateway, or
`Client::fabric_loopback` with an HTTP(S) gateway URL and pairing credential for remote access.
The HTTP(S) transport does not require Fabric; a reachable authenticated gateway is sufficient.

For a native Fabric connection, use the remote NodeID or unique trusted peer name:

```rust
let endpoint = fractal_client::fabric::FabricEndpoint::parse(
    "fabric://studio/st3-client-v0",
)?;
let client = fractal_client::Client::fabric(endpoint).with_credential(credential);
let capabilities = client.capabilities().await?;
```

The local Fabric daemon resolves names case-insensitively and creates the tunnel;
there is no HTTP proxy or manual TCP listener. Fabric trust and a protocol grant
are prerequisites. The transport permits only `/v1/client/` and `/v1/health`;
pairing credentials and device signing still follow the remote gateway's normal
authorization policy.

Every manifest read and mutation has a named typed method; raw collection strings, paths, JSON
parameters, and manually paired action discriminators are private implementation details.
Mutation callers pass the latest response snapshot and exact resource fences. `ClientError::Api`
preserves stable error codes such as `stale-fence`, `runtime-not-local`, and `cursor-gap`.

## Shared scenario fixtures

`scenarios::load_index(path)` reads the shared world catalog (including cast and slice paths).
`scenarios::load_slice(path)` reads a committed slice without embedding or pinning its bytes.
Call `slice.rebased(now_epoch_ms)?` to shift only its declared instant pointers, then
`slice.decode()?` to obtain located client-v0 models from both state and timeline.
Roster decoding includes the session/device inventories; details decoding includes launches
and launch variants. Resource wire values are boxed inside `WireValue`.
Strict slices reject unknown object fields, enum cases and discriminated-union tags while
preserving explicitly schema-declared open maps (including error details) and variant-specific
constraints such as device variant requirements. Tolerant slices
retain the production models' forward-compatible behavior. Diagnostic `unknown` repair
witnesses never replace wire data. Replay offsets, durations and opaque content are not shifted.
Object intersections are composed before closing their fields. Validation follows the kit's
safe-integer and time-codec bounds, timestamp grammar, UTF-16 string lengths and ECMAScript
whitespace; `uri-reference` is annotation-only. Integral numeric spellings such as `1.0` decode
like `1`. Fields previously narrower than their schema's integer domain use `u64` for nonnegative
domains and `i64` when negative values are permitted; explicitly bounded fields keep their types.
Large capability versions, signed terminal cursor coordinates and terminal process exit codes
therefore round-trip without truncation. Scenario validation still enforces schema bounds.

## Retained typed collection follows

`follow::FollowClient::connect(client.clone()).await?` owns one collections socket and
one reconnect loop. Clone that handle, rather than connecting for each model:

```rust
use fractal_client::follow::{AttentionDescriptor, FollowClient};
use futures_util::StreamExt;

let follows = FollowClient::connect(client.clone()).await?;
let mut attention = follows.watch(AttentionDescriptor::default(), 100).await?;
while let Some(state) = attention.next().await {
    // Inspect state.freshness and state.error separately from the retained value.
    if let Some(window) = &state.value {
        for item in &window.items {
            println!("{}", item.title);
        }
    }
}
```

`current()` reads the latest retained state without waiting. Receiver clones share
the subscription and immutable `Arc` rows; dropping the last receiver unsubscribes.
The stream coalesces intermediate states for slow readers, retaining one state rather
than an event queue. A window contains the server's ordered rows, snapshot and
`has_more`; it is not complete history. Limits are 16 subscriptions per connection,
1–200 rows per window, 16 pending watch requests, and 32 queued socket writes plus one
in-flight command. Selectors must fit within the 16 KiB command budget including its
reserved envelope. `AttentionDescriptor.person` selects the authorized person.

Writes are polled alongside admission and lifecycle signals, never awaited by a
blocking worker branch. Dropping a watcher during a blocked send cancels the socket
instead of stranding cleanup; remaining watches rejoin on one fresh connection.
Queue overflow likewise reconnects rather than retaining more commands. Dropping
the last `FollowClient` handle shuts down immediately, without waiting for the peer
to read or acknowledge a close handshake; retained receiver streams then end.
The write liveness contract matches the server's eight-second collection send
deadline: if any command, including cleanup's unsubscribe, cannot flush in eight
seconds, the client drops the socket and reconciles retained watches on reconnect.
Pending cleanup remains tracked until its unsubscribe flushes or the socket resets,
even after the subscription leaves the routing table. Reset is the terminal release
action for a stuck unsubscribe, because only delivery or socket loss releases the
server-side subscription.

Reconnect and resync retain the last window as stale until a new snapshot arrives.
Each resubscribe uses a new wire ID so queued frames from older generations cannot
overwrite current state. Transport loss is stale without a subscription error;
server subscription failures and malformed rows remain explicit errors. A successful
snapshot clears the error. Missing order rows fail the update atomically rather than
silently yielding an incomplete fresh window.

Implement `follow::Descriptor` to pair an existing collection selector with a typed
raw-row decoder (`decode_row` supports generated Serde models). No resource-to-JSON
round trip is needed. Descriptors do not add server collections or capabilities;
unsupported selectors still fail through the normal subscription error.

## Raw PTY connectors

`raw_terminal_attachment(terminal_id, runtime_incarnation, RawTerminalMode::Attach)` acquires
a single-use capability fenced to the terminal's current runtime incarnation. `RawTerminalMode::Peek`
requests a read-only connection instead. The method extracts a `RawTerminalAttachment` from
the standard client-v0 response envelope: it carries the terminal ID, incarnation, owner host,
mode, and stream capability.
The normal terminal stream authentication and terminal access rules also apply to raw streams.

`raw_terminal_stream(&attachment)` consumes that capability at the gateway and returns a
`tokio::net::UnixStream`, usable as a `pty-terminal` connector. It negotiates `fractal.client.pty.v0`
with the capability in the secondary `fractal.cap.<capability>` WebSocket subprotocol.
Both trusted local Unix and paired Unix/HTTP(S) clients expose the same connector.
Every receiving host must run the #3430 accepting release before this sender
cutover is deployed. Pre-alias receivers are unsupported; there is no old-name retry.

```rust
let attachment = client
    .raw_terminal_attachment(
        "terminal/demo",
        "demo-runtime:incarnation",
        fractal_client::RawTerminalMode::Attach,
    )
    .await?;
let connector = client.raw_terminal_stream(&attachment).await?;
// Give connector to pty-terminal; the consumer sends its own ATTACH frame.
```

The bridge carries binary PTY bytes unchanged in both directions: it does not decode PTY
frames, send ATTACH/PEEK, or turn output into screen projections. WebSocket message boundaries
are not PTY frame boundaries. Buffering is bounded and a slow consumer applies backpressure.
Dropping the connector closes the remote attachment and cancels the bridge, including when a
write is stalled. A closed remote stream produces EOF on the connector.

Reconnect by explicitly acquiring a **new** attachment and opening a new connector; capabilities
cannot be replayed, and the client does not transparently reconnect or reuse one.

### Foreground observation leases

PEEK streams expire after 60 seconds without selected foreground observation and after
300 seconds absolute. PTY traffic does not renew them. Use
`raw_terminal_stream_controlled(&attachment)` to obtain
`RawTerminalStream { stream, activity }`; clone `activity` and call
`activity.selected_use().await` only while the terminal is selected in the foreground
(approximately every 20 seconds). Stop sending controls on deselection or backgrounding.
The byte-only connector has no automatic renewal and remains idle-expiring.
ATTACH streams do not have observation leases: calling `selected_use()` on their activity
handle returns a local protocol error without sending a control or closing the PTY stream.

Rotate before the absolute deadline by acquiring a fresh capability and connector. Send the
existing PEEK initialization frame and receive fresh GEOMETRY and SCREEN before replacing
the visible surface; then drop the old connector. Expired streams cannot be extended.
Selected-use success means the control was sent, not that renewal was acknowledged.
Revocation or lost authority watches closes existing streams; recover through fresh
attachment/readiness APIs, never by replaying an old capability.

Paired observations require the pairing issuer's gateway. `pairing.revoke` on another
member fails without committing, with HTTP 409 `issuer-required` and
`details.issuer_host_id`. Automatic cross-gateway revocation routing is not provided.

Run `cargo run -p fractal-client-codegen -- --check` from the repository root to verify the generated
models and operation surfaces exactly match the normative schema and operation manifest.
