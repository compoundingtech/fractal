# External assistants

Status: assistant pairing, grant management, bounded reads, and explicitly scoped native writes are implemented. This document does not authorize live grant issuance or adapter deployment. The standalone ChatGPT adapter uses caller-bound grants; the original proof-gated read mode remains available. Coordinated kill requirements below remain a separate activation gate.

The normative wire contract is [`client-v0.schema.json`](client-v0/schemas/client-v0.schema.json), [`operations.json`](client-v0/schemas/operations.json), and the [fixtures](client-v0/fixtures). Regenerate Rust, Swift, and TypeScript clients with `cargo run -p fractal-client-codegen`; do not edit generated clients. Existing person pairing retains its wire shape and historical person claims. Assistant capability discovery must fail closed on older servers, never fall back to a person credential or broad limited person grant.

## Adapter call diagnostics

`fractal-chatgpt` emits one structured `tracing` completion event per dispatched tool
call, including local validation failures and cancellation when its future is
dropped. Capture is off by default. Operators may opt in with
`RUST_LOG=fractal_chatgpt::call=info` on the adapter process; records go to stderr,
never the MCP stdio output or the default user answer. Only the adapter call
target is admitted, even with a broader `RUST_LOG` setting. This does not enable
daemon OTLP export; configure the daemon's existing trace export separately.

Fields are `tool` (closed published name or `unknown`), `outcome`
(`success`, `error`, `cancelled`), `code` (adapter-selected code, never a raw
daemon error), monotonic `duration_ms`, optional `daemon_request_id` (only a
`request/<UUID>` from the daemon envelope), and optional `response_bytes`
(exact buffered HTTP response body length, not MCP framing or a token count).
Duration covers validation and gateway dispatch/body decoding, not subsequent
MCP delivery. Cancellation before an envelope is decoded has no daemon ID;
it does not prove a proposed write was rolled back. Hard process termination
cannot emit a completion event.

No arguments, message/alert content, credentials, handles, raw request/response,
arbitrary error messages, or caller-chosen action IDs are logged. Byte counts
reuse the existing JSON decoder's response buffer, without a second serialization.
Daemon request IDs can join existing `st.request.id` server evidence when that
capture is enabled. Browser read/send matching and page/budget efficiency
acceptance remain separate rollout evidence; instrumentation does not authorize
new production writes or grant changes. Refs #3336, #2172, #3257.

## Trust boundaries and residual risk

The intended composition is external reasoning service → MCP tunnel/shim → SCG approval/execution → Fractal assistant grant → receiving agent. SCG verifies the caller's signing identity; Fractal separately verifies its assistant grant and authorization proofs. A request-body actor, bare approval identifier, ordinary tunnel signature, or person-device signature is not an assistant authorization proof.

The SCG host owns the Fractal bearer and assistant key. A separate pinned approval-attestor key authorizes read/send proofs. Neither Fractal bearers nor person keys, attestor secrets, raw socket access, or private profiles may enter external model context, tool responses, plugin configuration, or process arguments. The shim owns only its request-signing and tunnel credentials. Pairing is an explicit operator operation, not adapter startup behavior.

Approved reads disclose data to the external model provider. A compromised executor host can steal local secrets; an approved message can influence its recipient. Revocation cannot recall bytes already disclosed, committed messages, or started effects. Transport/client/plan support is a separate feasibility gate: unsupported writes remain unsupported, without a read-hinted workaround or a pending send created through a read tool. Voice approval must use an actually supported explicit approval interaction, not inferred spoken consent.

## A. Principal, pairing, and revocable grant

### Authority and lifecycle

- The only supported principal is `assistant/chatgpt`, with trusted class `external-assistant`. It never has person authority. Its grant-bound request audit actor is `assistant/chatgpt/session/<grant-id>`; authoritative message and answer claim actors, message senders, and answer respondents are `assistant/chatgpt`, not the owning person. Message and answer evidence retain `assistant_owner`, `assistant_session_actor`, and `assistant_grant_id` as on-behalf-of provenance, separate from person-delegation authority.
- Only the daemon's configured owning person through the trusted-local CLI over the Unix API may begin, inspect, renew, revoke, or change a grant. Missing owner configuration fails closed; remote clients, agents, assistants, and other people cannot manage grants. Deployment owner identity is external configuration, not a hardcoded public identity. Public examples and tests use the fixture owner `person/operator`.
- "Owner" authority here means same-UID trusted-local socket access: the daemon's configured owning person acts through the local Unix client, and any process already running as the daemon's UID is trusted the same way. The `/proc` ancestry check distinguishes local from remote callers; it is not a security boundary against a hostile same-UID process, and A does not claim it to be one. The host's UID isolation is the boundary.
- Administrative reads validate server-established harness and native-peer provenance, not just `x-st3-person`. Bound agents and harness peers cannot inspect grants or enumerate owner assistant devices with a forged owner header; ordinary agent projection reads remain available. Paired person devices are not trusted-local grant administrators, even over Unix.
- Begin uses the existing five-minute, single-use challenge flow. Scopes, identity, and the configured owner as issuer are sealed at begin; remote completion supplies only the challenge code and device key, never scopes, issuer, or actor selection. Assistant completion must not enroll the key in a person signing chain.
- `st devices pair --assistant` explicitly seals `minimal_read_mode: "caller-bound-allow"` with the supplied scopes, expiry, and disclosure consent in the challenge. Completion preserves that mode atomically; fresh CLI grants can read without a policy-update call. Wire callers that omit the mode retain `approval-once`. Grant management accepts either the printed `assistant-grant/<hex>` identifier or its bare hexadecimal suffix.
- `expires_in_seconds` defaults to 3600, must be positive, and cannot exceed 2592000 (30 days), the ceiling advertised by `operations.json` as `assistant_grant_max_ttl_seconds` and enforced for issuance and policy updates. Challenge expiry and grant expiry are independent. There is no automatic renewal. Policy update or renewal increments the policy revision; creation starts at revision 1.
- Policy updates check the effective expiry from the latest policy: an expired or revoked grant cannot be revived, while a grant renewed before expiry remains updatable after its original expiry.
- Each request rechecks the authoritative grant, revocation, expiry, and current policy. Revocation targets the exact grant, not a shared device actor. Restart must not revive a revoked or expired grant.
- Completion, authenticated assistant requests, renewal/policy changes, and revocation must reach the authoritative grant-issuer member. Replicated grants cannot establish current revocation/policy authority or enable cross-member proof replay.
- Versioned assistant grant/revocation claims are separate from historical person claims. Credentials are stored authoritatively as digests, never bearers. Public projections must not expose credential digests. Rebuildable projections are not sole authorization authority.
- Creation, completion, renewal, policy change, and revocation outcomes must retain issuer, actor, transport, timestamps, and claim references for audit. Logs and public CLI output must not expose bearer credentials or private keys.

