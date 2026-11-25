# Fractal topic threads

Status: implemented contract. Thread clients require the current daemon thread resource and
exact summary feed; upgrade older endpoints before using threads. Stored history and existing
device pairings are preserved across that upgrade.

Fractal stores addressed messages and their `in_reply_to` edges. Earlier Inbox discussions
were found by message subject. Durable threads replace that subject-based identity: a fresh
request with a different title can refer to the same discussion while retaining its own answer
and decision history. Existing subject-based messages remain ordinary conversation history.

## Identity and history

A thread has an opaque durable identity in the `thread/` family. Its editable title, current
state, conversation, message history and related items are read through that identity. A
new thread started from an Inbox item can commit its creation, origin link and first message
together. A client may hold an unsent draft without publishing an empty thread.

The typed first-message writer uses `append_claim_with_commit_context_tx` inside the
thread-creation transaction, with the creation's batch ID. This sanctioned seam shares
the ordinary commit-context preflight and serialized admission, including recipient
retirement checks, without nesting a writer lock. The secondary message keeps its
`thread-start-message:` operation body identity and signature expectation in that same
transaction; a refusal or caller rollback leaves neither thread nor message. Ordinary
untyped appends still cannot set thread relation fields.

The thread ID is independent of the first message ID. Anchoring it to the first message could
reuse reply-chain lookup, but would couple the discussion's identity and availability to that
message's delivery, visibility and retention. A separate identity makes rename, reopening and
reference from other resources explicit. The cost is a durable thread projection and admission
rules for mutations and links.

Every message still has its own message ID and ordinary sender attribution. Its thread
reference is explicit. A reply to a specific message records a thread quote whose parent belongs
to the same thread. Ordinary `in_reply_to` retains its request settlement behavior. The reply can
appear in the agent conversation and in the thread as one message. Clients use authoritative
order, current conversation visibility and stable cursors.
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

The typed daemon start/send boundary also accepts retained episodes after resolution.
One shared, privately constructed `QualifiedDiscussionSource` proves an existing canonical
opening, its original recipient, and the current recipient. Supported retained sources are
runtime asks (`work.person-asked`), manifest-defined custom items (creation claim and
`recipient_field`), human gate reviews (`gate.requested`), and reserved notices with the
pinned notice creation, registration and owner chain. The notice episode is its source
creation claim ID, not a snapshot ID. Labels, `about` links and arbitrary
`custom.attention.*` kinds confer no authority. Reassignment removes retained discussion
rights. Enrollment, browser/device admission, message scope, rollout and responsible-agent
checks still apply; a retained source never grants an otherwise refused write.
Current and retained starts preserve the original source agent unless native authoritative
`agent.retired` lifecycle evidence proves terminal retirement/removal. Only that terminal
proof selects the configured `inbox_fallback_agent`; offline or unknown runtime, a missing
declaration, reversible stop and other admission refusals never select a replacement.
There is no inferred role or default agent. Existing threads retain their frozen audience;
they are never retargeted and the retired agent is never revived.

Before first send, read the current Inbox row's optional `start_recipient`, or request
`GET /v1/client/threads/start-context?source_id=...&episode=...` for an exact opening key
already known from an Inbox card, a bounded existing history read or source context.
This is not a retained-source discovery list. Successful preview returns that same
`start_recipient` union: `original`, `configured` for a source without an original agent,
or `fallback` with the original agent and authoritative terminal retirement claim reference.
On current rows, `conversation_id` equals its `agent_id`. Denied/missing openings share
the unknown-source 404; an unconfigured fallback after terminal retirement returns the
already-declared `recipient-retired` conflict. Source and pairing authority are rechecked.
The Start request still carries its displayed `agent_id` and optional signature; a recipient
mismatch refuses without rewriting the signed packet. Refresh the preview before confirming
a changed recipient. The scoped MCP thread-read family mirrors this metadata-only preview
under its existing exact conversation binding; MCP remains read-only.

