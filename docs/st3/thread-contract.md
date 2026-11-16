# Fractal topic threads

Status: proposed contract for review. The examples describe the intended behavior. The existing
client protocol and daemon do not yet provide this thread resource.

Fractal already stores addressed messages and their `in_reply_to` edges. Inbox's **Chat about
this** currently finds a discussion by the message subject. That works while returning to the
same Inbox item. A fresh request with a different title can refer to the same discussion even
though it needs its own answer and decision history.

## Identity and history

A thread has an opaque durable identity in the `thread/` family. Its editable title, current
state, conversation, message history and related items are read through that identity. A
new thread started from an Inbox item can commit its creation, origin link and first message
together. A client may hold an unsent draft without publishing an empty thread.

The thread ID is independent of the first message ID. Anchoring it to the first message could
reuse reply-chain lookup, but would couple the discussion's identity and availability to that
message's delivery, visibility and retention. A separate identity makes rename, reopening and
reference from other resources explicit. The cost is a durable thread projection and admission
rules for mutations and links.

Every message still has its own message ID and ordinary sender attribution. Its thread
reference is explicit. A reply to a specific message records a thread quote whose parent belongs
to the same thread. Ordinary `in_reply_to` retains its request settlement behavior. The reply can
appear in the agent conversation and in the thread as one message. Clients use authoritative order, current conversation visibility and stable cursors.
Subjects, titles, text similarity, and agent incarnation are presentation or context data.

## Links and lifecycle

A person can start an inline thread from a readable Inbox item. An agent can attach a fresh ask
to an existing thread when authorized on both sides. A reply in the thread and a follow-up ask
are separate operations. An ask created from a thread can carry the thread reference explicitly.
An ask created elsewhere must explicitly attach to that thread. Closing, answering or replacing
one ask leaves the thread and the other asks intact. A fresh ask may link to a closed thread
and show its full history without reopening it. An explicit predecessor and reason can support
a future decision-history view. Existing mission step gates own dependency trees and
conditional activation.

The initial lifecycle is Open, Close and Reopen. Closing moves a thread into Recently closed
threads, leaves history readable and keeps any linked ask answerable. An ordinary incoming reply
from the agent joins closed history without reopening the thread, adding an alert or resurrecting
the Inbox row. When admission sees a closed thread, the owning person must explicitly reopen it
before sending a reply. A fresh linked ask can still show the closed thread. An authorized rename changes the shared title everywhere and
preserves the thread, messages, links, unread state and current lifecycle.

Show rename, close and reopen as events in the thread timeline with their actor and time.
Project each event from its accepted lifecycle claim and reuse the operation identity to
deduplicate retries. These events do not add messages, notifications or unread replies.

Rename shows the exact previous and new titles. The request identifies the title claim
the editor saw and supplies the new title. The admitting daemon checks that the observed
claim is still its effective title in the writer transaction. A stale title returns a
conflict so the editor can refresh. The accepted rename retains the previous title from
that claim, its identity
and the new title together, so timeline rendering does not depend on retaining the older
claim body or inferring the title from replica arrival order.

An embedded panel's disclosure state is local to its client and placement. A compact row carries
the title, message count, lifecycle status and any unread hint on one line where space permits.
Expanding or opening a dedicated thread tab navigates to the same resource. Agent sessions and
missions can each embed several threads, with separate disclosure state for each. A thread's
display never requires the reader to join the unrelated portions of agent transcripts.

Use the conversation composer with an explicit destination. When a thread is active, the
composer sends into it without a separate reply mode. A quoted reply adds the message target.
Agent conversation and thread drafts are separate and survive switching destinations.

Track read progress per reader and thread against ordered messages. Moving a cursor forward can
clear an unread reply on the existing Inbox row without creating an ask, changing the alert count
or resolving an approval. Which visible interaction advances the read cursor should be tested
with real UI visibility and reviewed before client implementations treat it as read.

## Daemon boundary

Each thread belongs to one existing person and agent conversation. Reuse that conversation's
principal identities, reader authorization, sender attribution and grant enforcement. The
person can manage the thread's title and lifecycle. The agent can send replies and attach an
ask it owns in that conversation. Threads do not add an audience editor, invitations or a
second access list. A mission or session link makes the thread discoverable only to readers
who could already read that conversation. A new agent incarnation can add session context
while the thread keeps the same owning conversation.

Use durable claims as authority and bounded, rebuildable projections for summaries by context
and ordered message windows. Subscriptions advance by the same data and policy cut as their
reads, with explicit reauthorization on scope or source changes. A link is a reference only.
It cannot grant conversation access or reveal an otherwise inaccessible context's title. Direct reads and collection reads apply the same
visibility rules. Filter unauthorized summaries, message edges, counts and source links before
they leave the daemon.