### Wire shape

`POST /v1/client/pairings` accepts disjoint untagged `PersonPairingBegin` and `AssistantPairingBegin` variants. The person variant retains its existing `person_id`, optional `full_control`, and optional limited scopes. The assistant variant has `api_version`, `device_name`, `assistant_id: "assistant/chatgpt"`, explicit `scopes`, optional `expires_in_seconds`, `disclosure_consent: true`, and `consent_evidence`: a non-empty free-form string of at most 512 bytes naming the recorded owner consent (for example an `axe decision ask` reference). It has no `person_id` or `full_control`. Consent must explicitly cover disclosure of reads to OpenAI; missing or empty consent evidence denies issuance. The consent evidence is not a read approval, send approval, or permission to disclose to unrelated callers.

An assistant challenge retains `kind: "pairing-challenge"`, pairing ID, code, and challenge `expires_at`; it also returns principal, `grant_kind: "assistant"`, grant ID, grant expiry, policy revision, and `minimal_read_mode: "approval-once"`. Existing person challenges remain unchanged, apart from optional `grant_kind: "person"`.

Completion returns the normal snapshot envelope. `PairedSession` is a disjoint untagged union: existing `PersonPairedSession` retains `kind: "paired-session"`; `AssistantPairedSession` uses `kind: "assistant-paired-session"` and returns `api_version`, principal, sender class, device ID, session actor, credential, scopes, expiry, and `grant`. The credential belongs only in protected client storage. Person profiles reject assistant sessions; assistant profiles are stored separately with owner-only permissions and never install a person signing device.

Assistant clients validate that the issuer is a concrete `PersonId` returned by the selected trusted gateway completion, not that it equals a named deployment owner. They do not accept caller-selected issuers or use person grant/signing-chain proofs as assistant authority. The daemon enforces configured-owner issuance and preserves the sealed issuer.

`AssistantGrant` returns `kind: "assistant-grant"`, ID, principal, sender class, session actor, device ID, key ID, issuer as the daemon-configured owning person's concrete `PersonId`, exact scopes, RFC 3339 expiry, integer policy revision, minimal read mode, recipient allowlist, disclosure consent, consent evidence, and revoked status. The key ID is the SHA-256 hexadecimal digest of the canonical public-key string, not a credential digest. Pairing starts with an empty recipient allowlist. Read mode is either `approval-once` or explicitly sealed `caller-bound-allow`; the CLI selects the latter.

Trusted-local management routes are:

| Method | Route | Response value |
| --- | --- | --- |
| GET | `/v1/client/assistant/grants/{id}` | `AssistantGrant` |
| POST | `/v1/client/assistant/grants/{id}/revoke` | `AssistantGrant` |
| POST | `/v1/client/assistant/grants/{id}/policy` | `AssistantGrant` |

Policy requests contain `api_version`, explicit scopes, optional duration with the same default/maximum, disclosure consent, consent evidence, and `minimal_read_mode` (`approval-once` or `caller-bound-allow`). These operations use the existing envelope/error conventions and advertise `assistant.grants.v1`. Device and capabilities projections distinguish assistant metadata from a person delegator; assistant device rows must not synthesize `person_id`.

### Explicit native write scopes

Write authority is separate from read consent and must be named explicitly at pairing or policy replacement:

| Scope | Authority |
| --- | --- |
| `write.assistant.answers` | Immediately answer supported owner alerts through `work.done` |
| `write.assistant.messages` | Send a plain-text message to any existing agent, never a person |
| `write.assistant.custom-reply` | Submit a supported custom reply |

For example, add `--scope write.assistant.answers` to `st devices --as person/operator pair --assistant assistant/chatgpt`, alongside the desired read scopes, expiry, and disclosure-consent arguments. Existing read-only grants do not acquire this authority: update their policy explicitly. `read.assistant.attention` only permits reading attention, not answering it. Answers change authoritative owner work and may release waiting work, so treating a read scope or disclosure consent as write authorization would silently escalate existing grants.

With `write.assistant.answers`, supported `work.done` proposals execute immediately without creating another owner approval ask. The existing fenced, idempotent action path still binds the alert ID, episode, selected answer/option, fence, and idempotency key. Dispatch and the authoritative answer transaction retain assistant grant/policy provenance and recheck current authority; the assistant does not become a person principal.
The writer transaction restricts this authority to a supported structured `work.person-asked` request assigned to the grant's owner, with a named answer. Authored person review steps are not alert asks and remain completable only through ordinary owner authority, not this assistant scope.

