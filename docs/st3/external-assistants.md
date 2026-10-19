# External assistants

Status: assistant pairing, grant management, bounded reads, and explicitly scoped native writes are implemented. This document does not authorize live grant issuance or adapter deployment. The standalone ChatGPT adapter uses caller-bound grants; the original proof-gated read mode remains available. Coordinated kill requirements below remain a separate activation gate.

The normative wire contract is [`client-v0.schema.json`](client-v0/schemas/client-v0.schema.json), [`operations.json`](client-v0/schemas/operations.json), and the [fixtures](client-v0/fixtures). Regenerate Rust, Swift, and TypeScript clients with `cargo run -p st3-client-codegen`; do not edit generated clients. Existing person pairing retains its wire shape and historical person claims. Assistant capability discovery must fail closed on older servers, never fall back to a person credential or broad limited person grant.

## Adapter call diagnostics

`st-chatgpt` emits one structured `tracing` completion event per dispatched tool
call, including local validation failures and cancellation when its future is
dropped. Capture is off by default. Operators may opt in with
`RUST_LOG=st_chatgpt::call=info` on the adapter process; records go to stderr,
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

The intended composition is external reasoning service → MCP tunnel/shim → SCG approval/execution → st assistant grant → receiving agent. SCG verifies the caller's signing identity; st separately verifies its assistant grant and authorization proofs. A request-body actor, bare approval identifier, ordinary tunnel signature, or person-device signature is not an assistant authorization proof.

The SCG host owns the st bearer and assistant key. A separate pinned approval-attestor key authorizes read/send proofs. Neither st bearers nor person keys, attestor secrets, raw socket access, or private profiles may enter external model context, tool responses, plugin configuration, or process arguments. The shim owns only its request-signing and tunnel credentials. Pairing is an explicit operator operation, not adapter startup behavior.

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

Use native snapshot/page envelope semantics and typed generated clients, never CLI output, KDL, arbitrary claim parsing, or prose scraping. Assistant read scopes are closed and unrelated person scopes are not accepted:
The attention, missions, and agents pages retain the optional native `sync` notice during replication catch-up or divergence. Its closed `SyncNotice` contract is shared with ordinary pages and decoded by the generated SDKs.

| GET route under `/v1/client/assistant/` | Scope | Allowed fields, in addition to snapshot/page metadata |
| --- | --- | --- |
| `summary` | `read.assistant.summary` | Counts of the configured owner's attention, agent states, mission states; `as_of` |
| `attention` | `read.assistant.attention` | ID, attention-kind enum, person fixed to the configured owner, agent/mission references, revision/episode, waiting-since; no title, body, options |
| `missions` | `read.assistant.missions` | Mission/run/agent IDs, phase, step counts, blocked count, revision, updated-at; no authored goal or status prose |
| `agents` | `read.assistant.agents` | Agent ID, lifecycle/runtime-state enums, current mission/run IDs, updated-at; no paths, environment, conversation previews |
| `conversations`, `conversation-detail`, `conversation-timeline`, `conversation-search` | `read.assistant.conversation` | Bounded conversation metadata, timeline entries, and search results through dedicated assistant projections |
| `mission-detail` | `read.assistant.missions` | Allowlisted mission operational detail |
| `work`, `work-detail` | `read.assistant.work` | Bounded work metadata and allowlisted work operational detail |
| `agent-detail`, `agent-queue` | `read.assistant.agents` | Allowlisted agent operational detail and bounded queue metadata |
| `conversation-content` | `read.assistant.conversation` | Exact agent/session/reference/range, bounded text, revision, returned bytes, truncation/next-range |
| `terminal-content` | `read.assistant.terminal` | Exact agent/terminal/incarnation/screen revision, bounded text, returned bytes, truncation |

The baseline grant contains only the four minimal scopes: `read.assistant.summary`, `read.assistant.attention`, `read.assistant.missions`, and `read.assistant.agents`. The configured owning person may explicitly add conversation, work, and terminal scopes; registering a tool does not expand an existing grant. `approval-once` grants require a proof for each read; explicitly sealed `caller-bound-allow` grants admit reads within the exact grant scopes, expiry, consent, caller binding, and response budget. The adapter's rich projections remain bounded and do not grant unrelated person authority. New detail projections disclose allowlisted operational fields only: no environment, secrets, workspace paths, or terminal content. The exact-range `conversation-content` contract remains unchanged.

Disclosure filtering is structural, not word-based redaction of authorized conversation prose. Conversation search snippets resolve the matched entry's safe plain native text, never raw tool payloads; workspace, declaration, and tool metadata remain excluded. Authorized conversation prose may itself mention paths or other sensitive information. The conversation scope and bounded reads govern that disclosure; they do not promise to scrub path-like strings from the text.