| Operation | Admission boundary |
| --- | --- |
| List, read or subscribe | The existing reader and grant checks for the owning conversation. Apply context authorization to context listings and source links. Bind cursors to the reader, scope, thread or context, and revision semantics. |
| Start or reply | An attributed authenticated principal and the existing send authority for the owning conversation. Validate an optional quoted parent against the same thread and its visibility. |
| Rename, close or reopen | The owning person through an authorized client action. Sending messages and answering asks confer no lifecycle authority. |
| Attach a follow-up | Send authority in the owning conversation and authority to create or update the linked source. A link cannot broaden either audience. |
| Answer an ask | The existing ask action and exact ask authority. The thread identity cannot authorize or stand in for an answer. |

Writes validate authority in the admitting daemon transaction. Bind idempotency to the actor,
operation and target, and reauthorize receipt recovery after revocation. Rename, Close and Reopen
check the metadata revision visible at admission. A stale client refreshes and may retry. An
accepted incoming reply remains append-only and does not change the title or lifecycle
revision. Existing historical subject-based discussion remains readable through its existing
path. There is no inferred migration from subjects to durable thread IDs.

### Fleet behavior and receipts

Reuse existing Fractal conversation routing, claim replication, authorization and request keys.
Thread identity follows the stable conversation while agent placement determines where its
runtime receives messages. Thread operations use the same routing and availability as
conversation operations when an agent moves. They do not require the old runtime or introduce
a separate writer service. If the conversation path cannot admit an operation, report its
actual availability or refusal.

Use the daemon's typed receipt to show an accepted action. Local acceptance confirms the
local commit, not admission at another member or delivery to the agent. If the response is
lost, retry the same request key. A member that has the receipt returns it. Partitioned
members may accept separately before exchange. Matching operations reconcile on exchange and
conflicting key reuse produces the existing typed conflict. Surface that outcome and refresh
the affected view rather than presenting the earlier local receipt as fleet-wide proof.

A revision check fences stale changes against the admitting member's known state. Concurrent
changes accepted on different members reconcile by a deterministic claim projection. Close
prevents outgoing replies on a member that knows the thread is closed. An independently
admitted reply and close may both stand after exchange. Preserve accepted replies in history.
The operation does not impose an atomic order across disconnected members. The
[existing replication rules](replication.md) define the underlying
convergence and retry behavior.

## Cross-surface contract

| Surface | Conformance at release |
| --- | --- |
| TUI, web and iOS | Start inline in Inbox, type and reply to a message, show a fresh linked ask with full discussion, and answer that ask independently. Use the same thread in Inbox, agent session, mission and a dedicated tab or screen. Compact embeds start collapsed. Web and terminal can reveal Reply on hover with accessible keyboard equivalents. Mobile supplies an equivalent touch action. |
| TUI, web and iOS | Rename, close, list under Recently closed threads, read replies while closed, reopen, and retain selection and draft destination across navigation. State and unread progress agree when another authorized client changes them. |
| MCP | The current conversation read grant permits bounded discovery, summary and timeline reads for all threads in every conversation that grant can already read, within its existing owner and visibility scope. Each request selects one exact conversation. Revalidate grant and visibility on every request, page and cursor. Reuse the daemon's thread projection. A context link does not expand the grant. |
| MCP writes | Production writes remain gated on a separate scope decision, the [keyed issuer prerequisite](https://github.com/compoundingtech/fractal/pull/3324) and the [assistant access review](https://github.com/compoundingtech/fractal/issues/3326). Future write capabilities must preserve existing authority and assistant attribution through typed daemon actions. |

Implement and review behind capability negotiation. TUI, web, iOS and MCP reads share one
release acceptance and the same thread protocol. Production MCP writes have their own
authorization review.
Existing conversation read grants apply to existing and future threads in all conversations
they already authorize, including fleet-wide access where the existing grant has that breadth.
Thread reads add no grant or scope and convey no write authority.

The minimum integrated fixture starts on ask A, creates a thread, quotes a message, renames
and closes it, receives an ordinary reply, and links a fresh ask B. Inspect the same history
through each context and another authorized surface. Verify that B needs its own answer, the
thread stays closed until reopened, and a same-title thread and an unrelated transcript
message never appear in its history. Run with persisted state across daemon restart and verify
incremental updates, bounded pagination, a revoked grant during a write, a forbidden
cross-thread quote, and a duplicated request key. Verify that rename, close and reopen appear
once with the correct actor and time, without affecting message or unread counts. Verify the
previous title against the observed claim on rename, reject a stale title revision,
and retain both titles when rendering after replication and restart. Across two
members, verify the same-key retry after exchange and
show that local acceptance, peer observation and agent delivery are distinct. Verify that an
agent moved using the existing placement mechanism can continue the thread when the prior
host is offline, within ordinary conversation availability and without claiming unreplicated
receipts were recovered. For MCP, verify that a granted conversation exposes all its threads without
including threads from another conversation in the same page. Reject cross-conversation cursor
reuse and refuse a
continued read after grant revocation.

## Decisions before wire freeze

Select exact operation names, claim schemas,
subscription shapes, and cursor format only when the fixture runs through the existing
client boundary. The [original topic projection discussion](https://github.com/compoundingtech/fractal/issues/49)
and [normalized conversation contract](conversation-normalization-design.md) provide context.