The resulting `work.person-done` claim uses `assistant/chatgpt` as its actor. Work projections expose the same principal as `person_answers[].respondent`, alongside optional server-owned `sender_class`, `assistant_grant_id`, `assistant_owner`, and `assistant_session_actor`. Owner and session context do not replace the respondent and do not set `acted_for`: that field remains reserved for an agent recording a person's prior instruction with delegation proof. `st work show` displays the assistant respondent and the separate grant, owner, and session actor context.

`write.assistant.messages` is intentionally broad across existing agents. The API and message-acceptance transaction do not require exact membership in `recipient_allowlist`, even when it is empty. That field remains stored policy metadata, not a recipient restriction. Non-agent and person recipients remain refused, and the recipient must exist. Grant expiry, revocation, policy revision, explicit scope, fenced dispatch, idempotency, and external-assistant sender attribution remain enforced. A ChatGPT confirmation card is advisory, not a second daemon approval or downstream owning-person authorization.

### Acceptance

Reject missing owner configuration, remote, agent, non-owner, absent-consent, unsupported-scope, mixed person-and-assistant, zero-duration, and over-maximum issuance. Remote completion cannot escalate scopes, select an issuer, or select a person actor. Use `person/operator` as the configured fixture owner and prove another concrete person is denied. Exercise expiry, exact-grant revocation, policy revision, and restart. Verify sealed issuer preservation, no person chain enrollment, and no credentials/digests in grant projections or safe CLI output. Merging A is not permission for live issuance.

## B. Explicit proof-gated reads

Use native snapshot/page envelope semantics and typed generated clients, never CLI output, KDL, arbitrary claim parsing, or prose scraping. The six scopes are closed and unrelated person scopes are not accepted:
The attention, missions, and agents pages retain the optional native `sync` notice during replication catch-up or divergence. Its closed `SyncNotice` contract is shared with ordinary pages and decoded by the generated SDKs.

| GET route under `/v1/client/assistant/` | Scope | Allowed fields, in addition to snapshot/page metadata |
| --- | --- | --- |
| `summary` | `read.assistant.summary` | Counts of the configured owner's attention, agent states, mission states; `as_of` |
| `attention` | `read.assistant.attention` | ID, attention-kind enum, person fixed to the configured owner, agent/mission references, revision/episode, waiting-since; no title, body, options |
| `missions` | `read.assistant.missions` | Mission/run/agent IDs, phase, step counts, blocked count, revision, updated-at; no authored goal or status prose |
| `agents` | `read.assistant.agents` | Agent ID, lifecycle/runtime-state enums, current mission/run IDs, updated-at; no paths, environment, conversation previews |
| `conversation-content` | `read.assistant.conversation` | Exact agent/session/reference/range, bounded text, revision, returned bytes, truncation/next-range |
| `terminal-content` | `read.assistant.terminal` | Exact agent/terminal/incarnation/screen revision, bounded text, returned bytes, truncation |

The baseline grant contains only the first four minimal scopes: `read.assistant.summary`, `read.assistant.attention`, `read.assistant.missions`, and `read.assistant.agents`. The configured owning person may explicitly add content scopes. `approval-once` grants require a proof for each read; explicitly sealed `caller-bound-allow` grants admit reads within the exact grant scopes, expiry, consent, caller binding, and response budget. The adapter's rich projections remain bounded and do not grant unrelated person authority.

`ScgReadAuthorization` must bind verified issuer/key and identity `chatgpt`, assistant principal, grant and policy revision, approval/execution IDs, exact projection/filter/cursor or content resource/revision/range, response-byte maximum, request digest, expiry, and a one-use nonce. Fractal verifies a separately pinned attestor key and consumes the authorization atomically. A new page or range needs a new approval. Exact retries may recover the same bounded receipt inside the protected executor, but cached/status/private-result re-delivery to the external model requires fresh authorization.

Pagination binds the exact projection class, grant, policy revision, snapshot and page size: metadata pages use `assistant.<route>`, while rich pages use `assistant.<route>-rich`. A rich cursor cannot continue a metadata request, including one authorized by another grant or after a read-mode change. Missions use the native keyset and page-sized card projection; agents use a published roster slice or cached shallow refs with page-sized card builds; attention uses an owner-indexed published slice. Cursors do not cache fully built assistant collections. A missing initial publication is not-ready; a replaced publication or expired cursor requires restarting pagination. Summary counts grouped mission phases and published attention/agent state inputs instead of building collection cards.

Native mission membership seeks distinct mission IDs and their latest/live headers through additive SQLite indexes rather than ranking all historical runs. Sorting still visits shallow mission keys; only the selected page builds cards. Existing stores install the seek indexes on open, and older binaries can safely ignore them.

Assistant identifier disclosure remains restricted to nonempty ASCII tokens of at most 256 bytes, using alphanumerics and `/._:-@+=~`. Canonical source IDs outside this allowlist remain omitted, not exposed through rows or encoded continuation keys. Mission and agent membership applies this policy before keyset/offset pagination and byte packing, so filtered source rows cannot shift a delivered cursor or create an empty continuation loop. Grouped mission summary counts share the exact SQL identifier predicate with mission pages, without constructing cards. Native collection membership is unchanged.

Mission cursors bind the store claim cut and inspect at most 513 intervening claim headers. Changed mission inputs or more than 512 intervening claims return `page-cursor-expired`, since an updated mission can move across the mutable sort key; unrelated read-authorization receipts do not themselves invalidate a continuation. Clients restart rather than silently losing that mission. Mission metadata selects the latest live run, falling back to the latest terminal run, independently of the native three-run preview. Summary agent-state counts apply the same scalar delivery-presence assessment as agent pages; rich attention and exact-alert content retain update-type request payloads.

Mission claim invalidation uses the canonical subject families from `fractal_schema::registry()`, not substring matching; the projection fence below can additionally expire a cursor:

| Source input | Cursor treatment |
| --- | --- |
| Mission definitions/revisions/publication and retirement (`mission`) | Expire: changes membership, definition content, fallback phase/time or revision. |
| Run creation/state/current generation/root phase and readiness faults (`mission-run`) | Expire: changes latest/live selection, sorting, membership, phase, run identity or blocked state. |
| Step state/assignee/lease, person asks and mission products (`step-run`) | Expire conservatively: changes selected-run scalars; standalone asks can also create mission membership. |
| Generation/proposal adoption (`run-generation`, `revision-proposal`) | Expire conservatively: can replace the selected run's generation, revision and steps. |
| Canonical replacement/repair (`repair.applied`, `record.repaired`) | Expire conservatively: can replace earlier mission inputs. |
| Agent runtime, harness observations and delivery presence (`agent`), host availability and unrelated custom/read receipts | The claim-header guard does not expire these by themselves: mission membership/order comes from mission definitions and runs, not current agent presence. The graph projection fence can expire conservatively when unrelated graph state changes, including agent declaration/runtime/presence changes that update graph-covered columns. Read-authorization receipts do not write graph projections. |
| Wall-clock grace/lease expiry | Keep the original cursor snapshot time, so passage of time alone cannot move membership or change lease assessment within the sequence. |

The original claim cut remains fixed across pages. The guard reads at most 513 intervening subject/kind headers, without source bodies or card construction. Exactly 512 unrelated claims remain valid when the projection fence is unchanged; the 513th conservatively expires the cursor even when unrelated. The receipt regression uses 256 production read-used/read-outcome pairs (512 claims), preserving signed continuation and cross-grant rejection.

Mission cursors also bind the graph projection health status/frontier and the existing transactional `graph_generation`, read in the same SQLite snapshot as the claim cut, keys and cards. Admitted replication claims can already be inside that cut while their mission projections are still old; projecting them changes the source marker and returns typed `page-cursor-expired` rather than skipping a mission that moved before the saved key. A claimless full replay at an unchanged frontier also expires: existing graph triggers cover inserts/deletes and covered-column updates on desired declarations, mission definitions, runs, generations, steps and proposals; replay deletes/reinserts these rows. The generation is initialized only if absent and incremented transactionally, not reset by replay. Its SQLite integer read fails closed on overflow rather than wrapping. These fences deliberately trade some unrelated graph-change expirations for bounded, snapshot-consistent continuation; they do not hash cards or scan projection history.

Content responses default to 4 KiB and have a hard maximum of 16 KiB UTF-8 including envelope overhead; configuration may lower these limits. Content reads do not stream, follow attachments, or silently fetch entire bodies. Over-budget requests fail rather than automatically paginate.

Assistant principals are denied every route and stream outside the assistant surface, including generic messages, timelines, search, declarations, documents, history, events, and terminal WebSockets. Ordinary `observations.report`, `prompt.respond`, and message lifecycle mutations remain denied. The explicitly scoped native writes described above use the assistant proposal route; native/legacy alternate routes cannot bypass their authority checks.

B adds typed projection/content/proof schemas, the six operations with exact scope/budget metadata, and advertised assistant response ceilings. Acceptance must cover unrelated-route/mutation denial, field-only projection despite secrets/injected text in source bodies, absent/wrong/expired/reused authorization, stale revisions, changed ranges or grants, Unicode/envelope ceilings, and no private bytes in unapproved, denied, expired, foreign-handle, cached, or replayed status results.

### Read wire contract

Query parameters are scalar strings; duplicates, unknown names, and any `person` selector are rejected with `validation-failed`. The person is always the grant issuer, the daemon-configured owner.

| Route | Query | Response value |
| --- | --- | --- |
| `summary` | `max_response_bytes` | `AssistantSummary`: `kind`, `as_of`, `counts` only |
| `attention`, `missions`, `agents` | `cursor`, `limit`, `max_response_bytes` | `AssistantAttentionPage`, `AssistantMissionPage`, `AssistantAgentPage`: the native page shape with collection `assistant.<route>` and empty `filters` |
| `conversation-content` | `agent_id`, `session_id`, `reference`, `revision`, `start`, `end`, `max_response_bytes` | `AssistantContentPage` with `kind: "assistant-conversation-content"` |
| `conversations` | `cursor`, `limit`, `max_response_bytes` | Allowlisted conversation metadata, current session IDs and bounded read handles |
| `conversation-detail` | `agent_id`, `max_response_bytes` | Current conversation identity and a bounded timeline preview |
| `conversation-timeline` | `agent_id`, optional `session_id`, `cursor`, `limit`, `max_response_bytes` | Allowlisted timeline items, bounded text snippets and exact content read ranges |
| `conversation-search` | `text`, optional `agent_id`, `cursor`, `limit`, `max_response_bytes` | Safe-text search hits with bounded snippets and current conversation identities |
| `terminal-content` | `agent_id`, `terminal_id`, `incarnation_id`, `screen_revision`, `max_response_bytes` | `AssistantContentPage` with `kind: "assistant-terminal-content"` |

Every read carries `x-st3-scg-read-authorization`: unpadded base64url JSON `ScgReadAuthorization { payload, signature }`. The closed payload contains `issuer`, `identity: "chatgpt"`, `assistant_id: "assistant/chatgpt"`, `grant_id`, `policy_revision`, `key_id`, `approval_id`, `execution_id`, `projection` (the route suffix), `parameters` (every decoded query parameter, cursor and range included), `max_response_bytes`, `request_digest`, `expires_at_unix_ms`, and `nonce`. `signature` is unpadded base64url Ed25519 over the UTF-8 bytes `st3.scg-read.v1\n` followed by the canonical JSON payload: recursively sorted object keys, compact, integers only. `request_digest` is lowercase hexadecimal SHA-256 of canonical `{"method":"GET","path":<exact path>,"parameters":<decoded map>}`. st consumes the nonce atomically before returning private bytes; a replayed, expired, mismatched, or attestor-unverifiable proof is denied and discloses nothing. Generated clients never sign, retry, or widen the budget: they send exactly one request and reject a proof whose projection, parameters, or budget differ from the call.