`ScgReadAuthorization` must bind verified issuer/key and identity `chatgpt`, assistant principal, grant and policy revision, approval/execution IDs, exact projection/filter/cursor or content resource/revision/range, response-byte maximum, request digest, expiry, and a one-use nonce. st verifies a separately pinned attestor key and consumes the authorization atomically. A new page or range needs a new approval. Exact retries may recover the same bounded receipt inside the protected executor, but cached/status/private-result re-delivery to the external model requires fresh authorization.

Pagination binds the authenticated issuing grant ID and effective policy revision, exact projection class, snapshot and cache. Metadata and rich pages use distinct `assistant.<route>` classes, further namespaced by an opaque grant-policy digest. Queue handles also bind the selected agent. Timeline handles and the assistant-only search cache/cursors carry the same grant-policy binding; native non-assistant paging is unchanged. A cursor cannot continue under another grant, after any policy revision change, or through another projection. Revoking a grant does not transfer its handles to another active grant. Fresh authorization never authorizes release of another grant's cached rows. Missions use the native keyset and page-sized card projection; agents use a published roster slice or cached shallow refs with page-sized card builds; attention uses an owner-indexed published slice. Cursors do not cache fully built assistant collections. A missing initial publication is not-ready; a replaced publication or expired cursor requires restarting pagination. Summary counts grouped mission phases and published attention/agent state inputs instead of building collection cards.

Native mission membership seeks distinct mission IDs and their latest/live headers through additive SQLite indexes rather than ranking all historical runs. Sorting still visits shallow mission keys; only the selected page builds cards. Existing stores install the seek indexes on open, and older binaries can safely ignore them.

Assistant identifier disclosure remains restricted to nonempty ASCII tokens of at most 256 bytes, using alphanumerics and `/._:-@+=~`. Canonical source IDs outside this allowlist remain omitted, not exposed through rows or encoded continuation keys. Mission and agent membership applies this policy before keyset/offset pagination and byte packing, so filtered source rows cannot shift a delivered cursor or create an empty continuation loop. Grouped mission summary counts share the exact SQL identifier predicate with mission pages, without constructing cards. Native collection membership is unchanged.

Mission cursors bind the store claim cut and inspect at most 513 intervening claim headers. Changed mission inputs or more than 512 intervening claims return `page-cursor-expired`, since an updated mission can move across the mutable sort key; unrelated read-authorization receipts do not themselves invalidate a continuation. Clients restart rather than silently losing that mission. Mission metadata selects the latest live run, falling back to the latest terminal run, independently of the native three-run preview. Summary agent-state counts apply the same scalar delivery-presence assessment as agent pages; rich attention and exact-alert content retain update-type request payloads.

Mission claim invalidation uses the canonical subject families from `st3_schema::registry()`, not substring matching; the projection fence below can additionally expire a cursor:

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

Assistant read responses default to 4 KiB and have a hard maximum of 16 KiB UTF-8 including envelope overhead; configuration may lower these limits. Content reads do not stream, follow attachments, or silently fetch entire bodies. Over-budget requests fail rather than automatically paginate.

Assistant principals are denied every route and stream outside the assistant surface, including generic messages, timelines, search, declarations, documents, history, events, and terminal WebSockets. Ordinary `observations.report`, `prompt.respond`, and message lifecycle mutations remain denied. The explicitly scoped native writes described above use the assistant proposal route; native/legacy alternate routes cannot bypass their authority checks.

B provides typed projection/content/proof schemas, operations with exact scope/budget metadata, and advertised assistant response ceilings. Acceptance must cover unrelated-route/mutation denial, field-only projection despite secrets/injected text in source bodies, absent/wrong/expired/reused authorization, stale revisions, changed ranges or grants, Unicode/envelope ceilings, and no private bytes in unapproved, denied, expired, foreign-handle, cached, or replayed status results.

### Read wire contract

Query parameters are scalar strings; duplicates, unknown names, and any `person` selector are rejected with `validation-failed`. The person is always the grant issuer, the daemon-configured owner.

| Route | Query | Response value |
| --- | --- | --- |
| `summary` | `max_response_bytes` | `AssistantSummary`: `kind`, `as_of`, `counts` only |
| `attention`, `missions`, `agents` | `cursor`, `limit`, `max_response_bytes` | `AssistantAttentionPage`, `AssistantMissionPage`, `AssistantAgentPage`: the native page shape with collection `assistant.<route>` and empty `filters` |
| `conversations`, `work` | `cursor`, `limit`, `max_response_bytes` | Existing assistant envelope with an `items` / `next_cursor` page |
| `conversation-detail`, `agent-detail` | Required `agent_id`; optional `max_response_bytes` | Existing assistant envelope with allowlisted operational detail |
| `conversation-timeline` | Required `agent_id`; optional `session_id`, `cursor`, `limit`, `max_response_bytes` | Existing assistant envelope with an `items` / `next_cursor` page |
| `conversation-search` | Required `text`; optional `agent_id`, `cursor`, `limit`, `max_response_bytes` | Existing assistant envelope with an `items` / `next_cursor` page |
| `mission-detail` | Required `mission_id`; optional `max_response_bytes` | Existing assistant envelope with allowlisted operational detail |
| `work-detail` | Required `work_id`; optional `max_response_bytes` | Existing assistant envelope with allowlisted operational detail |
| `agent-queue` | Required `agent_id`; optional `cursor`, `limit`, `max_response_bytes` | Existing assistant envelope with an `items` / `next_cursor` page |
| `conversation-content` | `agent_id`, `session_id`, `reference`, `revision`, `start`, `end`, `max_response_bytes` | `AssistantContentPage` with `kind: "assistant-conversation-content"` |
| `terminal-content` | `agent_id`, `terminal_id`, `incarnation_id`, `screen_revision`, `max_response_bytes` | `AssistantContentPage` with `kind: "assistant-terminal-content"` |