Authored human steps require an exact retained READY opening, a hash-verified
generation-pinned mission revision, and the immutable `mission-run.created` inputs,
requester, root/parent/workspace and initial default selector. The opening recipient is
reconstructed with the native selector materialization and canonically ordered assignment
mutations before the opening; today's assignee or generation is never opening proof.
Native recipient-bearing `work.released` and `work.handoff-returned` READY receipts
also open exact episodes; no synthetic second `step-run.state` receipt is required.
The shared proof passes bounded immutable materialization variables to the missions
reader, which interpolates historical title/goals with the existing template engine.
An unchanged title keeps its generation-creation attempt; retry goals are already
materialized and are never interpolated again.
Carried openings recover their epoch from the canonically prior actual READY write,
following the exact source-generation chain and matching attempts. Literal/inherited and
retained-input selectors qualify; missing admission/definition/interpolation/READY facts,
unrecoverable carried epochs or a different current recipient fail closed.
Before hydrating opening/source/manifest bodies, metadata-only admission bounds each
proof's complete read set (including repaired history) to 512 entries and 256 KiB.
Oversized/unprovable proofs are unavailable, never exempted from the bound. Notice
disposition reads select an exact canonical latest claim, not complete snapshot history.
Harness prompts, seat login, planning, revision proposals and operational faults have no
supported retained proof. An unknown, fabricated or unsupported retained episode receives
the same unavailable error. Phase A history uses this same qualifier, so a syntax-valid
but never-opened authored key exposes no historical row content.

Start never coalesces by origin, for current or retained sources. Every new operation
key creates an independent thread and persists that request's first message, even if
another discussion already links the exact source episode and recipient. Same-key
replay recovers its original result; changed intent under that key conflicts. Recovery
keeps its existing current-principal checks and works after source changes or retirement.
A fallback Start creates its own discussion without retargeting existing participants.
Open/list is navigation, not Start deduplication. Starting or replying to a retained
discussion does not reactivate its resolved Inbox source or settle its original work.

Unread Inbox-position lookup batch-qualifies the selected exact episodes through that
same source qualifier before inspecting owned thread replies. Opening/current-policy
discovery is one keyed SQL lookup for the full input, with indexed batched notice snapshot
discovery. Reserved notices additionally reuse the native per-notice keyed
source/registration/member-chain security validation; there is no per-row source
qualifier call or live Inbox refold for supported retained sources. Live-only families
keep their existing current-source qualification. This separate unread integration is
owned by the shared writer, under FPL's backend contract.

The native CLI convenience `fractal alerts discuss ITEM --body TEXT --as PERSON`
starts the same durable thread through `thread.start`. It uses the exact current source and
episode and the daemon-assigned responsible agent. It offers no recipient override. Use
`--idempotency-key KEY` to deduplicate a start while the source episode, title and responsible
agent remain unchanged. This stateless convenience rereads the current Inbox source; it does
not retain the original action packet. A resolved source is refused locally, and a changed
source or recipient with the same explicit key conflicts instead of creating another thread.
Inspect retained history by its durable thread ID in a current thread client. Without an
explicit key, the CLI uses its ordinary content-and-hour deduplication policy; an explicit key
keeps retry identity stable across that hour boundary. Clients that retain the original exact
action packet can use the daemon's receipt recovery after the source resolves.

A bound agent can create an ask with `fractal work ask`, then run
`fractal work link-thread ASK --thread THREAD --episode CLAIM --as "$ST_AGENT"`.
The `work.person-asked` claim ID is available in `fractal --json trace show ASK`. Repeating
the same link returns the original operation receipt. An attachment failure leaves the
ask available to retry with its exact episode.

The initial lifecycle is Open, Close and Reopen. Closing moves a thread into Recently closed
threads, leaves history readable and keeps any linked ask answerable. Ordinary replies from
the person or agent join closed history without reopening the thread, adding an alert or
resurrecting the Inbox row. Reopen is an explicit lifecycle action, independent of replying.
A fresh linked ask can still show the closed thread. An authorized rename changes the shared
title everywhere and
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

Mark shown messages read explicitly. The client submits up to 50 exact rendered message IDs
and the newest of those IDs as an anchor. Fetching or preloading a page alone does not mark it
read. The daemon verifies the authenticated person's ownership and every selected message before
writing ordinary replicated read facts for incoming replies. A late arrival stays unread even if
it sorts before the anchor. A retry of the same selection returns the same acknowledged incoming
IDs without duplicating facts. Empty selections acknowledge nothing.

Read progress also works in closed threads and requires the same message-control scope as
ordinary read actions. It does not reopen a thread, resolve an ask, or change the alert count.

## Daemon boundary