The response budget covers the entire JSON envelope. Without a query budget, the proof must name the pinned default (4096 unless lowered); an explicit `max_response_bytes` must equal the proof value. Neither may exceed the pinned maximum (16384 unless lowered). Assistant list pages return the largest ordered prefix of whole items that fits, up to `limit`; `page.next_cursor` resumes after the last returned item in the same cached snapshot. The cursor retains the requested item limit, so page lengths may vary with item sizes. If not even one item and its envelope fit, the read fails with 413 `response-budget-exceeded` naming that item's ID. Other responses or native source records over budget also fail with 413 `response-budget-exceeded`; non-UTF-8 content fails with 422 `content-not-text`. A forged, expired, or changed conversation reference returns 410 `conversation-content-invalidated`; a conversation revision mismatch, a session that is not the agent's current session, or a terminal incarnation/screen mismatch returns 409 `stale-fence`.

The MCP adapter advertises exactly eleven canonical tools:
`fractal_alerts_list`, `fractal_alert_get`, `fractal_alert_answer`,
`fractal_missions_list`, `fractal_agents_list`, `fractal_conversations_list`,
`fractal_conversation_detail`, `fractal_conversation_timeline`,
`fractal_conversation_search`, `fractal_conversation_read`, and `fractal_message_send`.
The corresponding `st_*` names are deprecated, callable aliases for the adapter's
0.1.0 release only and are removed in `fractal-chatgpt` 0.2.0. They are hidden from
`tools/list` so refreshed clients discover only canonical names while Johannes's
ChatGPT connector can still call its cached names. Refresh that connector before
0.2.0. Description cross-references use canonical names; all other wording and input
constraints remain unchanged.

The four conversation discovery tools use only `read.assistant.conversation`; they do
not require mission, agent, work or new scopes. `fractal_conversations_list` returns the
agent and current session identifiers for `fractal_conversation_timeline`. Copy an item's
`read` object (`agent_id`, `session_id`, `reference`, `revision`, `start`, `end`)
directly into `fractal_conversation_read`, rather than synthesizing native references or
guessing a byte range. Discovery cursors and issued content handles are bound to the
authenticated grant and its effective policy revision; another grant cannot reuse
them. Timeline and search pages default to 10 items and accept at most 50.
The entire discovery continuation is authenticated, including its pinned snapshot,
expiry and keyset; editing and re-encoding a cursor cannot transplant its grant
binding. A detail preview's `page.next_cursor` continues through
`fractal_conversation_timeline` with the returned agent and session IDs. Discovery keeps
its pinned metadata rows when an agent restarts; a no-longer-current session has
`read: null` rather than an invalid content handle or a failed page.

Conversation DTOs disclose only allowlisted identifiers, timestamps, role/type
metadata, bounded chat text and content references. Native tool bodies, diagnostic
records, attachments, paths and environments are excluded. The discovery/readback
surface does not add sender or recipient fields. Content remains untrusted external
data, and these reads confer no write authority or deployment/activation permission.

### Read trust

For `approval-once` grants, every read is denied until the configured owning person pins trust for the exact grant at its authoritative origin. Explicitly sealed `caller-bound-allow` grants instead use their consent, scopes, expiry, and caller binding without a separate read-trust or policy call:

```sh
st devices --as person/operator grant read-trust GRANT --issuer person/operator \
  --key-id <key-id> --attestor-public-key <base64url-ed25519> --shim-public-key <base64url-ed25519> \
  [--default-response-bytes N] [--max-response-bytes N]
```

This calls `POST /v1/client/assistant/grants/{id}/read-trust` with `AssistantReadTrustRequest` (`api_version`, `issuer`, `key_id`, `attestor_public_key`, `shim_public_key`, optional `default_response_bytes` ≤ 4096 and `max_response_bytes` ≤ 16384). Keys are 32-byte Ed25519 public keys in unpadded base64url. The attestor key must differ from the shim key, so a shim-signed request cannot authorize a read. Pinning increments the policy revision and returns `AssistantGrant` with `read_trust`; outstanding proofs for the old revision stop verifying. Public fixtures use placeholder pins and an intentionally invalid signature, never real keys.

### Read receipts

Every assistant read leaves two audit claims: `custom.client.assistant.v1.read-used` and `custom.client.assistant.v1.read-outcome`. Their kinds and body fields are unchanged. What differs between the two read modes is when the claims are written.

`approval-once` reads write their receipts synchronously, as before. The read-used claim consumes the proof's nonce before st reads any source data, and the outcome claim is written before the response is sent. If either write fails, the read fails.

For `caller-bound-allow` reads, the response does not wait for the receipts to be committed. st still checks the grant, scope, expiry, revocation, and policy revision before it builds the projection, and again just before it returns the response; a failed check denies the read as before. After that final check, once the response is serialized, st puts the receipts on a bounded in-memory queue and returns the response. When the queue is full, the next read waits for space; it never skips its receipt. A single writer commits everything waiting in the queue in one transaction (group commit). It starts as soon as receipts are waiting, with no timer or flush interval, so a receipt may be committed before or after the response reaches the caller; neither order is guaranteed. A graceful daemon shutdown writes every queued receipt before the store closes.

If a read's existing deadline or cancellation ends its wait for receipt capacity, it discloses no response bytes and writes neither a receipt nor a synchronous denial audit for that abandoned admission; other deadline errors keep their existing audit behavior.