The ChatGPT adapter exposes `summary` as `st_status_summary`; the new projections map respectively to `st_conversations_list`, `st_conversation_detail`, `st_conversation_timeline`, `st_conversation_search`, `st_mission_detail`, `st_work_list`, `st_work_detail`, `st_agent_detail`, and `st_agent_queue`. Each tool publishes a closed input schema and is read-only. All accept optional integer `max_response_bytes` from 1 through 16384; paginated tools accept optional positive integer `limit` and opaque string `cursor`. Results preserve the daemon envelope, including pagination; the adapter never follows a cursor automatically. Its standalone caller-bound mode holds only a scoped bearer, not a proof-signing key.

The following proof-header contract applies to `approval-once` grants only; the standalone adapter's `caller-bound-allow` mode instead uses the scoped bearer and sealed disclosure consent described above.

Every read carries `x-st3-scg-read-authorization`: unpadded base64url JSON `ScgReadAuthorization { payload, signature }`. The closed payload contains `issuer`, `identity: "chatgpt"`, `assistant_id: "assistant/chatgpt"`, `grant_id`, `policy_revision`, `key_id`, `approval_id`, `execution_id`, `projection` (the route suffix), `parameters` (every decoded query parameter, cursor and range included), `max_response_bytes`, `request_digest`, `expires_at_unix_ms`, and `nonce`. `signature` is unpadded base64url Ed25519 over the UTF-8 bytes `st3.scg-read.v1\n` followed by the canonical JSON payload: recursively sorted object keys, compact, integers only. `request_digest` is lowercase hexadecimal SHA-256 of canonical `{"method":"GET","path":<exact path>,"parameters":<decoded map>}`. st consumes the nonce atomically before returning private bytes; a replayed, expired, mismatched, or attestor-unverifiable proof is denied and discloses nothing. Generated clients never sign, retry, or widen the budget: they send exactly one request and reject a proof whose projection, parameters, or budget differ from the call.

The response budget covers the entire JSON envelope. Without a query budget, the proof must name the pinned default (4096 unless lowered); an explicit `max_response_bytes` must equal the proof value. Neither may exceed the pinned maximum (16384 unless lowered). Assistant list pages return the largest ordered prefix of whole items that fits, up to `limit`; `page.next_cursor` resumes after the last returned item in the same cached snapshot. The cursor retains the requested item limit, so page lengths may vary with item sizes. If not even one item and its envelope fit, the read fails with 413 `response-budget-exceeded` naming that item's ID. Other responses or native source records over budget also fail with 413 `response-budget-exceeded`; non-UTF-8 content fails with 422 `content-not-text`. A forged, expired, or changed conversation reference returns 410 `conversation-content-invalidated`; a conversation revision mismatch, a session that is not the agent's current session, or a terminal incarnation/screen mismatch returns 409 `stale-fence`.

### Read trust

For `approval-once` grants, every read is denied until the configured owning person pins trust for the exact grant at its authoritative origin. Explicitly sealed `caller-bound-allow` grants instead use their consent, scopes, expiry, and caller binding without a separate read-trust or policy call:

```sh
st devices --as person/operator grant read-trust GRANT --issuer person/operator \
  --key-id <key-id> --attestor-public-key <base64url-ed25519> --shim-public-key <base64url-ed25519> \
  [--default-response-bytes N] [--max-response-bytes N]
```

This calls `POST /v1/client/assistant/grants/{id}/read-trust` with `AssistantReadTrustRequest` (`api_version`, `issuer`, `key_id`, `attestor_public_key`, `shim_public_key`, optional `default_response_bytes` ≤ 4096 and `max_response_bytes` ≤ 16384). Keys are 32-byte Ed25519 public keys in unpadded base64url. The attestor key must differ from the shim key, so a shim-signed request cannot authorize a read. Pinning increments the policy revision and returns `AssistantGrant` with `read_trust`; outstanding proofs for the old revision stop verifying. Public fixtures use placeholder pins and an intentionally invalid signature, never real keys.

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