Inbox discussion responsibility is separate from the item's publisher and its human approval
recipient. Resolve the originating/assigned agent first, preserving historical associations even
when that agent has stopped. For a system or person-owned source with no such agent, configure
`inbox_fallback_agent = "agent/your-existing-agent"` in the daemon's TOML configuration and
restart the daemon. Home Manager exposes the same setting as
`services.smalltalk.inboxFallbackAgent`. This is an explicit agent identity, never an inferred company role or the
fault handler. Missing configuration and undeclared fallback agents appear in the
`inbox-responsibility` doctor diagnostic. A pending or conflicting identity retains its recorded
association and adds a discussion diagnostic in the resource's existing `operational.reasons`;
the card's person-only actions stay actionable. New discussion admission still refuses the
unavailable identity. Database failures remain errors. Setup and person-only security approvals remain
available before an agent is configured; creating a discussion requires an admitted recipient.
The fallback is daemon-local routing policy, not replicated history. Changing it updates current
Inbox associations and invalidates their derived caches. A thread freezes its responsible agent
when created: configuration changes do not retarget existing threads, change their audience or
rewrite their history. Discussing a security request does not authorize or settle its approval.

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
revision. Metadata successors override their causal ancestors even if the writer's clock is
behind. Canonical claim order resolves only concurrent edits. The opaque metadata revision
fences every known causal frontier branch and the effective creation, title, and lifecycle
claims. Excluding a previously effective claim by repair changes that version even when the
frontier itself is unchanged. The next accepted edit retains all frontier branches as predecessors. A title's revision still identifies the exact effective title claim.
Existing historical subject-based discussion remains readable through its existing
path. There is no inferred migration from subjects to durable thread IDs.

### Fleet behavior and receipts

Reuse existing Fractal conversation routing, claim replication, authorization and request keys.
Thread identity follows the stable conversation while agent placement determines where its
runtime receives messages. Thread operations use the same routing and availability as
conversation operations when an agent moves. They do not require the old runtime or introduce
a separate writer service. If the conversation path cannot admit an operation, report its
actual availability or refusal.

Lifecycle action receipts expose the immutable acknowledged operation claim as `metadata_claim_id`.
It is not a CAS revision. Obtain the next metadata fence from the rendered `ThreadSummary.revision`;
receipt recovery must not manufacture a fresh version for metadata the client has not observed.

Use the daemon's typed receipt to show an accepted action. Local acceptance confirms the
local commit, not admission at another member or delivery to the agent. If the response is
lost, retry the same request key. A member that has the receipt returns it. Partitioned
members may accept separately before exchange. Matching operations reconcile on exchange and
conflicting key reuse produces the existing typed conflict. Surface that outcome and refresh
the affected view rather than presenting the earlier local receipt as fleet-wide proof.

A revision check fences stale changes against the admitting member's known state. Concurrent
changes accepted on different members reconcile by a deterministic claim projection. Ordinary replies remain in a closed thread without reopening it. An independently
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

## Initial rollout

Release note: client capability `threads-v1` version 3 advertises the unread-reminder
contract (mark/clear actions, around-anchor timeline reads, and marked-unread discovery).

Install the privacy reader release on all serving fleet members before enabling typed writes.
The writer verifies the configured and active readers' privacy capability inside its transaction.
Reader readiness version 3 includes causal metadata projection and its frontier revision fence;
version 2 protects message content but cannot enable current writes. The signed message envelope
marker remains `thread_reader_version: 2`: this rollout changes metadata semantics, not that format.
A legacy member that receives a signed thread-bearing message retains the envelope in quarantine.
After upgrade, normal startup retries admission and restores the original timeline entry once.
Fractal application stores advance published main's default storage version 20 to the privacy
floor 23. Main's versions 21 and 22 retain their signature-reference and dependency-reader
meanings; the shared graph store still defaults to 20 when no privacy application is installed.
Private reader-pilot stores at versions 21 and 22 use the same normal application upgrade path.
Version 23 keeps all older max-22 binaries, including timestamp-only privacy pilots, from
reopening this upgraded store.
The privacy floor also activates main's existing compact-signature writer and migration:
every reader allowed to open version 23 includes the version-21/22 format readers, and the
downgrade fence excludes older readers. The normal open path preserves claim authority,
history, pairings, and operation receipts while those supported physical representations
migrate. Do not lower `user_version` to run an old binary against this state.

That local floor does not protect a fresh old network member. Before activating writes, directly
verify the current build and reader-readiness v3 on every serving/configured member; historical
startup advertisements alone are insufficient deployment evidence. Keep old fresh rejoins out
of the serving fleet. The transaction's existing peer-readiness gate remains fail-closed when
a known configured/active member advertises an older reader.