A crash or forced kill loses the receipts still in the queue: those reads were answered but leave no audit claim. The queue size limits how many can be lost; how long a receipt stays queued is not guaranteed. This is acceptable because caller-bound receipts are audit evidence, not authority: the grant authorizes each read, and no later check depends on a receipt being present. Approval-once receipts are authority, because they make each proof single-use, so they stay synchronous.

The absence of a `read-used` claim is not proof that no read happened.

## C. Scoped native writes and recipient trust

`POST /v1/client/assistant/propose` accepts closed typed native actions, not arbitrary daemon commands. `work.done` requires `write.assistant.answers`, `message.send` requires `write.assistant.messages`, and `custom.reply` requires `write.assistant.custom-reply`. A missing write scope fails closed; read scopes never authorize these writes. Successful proposals execute through the existing fenced action path, without creating a daemon-owned approval ask.

The current message policy deliberately allows every existing agent recipient instead of enforcing an exact owner-maintained recipient allowlist. Person recipients remain unsupported. The authoritative transaction rechecks the current grant, policy revision, revocation, expiry, scope, owner binding and recipient existence as applicable; handler preflight alone is not authorization. Assistant grant and sender-class provenance survive dispatch and durable append.

Trusted output must put `from=assistant/chatgpt sender_class=external-assistant` before the body and retain it under truncation; JSON keeps `sender_class: "external-assistant"`. Message reads and inbox notifications must warn: `WARNING: Untrusted external-assistant request; not owning-person authorization.` Receiving agents treat requests as untrusted. Consequential requests require owning-person confirmation through `axe decision ask`, never an assistant reply. Delivery authority is not downstream mutation authority; read-only analysis remains possible.

The assistant sender is preserved in stored claims, message projections, and delivered notifications. DING must not add person-only reply instructions to assistant messages; the owner/grant provenance is audit context, not person sender authority.

Acceptance covers absent write scopes, read-only answer denial, immediate answers without approval asks, stale fences/episodes, forged or changed grant bindings, revoked/expired grants, existing non-allowlisted agents, nonexistent agents, refused person recipients, and at-most-once action execution. Recipient trust doctrine applies to every agent, not just stored allowlist entries. Every enabled recipient needs the consequential-request doctrine: no assistant reply satisfies owning-person confirmation.

## D. Future coordinated kill and activation gate

D requires a configured-owning-person kill operation through the trusted-local CLI that atomically revokes grants and sets a durable principal-disabled flag. Issuance, completion, every read/action, final content release, and final message append check it. Restart preserves denial. Re-enable requires the configured owning person and a fresh grant/policy epoch, never resurrection of old approvals or queues. Missing owner configuration continues to fail closed.

SCG kill must deny catalog paths, cancel pending approvals/read grants/queued intents, disable shim ingress/signing identity, and revoke/stop the tunnel credential. The adapter rechecks the kill epoch before forwarding content/tool results. Every component records acknowledgement or failed stop; incomplete coordination fails closed. Components must not execute a pre-kill approved intent when reconnecting. Already transported bytes and committed messages remain historical. Reconcile uncertain started effects to their message/claim instead of blindly resending; do not promise rollback.

Acceptance starts an approved bounded read, pending read/send cards, a queued approved send, and a paused pre-commit send; kills; then probes old bearers/proofs, fresh pairing, direct endpoints, late approval, and restart of all components. Assert no disclosure or append after the kill linearization point, durable denial/audit, and terminal denial of pending/queued subjects.

Code merge, provider consent, and ambient tunnel access do not silently authorize live activation. The coordinated kill test must pass on the exact candidate revisions before live assistant issuance; activation also requires a recorded caller-binding/disclosure policy decision and supported plan/client feasibility. Write enablement requires explicit write scopes and the native action/recipient-trust acceptance above, not recipient allowlist population or a read-hinted workaround.

## E. Remote access over OAuth

Status: specification, not implemented or authorization to expose an adapter.
OAuth is a front door to the existing assistant grant, not a separate identity
or permission system. This section builds on A–D and the caller-bound grant
contract; it does not relax their disclosure, write, or activation gates.

**Guiding principle — operator-owned public access.** Fractal operates no
centralized hosted authorization service. Early adopters configure their own
public ingress using Tailscale, Cloudflare or another provider. This avoids
project obligations to preserve a hosted service through breaking changes or
infrastructure migrations, and does not require users to entrust sensitive
hosted data to the project. Fractal supplies composable local serving and
explicit URL configuration primitives, independent of the ingress provider.
Authority: Johannes, Q45 (on3kc3), via Compounding Manager.

### Components and boundaries

```text
MCP client (ChatGPT) -- bearer --> OpenAI tunnel --> adapter /mcp
        |                                           | resource server
        | discover, authorize, exchange code        | caller bearer per request
        v                                           v
Public HTTPS hostname                         fractal daemon
  /authorize, /token, metadata --------------> single grant authority
        |
        +-- browser: pairing code + requested scopes
                         |
Owning person -- trusted CLI/stui --> typed client API approve/deny
```

The adapter is the OAuth resource server. `/mcp` remains on the OpenAI tunnel;
the separate public HTTPS hostname exposes authorization, token exchange and
discovery metadata, not the daemon client API or a second public MCP endpoint.
The authorization server runs inside the fractal daemon/adapter deployment,
not an external IdP. The daemon alone owns pending requests, consent decisions,
grant issuance, token digests, refresh eligibility and revocation. Adapter
transport handling cannot manufacture an approved grant.

Remote mode holds no long-lived adapter grant credential: it forwards each
caller's bearer to the daemon. `--credential-file` remains a stdio/loopback
mode, never a fallback when remote authentication fails. Tunnel possession,
including an API key in the same OpenAI organization, is not authorization.

### Operator-owned ingress

