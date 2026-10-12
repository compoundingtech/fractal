# iOS terminal attachment to the owner

## Decision

Use the owning member's paired client service, `st3/client/FLEET_ID`, over
Fabric. Keep the phone's existing `TerminalScreen` rendering, collections stream,
terminal attachment capabilities, and incarnation fences. The ordinary gateway
continues serving Home, conversations, and other windows. Only the followed
terminal gets an owner client and a terminal-only collections socket.

Source baseline: `707b2522f`; direct terminal attachment was introduced by
[PR #2069](https://github.com/compoundingtech/smalltalk/pull/2069). This document
is a design, not evidence of a device build or successful owner attachment. All
deployment examples here use invented identities. The immutable mission design
contains the separate operations appendix with the actual host/grant matrix.

The latest mission instruction requires falling back through the daemon for
every failed direct attachment, including refusal and failed local PTY open.
It supersedes the earlier instruction and PR #2069's refusal behavior. A refusal
remains visible in the route reason. The fallback must still prove the selected
terminal and incarnation; it cannot accept a replacement process.

Fallback is a per-device local setting, defaulting to always falling back.
The person can choose `fabric only, no fallback`; a failed direct route then
returns its reason and never opens through the daemon. stui stores this in
`prefs.rs` with a palette entry; iOS exposes it in Fleet's carrier section.
It is never a graph claim. This policy governs terminal routes as well as the
phone's carrier fallback; reconcile it with the existing local carrier switch
instead of creating two contradictory controls.

## Why the client service

| Route | Latency work | Phone implementation | Authorization |
| --- | --- | --- | --- |
| PTY route, `st3/pty/FLEET_ID` | Avoids owner screen serialization and daemon byte carrying | Route line, binary PTY protocol, replay, modes, geometry, and a terminal emulator compatible with the existing screen UI | Existing route proves subject/incarnation, but carries no paired person credential or read-only scope; serving this to a phone would require new owner authorization |
| Owner client, `st3/client/FLEET_ID` | Removes gateway-to-owner relay from terminal reads and controls; retains owner screen rendering and serialization | Reuses HTTP, WebSocket, screen rendering, attachment capabilities, and signed actions; adds route selection and native target management | Paired-only router authenticates the device grant and person; read and control remain separate scopes |

Choose the smaller client-service change. The PTY route's lower theoretical cost
does not justify implementing an emulator and a new paired authorization boundary.
The owner client is direct to the owner even if iroh selects an encrypted relay.
Actual cold connection and first-screen times must be measured; source inspection
does not establish a speedup. Endpoint discovery, QUIC admission, HTTP requests,
and the new collections socket all contribute to opening time.

`route.rs` sends one newline-delimited JSON `RouteRequest` with `op`, `name`,
`subject`, and `incarnation`, then hands the stream to the PTY client after the
owner's answer. Its existing `Refused`/`Unreachable` distinction is retained for
diagnosis. It is not a paired-device access check. `direct.rs` currently requires
both read and control before that route, while iOS read-only following must work.

## Discovering the owner

The paired client API currently projects runtime `owner_host_id` and
`incarnation_id`, but exposes no authenticated mapping from that owner to its
Fabric node. The internal fleet membership endpoint is outside `fabric_router`.
Do not expose the internal API or put real node IDs in app source/build defaults.

Add an optional client read, `GET /v1/client/terminals/{id}/route`, requiring
`terminal.read`. Its response has the usual envelope and a route descriptor:
`terminal_id`, `runtime_incarnation`, `owner_host_id`, and an optional
`fabric` object with `node` and `service`. Resolve the selected subject/runtime
from the gateway's existing runtime authority and the current fleet membership
view; select the owner's advertised Fabric endpoint and derive the service as
`st3/client/` plus this daemon's fleet ID. Return no fabric object for absent,
ambiguous, or non-current membership. Do not guess by peer name on the phone.

This read is metadata from one store snapshot, without contacting the owner or
requiring a successful gateway-to-owner relay. A paired owner with a known node
can therefore be tried even if the gateway relay is unavailable. It reveals no
fleet secret, member private key, peer-list configuration, or privileged socket.
Update the API schema, generated TypeScript client, and server contract tests.
An older daemon's missing endpoint simply keeps the existing gateway path.

Pin the descriptor's terminal ID and incarnation before dialing. If an existing
follow already pinned an incarnation, a newly read descriptor cannot replace it.
Check every owner/gateway screen and attachment against that same pin. The first
owner screen must also match it, even though the screen read itself takes no
expected-incarnation parameter. When metadata is unavailable on an older daemon,
learn the initial pin through the existing gateway screen read as today.

Only use the gateway-authenticated current-fleet descriptor to select an owner.
A pasted arbitrary Fabric target must never receive the saved gateway credential.
An owner authentication failure does not initiate pairing or copy/create a grant.

## Owner and device authority

Expose the owner's configured `client_gateway_socket` using its normal
`fabric_router`, never the privileged `st3.sock`. Fabric first authenticates the
phone's NodeID and admits the exact service grant. The tunnel then carries the
ordinary bearer, feature headers, WebSocket capability subprotocol, and signed
action bytes unchanged. Fabric admission grants transport access only.

`client_v0::authenticate` hashes the bearer and resolves
`custom.client.pairing-completed` in the local graph; it does not require the
pairing claim's origin to equal the receiving host. `paired_client_session`
checks expiry/revocation and derives the exact pairing grant, session actor,
concrete delegated person, and scopes. This permits the same credential on a
current owner after its existing fleet replication has delivered the pairing
and relevant principal/device key records. Replication is a prerequisite, not
permission to mint a broader pairing. Missing or stale records may cause the
owner to refuse; the gateway remains available under the normal fence checks.

On the owner, screen/stream/attach/detach require `terminal.read`; input and resize
require `terminal.control` and the existing action authentication. Preserve the
device signing key and enrolled principal proof, without substituting trusted
Unix person headers. A read-only pairing opens the owner screen and attachment
but never gains control. Pairing/session revocation and capabilities remain
checked by the existing owner client service; direct discovery adds no scope.

Attachments belong to the issuing gateway and authenticated session. Obtain a
fresh owner attachment and subscribe on the owner's socket. Never reuse a gateway
attachment capability on the owner or an owner capability on the gateway.
Detach through the client that issued it. Fallback creates a new gateway
attachment with a fresh gateway snapshot and the same pinned incarnation.

## Native bridge and app lifecycle

The current native `BRIDGE: Mutex<Option<Bridge>>` makes a new dial stop the old
bridge. That would break the gateway feed. Replace the single slot with explicit
target handles, while preserving the legacy gateway dial/stop API. Each target
has its own `127.0.0.1:0` listener, fixed peer/service, cancellation state, and
the existing 16 concurrent tunnel session bound. Reuse an app-owned iroh endpoint
for simultaneous targets of the same identity/path mode; do not create competing
endpoints with the same secret merely to add a listener. Closing a target stops
only its listener/sessions; the shared endpoint closes after its last target.
Reuse the gateway target when it already names the owner. One gateway and at
most one followed terminal target suffice for the normal app.

Add target-aware native open/close/diagnostics through Rust FFI, Swift, and the
TypeScript wrapper. Preserve the old module interface for carrier/proof callers.
Only a live native return value selects loopback; never save a listener port.
Background/destroy closes all listeners and sessions. The foreground gate then
reopens only the targets still wanted. A cancelled late dial cannot publish a
route or close a newer target. Refusal diagnostics belong to the target, rather
than a global latest-attempt statistic from another listener.

The native dial currently only creates a listener; the real Fabric handshake
happens when HTTP connects. An owner route is ready only after owner
authentication, fenced attachment, and subscription succeed. Bound the complete
network route attempt to the existing stui four-second budget, not just native
listener creation, and cancel its unfinished work when the daemon takes over.
No additional polling is introduced. Listen to collections frames, foreground
changes, close/error events, and explicit follow/close actions.

`Feed.followTerminal`/`TerminalFollow` gain a terminal route coordinator. The
owner uses a terminal-only collections socket; the ordinary feed stays alive.
Expose route updates to `Terminal.tsx` separately from `onIssue`, so a good frame
clears a transient screen error without erasing a Fabric failure reason. Closing
the follow unsubscribes, detaches, and releases its owner target. Hold the pin
across socket loss and foreground reattachment.

Route terminal input and resize through the current terminal client as well as
reads/attach. Its snapshot IDs and sequence fences must come from that same
client; the global gateway `lastSnapshot` is invalid on the owner. Preserve
raw/key incarnation-only input and the existing older-daemon sequence requirement
handling. Do not replay unconfirmed keys through another route. A failed control
reports uncertainty and later input uses the newly established route.

## Route failure and incarnation safety

Display the same route words as stui: `Fabric to HOST`,
`via the daemon (Fabric: REASON)`, and remembered failure wording
`via the daemon (Fabric: REASON, not tried again yet)`. stui's local path keeps
`local PTY`; its failed local open uses `via the daemon (local PTY: REASON)`.
The phone never claims a local PTY route. A build without native Fabric, missing
route metadata, or missing peer/service/grant uses the gateway with a useful
reason. Fabric admission or owner refusal also falls back visibly, as required
by the later mission instruction. The gateway can itself refuse; show that error.
When `fabric only, no fallback` is selected, each of those failures remains an
error with the same reason and no gateway attach. Devices without a Fabric build
retain today's gateway behavior by default. Settings changes affect subsequent
route selection without replaying input or replacing the pinned terminal.

Keep 60 seconds of an owner-level unreachable route, keyed by owner, node, and
service for this paired connection. Record the reason/time only after the failed
attempt; skipped opens do not extend it or grow the message. Expiry permits the
next real open rather than scheduling probes. Success removes the entry. A
per-terminal refusal, missing incarnation, credential/scope denial, or changed
runtime does not mark all the owner's terminals unreachable. Remember missing
service/transport admission failures at the route level; do not retry a dead
route on every terminal open. Changes to pairing/target invalidate that memory.
Keep stui's existing 60-second owner memory with the same classification.

The gateway already guarantees the same terminal. On iOS,
`withFreshTerminalFence` reads that specific terminal, compares
`runtime_incarnation`, then supplies it with the attach/control fence;
`TerminalFollow.screen` rejects frames with another incarnation. The server's
`remote_terminal_live_session` resolves `terminal_subject(id)` and rejects a
different incarnation; remote attach additionally checks the owner's screen.
For stui, `attach_terminal` calls `raw_terminal_attachment(terminal, incarnation)`;
`raw_terminal::attachment` and the stream query resolve the same subject and
expected incarnation, and the raw lease binds subject/runtime/owner. Therefore
direct refusal can safely lead to a daemon attempt, which may reject too.

Do not fall back by restarting discovery and accepting its new incarnation.
Check both initial and reattached frames/attachment responses and retain the pin
through every transport change. Focus changes only focus, a tap only focuses
the keyboard, and routing never synthesizes keys or removes terminals.

## Operations contract

For each intended owner, operations exposes `st3/client/FLEET_ID` to that host's
configured paired-only client Unix socket and admits exactly the phone's public
Fabric NodeID to that service. The gateway host needs the same service if used
as the phone's ordinary Fabric carrier. For example, on invented hosts
`owner-a`, `owner-b`, and `gateway-a`, a new peer entry is:

```toml
[[peers]]
id = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
name = "demo-phone"
allow = ["st3/client/demo-fleet"]
```

This is a stanza in an existing format-2 peer book, not a replacement for the
file. No `st3/pty`, replication, shell, exec, sync, wildcard, or privileged API
grant is required. No inbound phone peer book is required; the dialer pins the
advertised member NodeID. Operations confirms exposures, applies/reloads peer
policy, and checks admission without granting more client scopes. This design
step neither changes host configuration nor deploys/restarts anything.

## Build verification

Use fake-feed Node tests for metadata availability, direct choice, admission and
owner refusal falling back, missing Fabric/grant, network failures, the complete
attempt budget, unchanged-incarnation fallback, restart rejection, owner memory
expiry without extension, lifecycle cancellation, capability locality, and input
snapshot provenance. Exercise both the default fallback and the local
`fabric only, no fallback` choice, including refusal and missing Fabric.
Test route validation/labels/memory in shared
`clients/typescript/st3-views`, with transport/key-store dependencies remaining
in the app. Add server tests for the optional authorized route read and native
tests for simultaneous targets and target-specific stop with the 16-session
bound. No code or tests are implemented by this design step.

For the build proof, use an isolated two-daemon fleet on the authorized Linux
host and invented principals. Complete a read-only pairing, replicate the grant,
and use the phone-side TypeScript client plus Linux-native carrier against a
terminal owned by the other member. Measure start-to-attachment and
start-to-first-screen independently for direct owner service and gateway relay,
recording cold/warm transport state and selected direct/relay path. Assert a
control denial, successful read-only stream, and refusal of a restarted terminal.
Never use a member's screen or build/install the iOS app during this work.

Run the specified iOS typecheck/tests, shared views tests, and Rust nextest checks
outside the agent seat on the authorized host. Record actual measurements and
material limits in the build PR. A Linux bridge proof verifies phone-side logic
and protocol, not Swift linkage or physical-device behavior; those remain with
the app builder. Each PR includes its release-notes fragment, passes the public
repository checker before every push, and lands through the merge queue.