The daemon/adapter serves `/authorize`, `/token`,
`/.well-known/oauth-authorization-server` and
`/.well-known/oauth-protected-resource` on a local listener. The operator
configures two independent values: the public HTTPS issuer URL for that
listener's ingress, and the canonical MCP resource URI on the OpenAI tunnel.
The authorization-server metadata and authorization-response `iss` derive
only from the configured issuer; resource metadata and token audience binding
derive only from the configured resource URI. Verification compares those
configured identities, never values inferred from request `Host` or forwarded
headers. Ingress forwards traffic; it is not the grant or identity authority.

Only the authorization/token endpoints and both metadata documents are
published through this ingress. The daemon client API and `/mcp` are excluded;
the latter remains on the OpenAI tunnel. The local serving boundary, configured
issuer and configured resource URI are the same primitives for every provider.

Example recipes, subject to the activation gates below:

- **Tailscale Funnel:** enable HTTPS/Funnel for the operator's chosen node and
  publish the local authorization listener through Funnel. Configure the
  resulting public HTTPS origin as the issuer URL and the existing tunnel MCP
  URI as the resource URI. Forward the four authorization/discovery paths only;
  keep other daemon routes off that listener. No Tailscale identity replaces
  the owning-person approval step.
- **Cloudflare Tunnel:** create an operator-owned tunnel and public DNS route
  for the chosen authorization hostname. Route the four authorization/discovery
  paths to the local listener and reject unmatched paths. Configure that public
  HTTPS origin as the issuer URL and the existing tunnel MCP URI as the resource
  URI. Browser authorization and public-client token exchange must remain
  reachable without a separate interactive ingress login.

Neither provider is required; any HTTPS ingress preserving these boundaries
can use the same primitives. The owner dogfoods this composition in their
dotfiles, using operator-owned ingress configuration rather than a hosted
project service.

### Discovery and authorization-code flow

Use OAuth authorization code with PKCE `S256` (RFC 6749 and
[RFC 7636](https://www.rfc-editor.org/rfc/rfc7636)), following the
[MCP authorization profile](https://modelcontextprotocol.io/specification/2025-06-18/basic/authorization)
and [ChatGPT authentication requirements](https://developers.openai.com/apps-sdk/build/auth).
The OAuth 2.1 profile excludes implicit and password grants.

| Surface | Endpoint / behavior | Standard |
| --- | --- | --- |
| Resource server | Unauthenticated `/mcp` returns `401` with `WWW-Authenticate: Bearer resource_metadata="…"` pointing to its protected-resource metadata | [RFC 6750](https://www.rfc-editor.org/rfc/rfc6750), [RFC 9728](https://www.rfc-editor.org/rfc/rfc9728) |
| Resource metadata | `/.well-known/oauth-protected-resource` describes this MCP resource and its authorization server; published metadata is reachable through the tunnel resource and public metadata surface | RFC 9728 |
| Authorization-server metadata | Public `/.well-known/oauth-authorization-server` publishes the issuer, `/authorize`, `/token`, supported scopes and `S256` | [RFC 8414](https://www.rfc-editor.org/rfc/rfc8414) |
| Authorization | Public `/authorize` validates the allowlisted client, redirect URI, scopes, `resource`, state and PKCE challenge, then creates a pending daemon request | RFC 6749, RFC 7636, [RFC 8707](https://www.rfc-editor.org/rfc/rfc8707) |
| Authorization response | After owner approval, redirect to the validated client URI with a short-lived, single-use code, client state and the authorization-server `iss` | [RFC 9207](https://www.rfc-editor.org/rfc/rfc9207) |
| Token exchange / refresh | Public `/token` binds code redemption to the client, redirect URI, resource and PKCE verifier; refresh remains bound to the same grant | RFC 6749, RFC 7636, RFC 8707 |

The RFC 8414 authorization-server metadata advertises this complete contract,
using the operator-configured public HTTPS issuer:

| Field | Value |
| --- | --- |
| `issuer` | The authorization server's public HTTPS issuer identifier |
| `authorization_endpoint` | Absolute public HTTPS URL for `/authorize` |
| `token_endpoint` | Absolute public HTTPS URL for `/token` |
| `code_challenge_methods_supported` | `["S256"]` |
| `client_id_metadata_document_supported` | `true` |
| `token_endpoint_auth_methods_supported` | `["none"]` (public clients using PKCE) |
| `authorization_response_iss_parameter_supported` | `true` (RFC 9207) |
| `grant_types_supported` | `["authorization_code", "refresh_token"]` |
| `response_types_supported` | `["code"]` |
| `scopes_supported` | The supported assistant read and explicit native write scopes |

The RFC 9728 protected-resource metadata advertises:

| Field | Value |
| --- | --- |
| `resource` | This MCP resource's absolute HTTPS identity on the OpenAI tunnel |
| `authorization_servers` | Array containing the authorization server's issuer identifier |
| `scopes_supported` | The assistant scopes accepted by this MCP resource |
| `bearer_methods_supported` | `["header"]` (`Authorization: Bearer`, not query/body credentials) |

```text
1. Client -> /mcp: no bearer
   /mcp -> Client: 401 + protected-resource metadata location
2. Client -> metadata: resource identity, issuer, endpoints, scopes, S256
3. Client -> /authorize: client_id, redirect_uri, scope, resource,
                        state, code_challenge, code_challenge_method=S256
4. Daemon: pending request; browser displays pairing code and requested scopes
5. Owner -> trusted surface -> client API -> daemon: approve or deny request
6. If approved: /authorize -> client redirect: code, state, iss
7. Client -> /token: code, redirect_uri, resource, code_verifier
   Daemon -> Client: opaque access bearer, expires_in, granted scope, refresh token
8. Client -> tunnel /mcp: Authorization: Bearer <access token>
   Adapter -> daemon session(): bearer and this MCP resource identity
```

Denial never issues a code or token. A pairing code identifies the pending
request, not a credential or proof of owner consent. Code redemption cannot
change its approved principal, scopes or resource.

### Surface-independent owner approval

The daemon represents authorization as a pending request, independent of the
browser, CLI or stui. Its review data binds request ID, owning person, OAuth
client identity, assistant principal, redirect URI, resource, requested scopes
and expiry. The trusted surface displays that data before consent.

The client API exposes a typed operation with the semantic signature
`decideAuthorization(request_id, decision: Approve | Deny) -> AuthorizationDecision`.
This is a specification signature, not an existing wire operation. The daemon
authenticates the caller as the configured owning person on a trusted surface,
checks the request is pending and unexpired, and atomically records its
terminal decision. An assistant, external MCP caller, public authorization
page, or possession of the pairing code cannot approve. A decision cannot
broaden the displayed request; a changed request needs new consent.

The fractal CLI and the Fractal terminal UI are the first callers, using the existing pairing
ceremony. Web and iOS clients can call the same typed operation once they meet
the same owner/trusted-surface requirements; approval logic does not live in
CLI commands, the Fractal terminal UI widgets or an OAuth callback.

### Tokens, grants and checks at use

Each authorization creates one assistant grant, as pairing does. An access
token maps to exactly that grant: it is an opaque bearer of 32 random bytes,
with only its SHA-256 digest stored by the daemon. Refresh credentials likewise
use digest-only daemon storage; neither raw token belongs in logs or model
context. Refresh does not create an independent source of authority.

Scopes are the existing `read.assistant.*` scopes and explicitly selected
native write scopes from C. The four minimal reads remain mandatory:
`read.assistant.summary`, `read.assistant.attention`, `read.assistant.missions`
and `read.assistant.agents`. The first release includes reads plus the existing
explicit write scopes, but OAuth consent neither implicitly enables writes nor
replaces the R1 freeze.

Access tokens live for at most one hour, capped by grant expiry. Refresh is
valid only until that grant expires, at most 30 days, follows the existing
renewal authority rules, and cannot revive an expired, revoked or disabled
grant or broaden its scopes. Existing grant revocation invalidates access and
refresh use immediately; token expiry is not a revocation delay.

For public clients, rotate the refresh token on every successful use, as
required by OAuth 2.1 and [RFC 9700](https://www.rfc-editor.org/rfc/rfc9700).
The daemon atomically consumes the old refresh token and issues its replacement.
Retain rotated-token digests for reuse detection: presenting an already-rotated
refresh token revokes the entire token family, meaning its grant, through the
existing grant revoke. No access or refresh credential in that family remains
usable after revocation.

Access and refresh credentials have distinct token types and distinct digest
namespaces. A refresh token is never accepted as a bearer at use; an access
token is never accepted as a credential at `/token`. Type checks precede
credential lookup, so a digest match cannot cross the boundary.
Sender-constrained tokens (DPoP) are not chosen because rotation and reuse
detection provide the public-client refresh replay protection without an
additional proof-of-possession protocol.

The daemon's `session()` checks the bearer at every use, including refresh
authorization as applicable. Alongside issuer, origin, owner/actor binding,
expiry, revocation, scopes and the stored signature verdict, it requires the
token's stored resource audience to equal this MCP resource's identity
([RFC 8707](https://www.rfc-editor.org/rfc/rfc8707)). The adapter supplies its
configured resource identity, not a caller-selected substitute. A bearer for
another resource fails closed even if its grant scopes otherwise match.
The authorization server binds the validated `resource` to this audience;
an opaque bearer does not need a readable JWT `aud` claim.

### Client registration and principal mapping

Use Client ID Metadata Documents (CIMD) with an explicit allowlist, initially
ChatGPT's metadata URL. There is no open Dynamic Client Registration (DCR)
endpoint. A client-provided URL alone is not registration or authority; its
metadata and redirect URIs must pass the allowlist policy before a pending
request can be approved.

| Allowed client | Grant principal |
| --- | --- |
| ChatGPT | `assistant/chatgpt` |
| Other explicitly admitted MCP clients | `assistant/mcp`, as specified by [#3531](https://github.com/compoundingtech/fractal/pull/3531) |

The daemon selects the principal from the admitted client, never from a
caller-supplied actor. Each grant gets the existing
`<assistant-principal>/session/<grant>` session actor. An allowlisted OAuth
client is not a person and does not inherit person authority.

### Preconditions and activation order

```text
R1 option A -> bearer check at use -> OAuth -> non-loopback exposure
```

1. Deploy and verify R1 option A: keyed issuers for all four assistant claim
   kinds ([#3326](https://github.com/compoundingtech/fractal/issues/3326),
   [#3324](https://github.com/compoundingtech/fractal/pull/3324)).
2. Enforce the bearer check at use in `session()`: stored signature verdict,
   authorized issuer/owner and the resource audience, not just a digest lookup.
3. Enable OAuth issuance against that verified grant authority.
4. Only then permit non-loopback exposure, subject also to D's activation and
   coordinated-kill acceptance gates.

The Q32 grant-write freeze remains in force until R1 is deployed and verified;
OAuth issuance writes the same begun/granted claims and cannot bypass it.
Native assistant writes remain frozen until R1 as well. This spec or its merge
does not authorize live issuance, writes or public deployment.

### Open questions

- **DQ-E2 CIMD pinning:** what exact ChatGPT metadata URL is allowlisted, and
  how are URL/content changes and redirect-URI updates pinned and reviewed?
- **DQ-E3 Source admission:** does the R1 keyed-writer requirement also apply
  under `AdmissionAuthority::Source`? Resolve the admission contract before
  treating that path as satisfying the R1 gate.
