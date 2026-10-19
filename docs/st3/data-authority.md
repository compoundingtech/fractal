# st data authority

This document classifies the claim store's SQLite tables. `every_persistent_table_has_a_projection_scope`
(`crates/st3-seats/src/store/canonical_audit.rs`) fails when a table has no class; shared projection tables
are listed in `SHARED_TABLES` there and in the production projection digests. Replicated storage tables
have their own logical authority digests; local physical fields and caches are outside those identities.

The claim log and immutable blobs are the durable graph authority.

## Large inline values

New storage work must avoid unbounded large inline SQLite objects, including variable
metadata and index keys. Use a reviewed bounded physical representation or reference
without reducing the logical input contract or rejecting valid historical data.
[Large SQLite values](large-values.md) defines the transitional source inventory,
allow-for-now owner/TODO markers, static guard coverage and cleanup plan. No numeric
large-value threshold is accepted; the guard checks source-site exceptions, not sizes.

## Retention and local observations

A claim kind of `local` retention (`st schema show KIND`) is an observation that only the node
that made it reads. The daemon records it in `local_observations` instead of the claim log. It gets
no batch, no envelope and no digest, so it never replicates and trimming it never changes what a
peer holds. Every node computes the same graph from the same admitted claims. The daemon trims
the table at startup and hourly: rows older than `[observations] retention` (default `7d`, at
least `1h`) and rows beyond `max_per_subject_kind` (default 20,000) go. The newest row of each
subject and kind stays. An older build that still writes such a kind as a claim replicates it as
before, and readers merge those claims with the local rows.

A kind of `system-local` retention is local when the system records it without an actor and a
claim when a person or agent writes it as its actor. The reconciler's own `runtime.action.*`
records, its starts, stop requests, deadlines and kills, stay on the node that runs the runtime,
where the stop fence, restart windows and adoption read them. A person's signal names its
requester and replicates with its result. A local observation may cite another local observation
of the same node as evidence; a claim can cite only claims.

A kind of `latest` retention is an observation that other nodes read only in its latest state. The
local log keeps every observation. The claim log gets an ordinary claim of the same kind only when
the state changes, so every node reads the current state from the latest claim, and a checkpoint
may drop the claims each newer one replaced. `harness.observed` replicates when any field other
than `observed_at_ms` changes. `harness.usage` replicates the first reading of each incarnation and
semantics, a compaction or model change, a reading at least five minutes after the last replicated
one while the harness works, and any reading while it does not. The newest pending reading
replicates when the harness stops working.

A node can also send every local observation to an OpenTelemetry collector. The exporter is off
unless the config names a collector:

```toml
[observations.otlp]
endpoint = "http://127.0.0.1:4318"             # OTLP/HTTP JSON base URL
headers_file = "/absolute/path/otlp-headers.toml" # optional, for example x-api-key = "..."
```

Each local observation becomes one OTLP log record in OTLP/HTTP JSON:

- the record's timestamp is the observation time, and its body holds the fields;
- its attributes are `st3.subject`, `st3.kind`, `st3.actor`, `st3.incarnation_id` and
  `st3.local_id`;
- the resource names `service.name = st` and `st3.node`; the scope is `st.observations`.

Claude and Codex response token buckets are `harness.timeline` usage observations. They retain
the turn ID, model, agent, mission run, and step on the producing node. `local_usage_responses` remembers
each counted provider response, so replay across harness incarnations does not count it again;
`local_usage_spend` keeps cumulative buckets per agent incarnation, model, account, mission run and
step, and host even after the observation log is trimmed, and `local_usage_provenance` their pricing. The node replicates
`harness.usage` response rollups by agent, incarnation, model, mission step, and host. Fleet usage
queries subtract the last rollup before a period from the last rollup inside it.

Claude event hooks submit `harness.telemetry` observations to their local daemon. They retain
the counter recorded at the shared hook's actual application point, bounded warning/error logs,
operation times and the hook exit code. No collector or SDK exporter runs in a hook. Submission
is best effort with a 250 ms daemon deadline and cannot change the hook result. The timer-driven
status line stays local-only; an undeclared application produces no invocation count. Driver
diagnostics and native session/resume binding keep their existing state and diagnostic paths.

The daemon exports the captured warnings as correlated log records, `hook_invocations_total`
as monotonic delta points on `/v1/metrics`, and one `st.hook.claude-observe` operation span on
`/v1/traces`. Metric labels remain the bounded hook/event vocabulary; subject and incarnation
identities appear only in logs and spans. Span IDs are stable when a batch is retried. Collector
configuration lives in `[observations.otlp]`; `OTEL_EXPORTER_OTLP_*` in a native seat no longer
controls an exporter in its hooks.

Fresh native seats publish activity, context, account limits and normalized timeline operations
through an st-owned observation outbox, `st-harness-events.sqlite`, in their explicit observation
directory. Matching producers commit a snapshot or timeline operation and its queued event in
one SQLite transaction with full synchronization. A pipe wake notifies the native driver;
it publishes the ordered events to `/v1/harness-events` and deletes only the acknowledged prefix.
Daemon outages, a lost acknowledgement and driver re-execution leave unacknowledged events for
replay. The source runtime and driver remain attached to historical events when a successor
publishes a backlog. Admission binds the Unix peer to its seat and checks the publishing runtime
inside the daemon's writer transaction. Source runtime, spool sequence, claim kind and usage
semantics identify an exact retry; changed input is rejected. Graph-derived claim fields are
persisted in the spool before the first HTTP attempt so retries preserve their original attribution.

Authority reads reuse up to 128 harness-spool SQLite connections across the process, with
at most 64 per thread. At either limit a thread evicts its least recently used connection;
at the global limit a thread with no cached connection reads without caching. Atomic slot
reservations bound retained connections without a global query lock, and closing a connection
releases its slot, including on thread exit. Only connections are retained: each check opens
a fresh read transaction and releases it before returning, so subsequent checks observe
committed producer writes without retaining a WAL snapshot. Reads still validate the spool
format and provider/runtime binding; replacing the spool inode invalidates its connection.

The outbox is transport recovery state, not graph authority. The daemon retains each admitted
observation under its existing local/latest/durable policy. Producer snapshots remain local
coalescing and ownership evidence; timeline producers read only the previous matching entry and
latest status instead of parsing/replacing a retained JSON log. That auxiliary timeline history
is bounded to 4,096 operations and 2 MiB, independently of unacknowledged events. An outbox with 64 MiB of pending payload
rejects a producer transaction and reports the failure rather than dropping queued events.
An evidence deadline queues derived unknown once after fifteen minutes without fresh state;
a concurrent heartbeat supersedes that deadline. Native transcripts and session bindings retain
their existing source paths.

The outbox is enabled only when a provider launches fresh. A provider that a replacement binary
adopts keeps the polled record transport until its next restart.

The exporter keeps its cursor in `meta` and moves it only after the collector accepts a batch of
at most 512 observations, including every log/metric/trace request that batch needs. Delivery is
at least once: partial acceptance can repeat logs, delta points or spans on retry. A collector that is down delays export
with backoff up to five minutes and never blocks a write. Observations trimmed before export are
counted in the daemon log.

Codex and OpenCode delivery holds are durable `delivery.hold` decisions on the seat, with
an actor, reason, and explicit expiry. They are separate from reachability, observed activity,
and the harness's own approval/compaction holds. Inspect, set, and release them with:

```sh
st agents hold agent/example/worker
st agents hold agent/example/worker --for 15m --reason "Quiet interval" --as person/alex
st agents hold agent/example/worker --release --reason "Resume delivery" --as person/alex
```

`GET /v1/delivery/hold?subject=agent/example/worker` returns the current decision and whether
it is active. `POST /v1/delivery/hold` is the dedicated operation; ordinary claim submission
cannot write it. A person may control a seat, and a seat may control itself. Setting a new hold
requires a declared Codex or OpenCode seat, the harnesses whose former DND behavior this replaces.
A release records expiry zero; a hold requires a future Unix-millisecond deadline. Decisions
replicate and survive daemon/driver restart. Expiry enables delivery without another write.

Native wrappers no longer create or refresh legacy `status` files. Their observed-record
heartbeats remain session-owned. Codex/OpenCode pumps receive graph control explicitly, start
closed, and read control on mailbox arrival, with a 250 ms request deadline and three-second permit.
They renew the permit once per second only while native mail is waiting or a predecessor's hold
needs adoption. An empty mailbox closes the gate and makes no delivery-hold requests. The next
mailbox event obtains fresh control; idle time cannot leave a permit open or delay delivery.
An unavailable, malformed, or mismatched response closes the gate; a stalled driver loop also
loses its permit. Holds block new handoffs while providers, observations, and pending receipt
reconciliation continue. Input already handed to a harness cannot be recalled.

A replacement adopting an older Codex/OpenCode provider imports a still-fresh DND once with its
original expiry. The store admits that import only when no graph hold decision exists, inside
the writer transaction, so an explicit graph release cannot be overwritten by adoption. Fresh
launches ignore historical status files.

Runtime observations and harness observations remain separate claims. The status projection puts
runtime fields in `actual` and the current incarnation's harness fields in `harness`.

All other tables are indexes, projections, local capabilities, or transport recovery state.

Shared outcomes, selected source identities and durable action fences must agree when replicas
hold the same logical claims. A daemon, eval, repair or reader filtering one origin still follows
that rule. Local transport/history cursors, cache invalidation, publication cadence and
authorization secrets do not choose shared winners.

A seat queue has no table. Each read derives the order from the `step_runs` projection and the
replicated `agent.queue.moved` claims on the agent subject, so every replica computes the same
order from the same admitted claims.

| Table | Scope | Role and recovery source |
|---|---|---|
| `account_limit_readings` | Local | Account quota readings for this host; never replicated. |
| `account_limit_seats` | Local | Local assignment of seats to account quota windows. |
| `agent_message_days` | Local cache | Rebuildable eligible-message timestamps and daily recipient counts, plus a deduplicated queue of changed messages, over this node's retained send claims. They accelerate the daily usage estimate, are not replicated, and do not add shared identities to projection digests. |
| `agent_message_sends` | Local cache | Rebuildable eligible-message timestamps and daily recipient counts, plus a deduplicated queue of changed messages, over this node's retained send claims. They accelerate the daily usage estimate, are not replicated, and do not add shared identities to projection digests. |
| `arrangement_registers` | Shared projection | Canonical arrangement register operations derived from admitted claims. |
| `arrangements` | Shared projection | Selected canonical arrangement declarations and state; changed_index is a local arrival cursor. |
| `batches` | Shared storage, with stated local fields | Shared log metadata: origin/sequence/hash/accepted time; primary ordering inputs. Storage rather than an outcome projection. |
| `blobs` | Shared content authority | Immutable bytes/hash/size carried by durable claims or admitted blob records; included in shared projection digests. |
| `capabilities` | Local | Local secret-bearing authorization tokens, expiry/use bookkeeping. |
| `checkpoint_capture_epoch` | Local | Capture invalidation epoch, monotonic envelope/time guard bounds and trigger-migration version. Store-local physical rowid frontiers and capture timing are not replicated projection outcomes and are outside shared digests. |
| `checkpoint_claims` | Shared storage, with stated local fields | Shared immutable dropped-claim/evidence/ancestry/operation identity. Digest logical live/tombstoned identity where relevant; nodes may have trimmed at different times. |
| `checkpoint_envelopes` | Shared storage, with stated local fields | Shared immutable dropped-envelope identity. Live and trimmed nodes represent the same inventory using envelopes versus tombstones, so digest their logical union rather than treating trim progress as divergence. |
| `checkpoints` | Local | Local seal/verification/trim/adoption ledger and rowid high water; nodes legitimately occupy different protocol stages. Shared protocol facts are checkpoint.* claims. |
| `claim_signature_candidates`, `claim_signature_sources` | Shared current state | Distinct normalized carried signatures per live immutable claim and their carrying writers. Retained candidates remain durable authority input even if envelope history later becomes trimmable; this change qualifies no cap, eviction or trim eligibility. |
| `claim_signature_counts`, `claim_signature_writer_counts` | Incremental inspection projections | Candidate and carrying-writer counts maintained by admission/deletion triggers. The attributable inspection threshold reports a fault without deleting candidates. |
| `claim_signature_judging` | Local incremental work | Durable candidate cursor, best candidate and dependency availability for bounded writer loans. Relevant changes invalidate its accumulator; partial candidate state cannot authorize a dependent claim. |
| `claim_signature_migration` | Local bootstrap progress | Durable inventory cursor, reconciliation targets, serving model and explicit fault. Complete isolated legacy semantics serve until atomic candidate activation; reaching an envelope does not authorize its claims under candidate semantics, since later envelopes can carry more candidates. The legacy P0 remains visible during bootstrap. |
| `claim_signature_nonce_uses` | Shared incremental projection | Cryptographically matching key/nonce use per live claim, including non-winning candidates. Invalid signature bytes cannot reserve another key's nonce; deletion invalidates affected dependencies before removing use. |
| `claim_signatures` | Shared projection | Best candidate under current authority: Verified before Held before Invalid, then normalized signature bytes. Every candidate contributes authority dependencies and matching nonce use, including non-winners. Readers pin one complete legacy or candidate model. |
| `claim_verdict_fresh`, `claim_verdict_links`, `claim_verdict_queue`, `claim_verdicts` | Local cache of shared outcomes | Current-authority candidate fold and dependency work, outside replication digests and rebuilt by `recheck_claim_verdicts`. Links accumulate conservatively per page until claim removal or explicit recheck; completing a judgment does not rewrite its whole dependency set. Property tests compare inventory, selection, verdicts and recomputation across shuffled, split and duplicate delivery. |
| `claims` | Shared storage, with stated local fields | Shared immutable logical claim bytes/id/subject/kind/actor/predecessors/time. store_index alone is local; system-local/legacy migrated claims must be excluded from logical replicated-source digests. |
| `coordination_sends` | Local cache | Writer-maintained count and held metadata, bounded historical cursor/ceiling/progress. No new conversation payload; excluded from shared projection digests. |
| `custom_dependencies` | Shared projection | Reducer dependency relationships derived from registered custom subjects. |
| `custom_registrations` | Shared projection | Canonical custom-subject registrations. |
| `custom_sources` | Shared projection | Canonical source claims for custom-subject reducers. |
| `desired` | Shared projection | Shared selected declaration, ancestry/conflicts, ownership and full canonical body; existing ancestry/revision/id selection is deterministic and must be preserved. |
| `declared_resource_edges` | Shared projection | Indexed owner/name/target/reason edges derived from selected seat and mission declarations; every logical column participates in projection digests. |
| `documents` | Shared projection | Shared immutable name/hash binding, binding_claim_id and materialized canonical binding_key. created_index is a local arrival cursor; latest selection and history order use the indexed binding_key. |
| `event_positions` | Local | Payload-free admitted event positions and subject routing; replay/admission history can differ. |
| `events` | Local view | Canonical claim payloads joined to local admitted membership by arrival index. |
| `expected_claim_signatures` | Local | A device's signature for the claim this node is about to write, moved into `claim_signatures` by the write's own transaction and gone once it returns. |
| `fleet_generation` | Local | Membership-fold invalidation counter. |
| `fleet_invite_tokens` | Local | Local invite secret/redemption attempt/authorization bookkeeping; fleet.invite-* lifecycle claims are shared. |
| `glass_heads` | Shared projection | Canonical glass declaration heads derived from admitted claims. |
| `graph_generation` | Local | Local cache invalidation counter; same graph can have different mutation counts. |
| `held_keys` | Local | Which principal each key this node holds belongs to, and its delegation chain; the private keys are files in `STATE/keys`. |
| `idempotency` | Local | Hashed caller keys and derived responses. New receipts expire after seven days in bounded rowid order; active work extends retention. Rows at or below meta.idempotency_legacy_rowid have no recorded expiry and remain. |
| `local_agent_message_pending` | Local cache | Rebuildable eligible-message timestamps and daily recipient counts, plus a deduplicated queue of changed messages, over this node's retained send claims. They accelerate the daily usage estimate, are not replicated, and do not add shared identities to projection digests. |
| `local_blob_uploads` | Local | Who uploaded an attachment file, for the per-actor quota and for who may read it before a message names it. Never replicated; the bytes are files under `<state_dir>/blobs`, see [Attachments](attachments.md). |
| `local_blobs` | Local | Staged upload bytes awaiting a durable claim reference. Promotion into `blobs` commits with the referencing claim. |
| `local_coordination_backfill` | Local cache | Writer-maintained count and held metadata, bounded historical cursor/ceiling/progress. No new conversation payload; excluded from shared projection digests. |
| `local_custom_dirty` | Local | Custom projection subjects requiring maintenance in the current transaction. |
| `local_directive_note_dirty` | Local | Person directive-note subjects whose published read needs refresh. |
| `local_directive_note_pending` | Local | Person directive-note projection work queue. |
| `local_event_payloads` | Temporary local | Present only while duplicate local event payloads drain in bounded migration transactions. |
| `local_git_change_latest` | Local | Current workspace Git observations on this node. |
| `local_git_change_records` | Local | Local Git change observation records and cursors. |
| `local_glass_head_dirty` | Local | Glass heads whose published read needs refresh. |
| `local_glass_head_pending` | Local | Glass-head projection work queue. |
| `local_idle_messages` | Local | Disposable message delivery and source-wake index, maintained by claim writes and backfilled in bounded startup batches. Excluded from replicated projection digests. |
| `local_latest_slots` | Local | Local latest-retention publication slots and pending local observation pointers. |
| `local_limit_stops` | Local | Seats this node's limits policy stopped, once per account and weekly window. |
| `local_mailbox_argv_bindings` | Local | Launch-argv binding evidence for native mailbox ownership. |
| `local_mailbox_bindings` | Local | Stable binding request tokens mapped to daemon-allocated epochs. Lost acknowledgements retry the same binding; retired tokens cannot allocate a successor. Excluded from replicated projection digests. |
| `local_mailbox_leases` | Local | Authenticated native custody for each subject/component, including exact runtime/provider/session/sequence, process birth, token/epoch and revocation. Retained across daemon restart; excluded from replicated projection digests. |
| `local_mailbox_owners` | Local | This daemon's native socket subscription owner, live runtime incarnation and replacement epoch. Survives daemon restart; excluded from replicated projection digests. |
| `local_observations` | Local | Local-retention observations and their local frontier/id; never replicated. |
| `local_ordered_membership_pending` | Local | Deduplicated lifecycle invalidations drained in the writer transaction; no independent authority. |
| `local_publication_claims`, `local_publication_starts`, `local_publication_pending`, `local_publication_batches`, `local_publication_bootstrap`, `local_publication_launch_epoch` | Local derived state | Rebuildable retained runtime and incarnation launch evidence, canonical keys, exact delta and coalesced batch queues, cache generation, and durable bootstrap cursor. Outside replicated digests; missing or incomplete evidence is Unknown. |
| `local_resource_projection_pending` | Local projection work queue | Resource subjects whose admitted observations changed in the current transaction; flushed before commit |
| `local_seat_accounts` | Local operational fact | The account chosen for each pooled seat on this node; retained across restarts, never replicated |
| `local_subscription_mission_deferrals` | Local | Local reconciler capacity backoff/retry scheduling. |
| `local_stop_deferral_backfill` | Local cache | Trigger-maintained claim ids, subjects and recovered-failure references of retained-stop diagnostics, faults and recoveries, plus the ceiling and completion of the one-time copy of older claims. Rebuildable from claims; never replicated; excluded from shared projection digests. |
| `local_usage_provenance` | Local | Pricing contributions for the local spend slots. |
| `local_usage_responses` | Local | Counted provider response IDs and times; deduplicates replay across harness incarnations. |
| `local_usage_spend` | Local | Cumulative token and cost buckets; retained across observation-log trimming. |
| `local_work_lease_renewals` | Local | Local lease extensions, re-applied after canonical replay. Durable replicated lease anchors remain shared. |
| `message_index` | Shared projection | Shared message membership/closed fact. created_index is a local cache cursor. Subject order/onset/lifecycle must come from canonical message claims, never this cursor. |
| `meta` | Local | Local schema/version/writer/cache/frontier bookkeeping; individual fleet-binding metadata copies do not turn the table into a shared projection. |
| `mission_definitions` | Shared projection | Shared selected mission revision/state/claim. |
| `mission_revisions` | Shared projection | Shared full immutable mission revision/state/body and binding claim. created_index is a local historical cursor; repeated identical revision bindings need a deterministic representative claim. |
| `mission_run_after` | Shared projection | Shared predecessor run dependency. |
| `mission_run_deadlines` | Shared projection | Shared durable run timeout/deadline derived from run creation; evaluate expiration with an explicit as_of. |
| `mission_run_requests` | Local | Local original request-hash cache paired with idempotency; run identities in the request API are origin-scoped. |
| `mission_runs` | Shared projection | Shared run creation inputs, hierarchy, ownership, workspace, requester, mode, status/phase and durable timestamps. Digest full logical row, not just status columns. |
| `operations` | Shared projection | Shared operation identity, canonical claim, request digest and conflict state, derived from live claims plus tombstones. After trim, operations with no live claim are served by tombstones. |
| `ordered_membership_counts` | Shared projection | Transactional live count; `changed_index` is a local invalidation frontier and excluded from the digest. |
| `ordered_membership_heads` | Shared projection | Raw nullable position, winning revision and complete canonical winner bytes; hidden and absent heads remain represented. |
| `ordered_membership_lifecycle` | Shared projection | Authoritative visibility and selected declaration/retirement revision for each referenced member and container. |
| `ordered_membership_live` | Shared projection | Lifecycle-visible effective `(container, bucket, key, member)` ordering and pair revision. Root is indexed as the empty bucket; member identity breaks equal-key ties. |
| `peer_cursors` | Local | Local per-peer legacy transport progress. |
| `peer_replica_cursors` | Local | Local per-peer/per-writer transport progress. |
| `person_directive_notes` | Shared projection | Current person-authored directive notes derived from claims. |
| `planning_candidates` | Shared projection | Shared immutable variant/revision/document references, mission revision and submission timestamp. |
| `planning_previews` | Shared projection | Serialized preview claim fields and candidate fence. Embedded store_index is claim payload, not this replica’s arrival cursor. |
| `planning_sessions` | Shared projection | Shared launch/planner/request/config, status, selected revision, ownership and durable timestamps. |
| `projection_digest_generation` | Local | Projection-digest cache invalidation generation. |
| `projection_digest_operation_rows` | Local | Maintained operation-row digest contributions. |
| `projection_digest_repaired_claims` | Local | Repair exclusion cache over known original claim identities. Repair meaning and replacements are covered by shared claims; raw originals remain authenticated envelope history and do not contribute to source, operation or actual-state projections. Actual values, provenance, runtime origin and runtime-epoch reads use this durable repaired-ID membership even after local receipt cleanup. |
| `projection_digest_state` | Local | Maintained shared projection digest state; the logical digest is shared, these rows are local maintenance. |
| `projection_health` | Local | Local admission/projection/quarantine status, frontier, errors and observation timestamps. Deterministic shared degraded-state summaries must be separate from receipt health. |
| `replica_envelope_holds` | Local | Local validation/fencing hold state and observation time. |
| `replica_envelope_signatures` | Shared storage, with stated local fields | Shared signed envelope identity/member-key/signature; stored_at is local receipt time. Authority/signature inventory is distinct from outcome projection digests. |
| `replica_envelopes` | Shared storage, with stated local fields | Shared authenticated writer/sequence/hash/payload identity; relay, receipt_state/errors and received_at are local. Inventory hashes immutable envelope identity, not physical rows. |
| `replica_records` | Shared storage, with stated local fields | Shared raw record/position/claim identity and replicated repair meaning; admission status/errors/updated_at are local. Normalize from retained records plus tombstones; never compare physical receipt rows. |
| `claim_positions` | Shared storage | Each claim's canonical batch position: stored at admission, lowered only by a later record of the same claim, frozen before a claim or record deletion could renumber it, and never deleted. An input of canonical order, not a projection; it is not compared or exchanged. |
| `retired_claim_ids`, `retired_envelopes` | Shared storage | Exact identity sets of every claim ID and envelope identity a checkpoint table has held; they only grow. A SHA-256 hex identity is stored as its 32 bytes, any other value as its TEXT. Not yet read by admission or evidence; never compared or exchanged. |
| `retired_claim_withdrawals`, `retired_envelope_withdrawals` | Local storage | Retired identities a checkpoint table stopped holding (adoption keeping only a manifest), with the recording checkpoint, so a later conversion can judge them under the old recorded-state rules. |
| `replication_peers` | Local | Local connectivity, peer digest observations, receive timings and errors. |
| `replication_refusals` | Local | Outbound Fabric grant refusals and observation timestamps; route policy state outside shared projections. |
| `resource_observations` | Shared projection | Latest canonical `resource.observed` claim per resource subject; indexed by resource kind and opener |
| `revision_proposals` | Shared projection | Shared candidate/source generation, reviewers/approvals, cutover/compatibility, status/preview/successor and durable timestamps. |
| `run_generations` | Shared projection | Shared generation identity, revision, predecessor, actor/reason, status and durable timestamps. |
| `step_runs` | Shared projection | Step definition, attempts, results, durable claimant/incarnation and readiness. Local lease_expires_at_unix_ms and updated_at_unix_ms overlays are excluded from projection digests. |
| `stop_deferral_claims` | Local cache | Trigger-maintained claim ids, subjects and recovered-failure references of retained-stop diagnostics, faults and recoveries, plus the ceiling and completion of the one-time copy of older claims. Rebuildable from claims; never replicated; excluded from shared projection digests. |
| `unread_mail` | Local cache | Rebuildable unread-message timestamp and prefix-count indexes, plus a deduplicated queue of changed messages, over this node's retained sent/read/closed claims. They accelerate current age counts, are not replicated, and do not add shared identities to projection digests. |
| `unread_mail_pending` | Local cache | Rebuildable unread-message timestamp and prefix-count indexes, plus a deduplicated queue of changed messages, over this node's retained sent/read/closed claims. They accelerate current age counts, are not replicated, and do not add shared identities to projection digests. |
| `unread_mail_prefixes` | Local cache | Rebuildable unread-message timestamp and prefix-count indexes, plus a deduplicated queue of changed messages, over this node's retained sent/read/closed claims. They accelerate current age counts, are not replicated, and do not add shared identities to projection digests. |

Temporary clock simulation and checkpoint proof/adoption scratch state is local.
SQLite `sqlite_sequence` is a local allocation counter. Rust read caches and exported
mailbox files are disposable local caches; their shared source selections still
follow the canonical rules above.

The store rebuilds operation and planning projections when it opens.
The resource observation projection is backfilled once on the first open with its projection
version and rebuilt from admitted claims during graph replay. Updates, out-of-order replication,
repair, and retained-claim deletion refresh only affected resource subjects. Neither the
projection nor its pending-subject queue is independent authority.

Replication receipt decodes the wire's base64 representation and stores an envelope before
admission interprets its CBOR payload. Malformed base64 remains exact TEXT evidence for admission.

Admission validates each claim and blob independently. Invalid or unknown records do not enter graph projections.

Projection uses admitted claims and keeps the last good graph when one reduction fails.

`st doctor` compares the operation projection with the claim log.

An idempotent claim stores a keyed hash of the caller key in its `_operation` metadata.

`st claim --idempotency-key KEY` opts a public claim into this contract.

The claim stores the canonical request digest and canonical claim ID.

An exact retry returns the original claim.

A retry with different input returns `idempotency-mismatch`.

Conflicting replicated operations mark the affected graph subject as indeterminate.

A `record.repaired` claim names one bad record and one valid replacement claim.

Repair keeps the original record. It changes the record state to `repaired` and records the replacement reference.

The database does not store the caller key.

Eval cleanup removes every desired projection row owned by the terminal eval run.

Eval history stays in the immutable claim log.

A cleanup residue produces `eval.verdict` with `verdict=fail`.

A cleanup infrastructure error produces `eval.verdict` with `verdict=void`.

`workspace.observed` is a latest-only replicated observation written by the owning host during
agent workspace reconciliation. It carries `host`, `workspace`, and an optional `repository`.
The reconciler writes only changed values. Repository suggestions combine those observations
with current checkout declarations; a gateway reads graph evidence and never scans host disks.

## Ordered membership projections

`ordered-membership.edited` claims on a declared container select one canonical LWW
position or retained absence per `(container, member)`. These indexed pair records are
not graph subjects and do not own their members. Their tables are listed above.

The lifecycle registry beside the schema registry declares each supported family's
visibility authority and container capability. Seats remain visible while running, stopped,
offline or suspended; only a shared stop declaration hides them, and same-ID redeclaration
restores an unchanged position. Unknown members are hidden without erasing their heads.
Lifecycle invalidation follows the shared target reverse index, with work proportional to
incoming containers. Folder deletion/lifting follows the existing arrangement resolver.
Current keyset pages seek the materialized live order, never raw heads followed by dead-member
filtering. Admission limits count live entries only; replication retains convergent overflow.
Claim history, absent winners, hidden edges and folder tombstones have no new drop rule.

Version-2 arrangements have folder-only bodies and consume ordered memberships.
Version-1 placements remain intact and are not treated as empty membership pages or
implicitly converted. Canonical-winner-preserving migration of existing placements is
separate from this primitive. The combined layout identity is
`st3.shared-projections.person-directive-notes-declared-resource-edges-ordered-membership.v1`; older layouts continue exchanging claims
but do not compare incompatible projection maps.
Concurrent v1/v2 creates preserve any unrepaired v1 creation or placement authority.
Projection caches that effective layout in an existing explicit-version register; pure
legacy layouts gain no synthetic register, and reads/admission use an indexed lookup.
Repairing a retained arrangement or membership original rebuilds both dependent projections,
including the container revision and reverse edges, so operations omitted by a replacement
cannot remain visible. Incremental projection and rebuild both exclude repaired originals.
Repairs advance local arrangement and membership invalidation watermarks stored in `meta`,
even if the last arrangement or count row disappears. Bounded pages and held windows consult
those watermarks after rebuild; they are not shared authority or digested projection fields. Repairing a referenced
retained-claim member explicitly refreshes its lifecycle selection, because the repair does
not mutate the original claims row or fire its lifecycle triggers.

## Every store_index order

Locations below refer to the audited commit, so later line-number changes do not invalidate the inventory. Shared rows must move to the canonical helper, including commutative enumerations whose returned ordering is observable. Local rows may retain arrival order only for the purpose stated.

Added operational reader `outcome_history` is a local raw terminal-claim history page, using
the serving node's claim index as its continuation cursor. It does not reduce shared state.
Its recovered reasons and `mission_overview` reasons use canonical claim ordering; a local
history cursor is not an exception for choosing a shared reason.

| store.rs line | Function / constant | Scope | Reason |
| --- | --- | --- | --- |
| 810 | `AGENT_STATUS_INDEX_QUERY` | Local | Local snapshot/cache invalidation frontier; this index names what this node has seen. |
| 2582 | `authoring_pull_request_runs_tx` | Shared | Shared association between an observed PR and authoring runs; first-seen deduplication affects returned order. |
| 2659 | `claims_page_query` | Local | Local claim-log pagination with a node-specific cursor; shared reducers must not consume this arrival-ordered page as a fold. |
| 2825 | `agent_projection_index` | Local | Local status-cache invalidation frontier; newest arrival relevant to the cache. |
| 5750 | `open_reconcile_faults` | Shared | Shared fault/recovery facts folded by subject/scope even when filtered to one origin; origin filtering does not make replicated facts local. |
| 5792 | `open_reconcile_fault_claim` | Shared | Shared selected fault/recovery claim for a subject/scope/origin. |
| 5809 | `reconcile_fault` | Shared | Shared selected fault/recovery reason used by status and attention. |
| 5933 | `mission_run_origin` | Shared | Shared creator/ownership attribution from the selected run-creation claim. |
| 7095 | `work_action` | Local | Local lease-publication cadence; quiet renewals are host-owned. Durable work state and lease anchors remain shared. |
| 9363 | `apply_operational_repair` | Shared | Shared durable repair receipt/winner and idempotent evidence; selection must be stable. |
| 10057 | `events_after_bounded` | Local | Local event-stream cursor; events are notifications about admission on this node. |
| 10077 | `events_tail_bounded` | Local | Local event-stream tail and presentation order. |
| 10078 | `events_tail_bounded` | Local | Local event-stream tail and presentation order. |
| 10123 | `projection_time_at` | Local | Local snapshot frontier timestamp. Do not use it as a shared winner or a cross-node timeless digest input. |
| 10142 | `events_after_filtered` | Local | Local event-stream cursor, including filtered notifications. |
| 10445 | `eval_runtime_records` | Shared | Shared desired/gate facts exported for evals; local test execution does not make desired selection shared-arrival-safe. |
| 10483 | `eval_runtime_records` | Shared | Shared desired/gate facts exported for evals; local test execution does not make desired selection shared-arrival-safe. |
| 10763 | `member_reconcile_fault` | Local | Local runtime.reconcile-decision observation at a local snapshot. |
| 10796 | `member_reconcile_faults_for` | Local | Local runtime.reconcile-decision observation at a local snapshot. |
| 10889 | `pending_observer_refresh_attempt` | Shared | Shared pending refresh selection; LIMIT 1 must select a canonical request. |
| 10905 | `gate_request_for_owner` | Shared | Shared current gate request selection and approval fences. |
| 11221 | `pending_attention_closed_by_condition` | Shared | Shared historical attention source/condition selection; derived-attention replaces current heuristics, but retained historical readers still need canonical order. |
| 11318 | `attention_requests` | Shared | Shared attention history listing; preserve stable source order. |
| 11350 | `pending_attention_requests_raised_by` | Shared | Shared pending attention source set, even for host-authored requests. |
| 11502 | `attention_items` | Shared | Shared attention source selection/onset time and subscription-failure fold; acceptance time alone is not a total order. |
| 11533 | `attention_items` | Shared | Shared attention source selection/onset time and subscription-failure fold; acceptance time alone is not a total order. |
| 11697 | `selected_actual_origin` | Shared | Shared selected runtime/resource ownership attribution; must match canonical actual-state selection. |
| 11712 | `selected_actual_origin` | Shared | Shared selected runtime/resource ownership attribution; must match canonical actual-state selection. |
| 12396 | `pending_subscription_mission_requests` | Shared | Shared durable pending delivery requests; affects dispatch order and grouped attention sources. |
| 12523 | `subscription_mission_deferral` | Shared | Shared legacy replicated deferral; distinct from local_subscription_mission_deferrals. |
| 12568 | `pending_schedule_work_requests` | Shared | Shared pending schedule requests and action ordering. |
| 12588 | `schedule_work_start_for_request` | Shared | Shared request disposition/start winner. |
| 12627 | `usage_summary_at` | Shared | Shared replicated usage fold: response_rollup and context occupancy are order-sensitive. |
| 12661 | `usage_summaries_at` | Shared | Shared usage fold per subject; same issue as usage_summary_at. |
| 12921 | `usage_period_rows` | Shared | Shared period totals; equal observed-time ties currently use local store index. |
| 13051 | `claims_for_subject_kind_at` | Local | Local raw-history page using a local cursor; callers reducing shared state need the canonical helper instead. |
| 13235 | `agent_last_activity_at` | Local | Local operational activity/silence view combines host-only timeline observations with newest arrivals. |
| 13241 | `agent_last_activity_at` | Local | Local operational activity/silence view combines host-only timeline observations with newest arrivals. |
| 13242 | `agent_last_activity_at` | Local | Local operational activity/silence view combines host-only timeline observations with newest arrivals. |
| 13243 | `agent_last_activity_at` | Local | Local operational activity/silence view combines host-only timeline observations with newest arrivals. |
| 13301 | `timeline_claim_rows_for_incarnation_at` | Local | Local harness timeline/history page. Current timeline retention is local; legacy rows can appear in claims. |
| 13339 | `claims_for_kind_at` | Local | Local raw-kind history page; not a permitted source of arrival-ordered shared reduction. |
| 13435 | `work_claim_is_orphaned` | Shared | Shared runtime-incarnation/terminal-state selection controls durable work ownership. |
| 15220 | `export_replication_for_heads` | Shared | Shared immutable batch position, not inter-batch arrival. Relative store indexes within one atomically admitted batch preserve its wire position; migrate to explicit position for uniform enforcement. |
| 16930 | `resolve_mission_run_inputs` | Shared | Shared resource input claim identity persisted in a run; latest must be canonical. |
| 17066 | `desired_row_at` | Shared | Shared desired selection within a local historical frontier. Winner is ancestry/revision/id and is commutative today; traversal and returned logical rows must still be canonical. |
| 17596 | `latest_resource_kind` | Shared | Shared resource schema/type selection from durable observations. |
| 18503 | `rebuild_planning_tx` | Shared | Shared planning fold; out-of-order started/candidate/approved causes skipped updates, quarantine, and different launch state. |
| 18785 | `validate_message_transition` | Shared | Shared message lifecycle validation; newest arrival can authorize/reject the wrong transition. |
| 19017 | `latest_claim_id_tx` | Shared | Shared predecessor selection for new durable claims. Evidence identity must not depend on arrival. |
| 19470 | `current_harness_at` | Shared | Shared replicated harness/work activity reduction; local host overlays are separate. Work activity still compares physical indexes against runtime/harness indexes. |
| 19525 | `claim_ids_at` | Shared | Shared evidence/optimistic-fence identity set at a local frontier; serialized ordering must be stable. |
| 19546 | `desired_conflicts_at` | Shared | Shared conflict set at a local frontier; membership is commutative, but returned ordering is observable. |
| 19825 | `pending_human_reviews_tx` | Shared | Shared human gate/review source ordering and pending state. |
| 19866 | `attention_request_view_tx` | Shared | Shared legacy ask/resolution selection; equal acceptance times lack writer/sequence/position ties. |
| 19879 | `attention_request_view_tx` | Shared | Shared legacy ask/resolution selection; equal acceptance times lack writer/sequence/position ties. |
| 19968 | `pending_attention_requests_tx` | Shared | Shared pending attention sources; physical arrival changes order. |
| 20294 | `attention_target_moved_on_tx` | Shared | Shared terminal/recovery episode selection; ClaimMoment also compares (accepted time, local index), which must become a canonical identity. |
| 20365 | `attention_target_moved_on_tx` | Shared | Shared terminal/recovery episode selection; ClaimMoment also compares (accepted time, local index), which must become a canonical identity. |
| 20382 | `attention_target_moved_on_tx` | Shared | Shared terminal/recovery episode selection; ClaimMoment also compares (accepted time, local index), which must become a canonical identity. |
| 20412 | `attention_target_moved_on_tx` | Shared | Shared terminal/recovery episode selection; ClaimMoment also compares (accepted time, local index), which must become a canonical identity. |
| 20447 | `pull_request_state_tx` | Shared | Shared selected PR resource state used to close/currently show attention. |
| 20522 | `loop_run_moved_on_tx` | Shared | Shared loop recovery/episode selection. |
| 20578 | `mission_run_ended_tx` | Shared | Shared terminal run episode onset. |
| 20683 | `attention_target_state_tx` | Shared | Shared runtime/observer/subscription/loop/message source state selection. |
| 20694 | `attention_target_state_tx` | Shared | Shared runtime/observer/subscription/loop/message source state selection. |
| 20702 | `attention_target_state_tx` | Shared | Shared runtime/observer/subscription/loop/message source state selection. |
| 20711 | `attention_target_state_tx` | Shared | Shared runtime/observer/subscription/loop/message source state selection. |
| 21180 | `operational_repair_plan_tx` | Shared | Shared persisted wake failure selection used in fenced durable repairs; local lease/time inputs stay outside timeless shared digests. |
| 23307 | `fleet_invites` | Shared | Shared invite created/redeemed/revoked lifecycle; token secrets and attempt counters are separate local rows. |
| 23922 | `seed_replica_envelopes_tx` | Shared | Shared immutable batch wire position during legacy envelope seeding; relative intra-batch index is stable, unlike global arrival. |
| 26124 | `generation_run_tx` | Shared | Shared run ownership fallback from generation/run creation claims. |
| 26139 | `generation_run_tx` | Shared | Shared run ownership fallback from generation/run creation claims. |
| 26175 | `run_tree_of_tx` | Shared | Shared aggregate/root ownership fallback; chosen creation/proposal claim must be stable. |
| 26204 | `run_tree_of_tx` | Shared | Shared aggregate/root ownership fallback; chosen creation/proposal claim must be stable. |
| 26519 | `try_project_simple_replication_tx` | Local | Local projection-backlog frontier scan. Each shared aggregate must still reduce canonically; sorting run_updates only by acceptance time is insufficient for equal-time ties. |
| 28599 | `seat_queue_inputs_tx` | Shared | Shared queue moves: nearly canonical but missing batch-id/explicit-position/final-id tie breaks; use the helper. |
| 29548 | `enrich_step_summaries_at` | Shared | Shared durable progress/submission summary at fixed as_of; equal-time local-index ties alter latest content. |
| 29846 | `enrich_step_wake_at` | Shared | Shared durable wake-failure episode selection; host-local live harness overlay remains local. |
| 29908 | `harness_incarnation_for_key_at` | Shared | Shared replicated incarnation selection for wake fences. |
| 30202 | `active_step_blockers_tx` | Shared | Shared pending blocker source ordering at fixed as_of. |
| 31216 | `loop_run_views_tx` | Shared | Shared loop state winner and ordered round results. |
| 31233 | `loop_run_views_tx` | Shared | Shared loop state winner and ordered round results. |
| 34044 | `the_newest_message_to_a_recipient_is_found_by_index` | Local | Isolated test/fixture verification query; does not project live shared state. |
| 39669 | `replication_inventory_uses_the_batch_indexes` | Local | Isolated test/fixture verification query; does not project live shared state. |
| 42599 | `a_worker_can_submit_each_retry_attempt_once` | Local | Isolated test/fixture verification query; does not project live shared state. |

CANONICAL_ORDER (705-706) and CANONICAL_ORDER_DESC (707-709): Shared ordering helpers, with the intra-batch physical-index limitation described above. RUNTIME_SUBJECTS and SUBJECTS_IN_RANGE order by subject, not by store_index; their index bounds are Local snapshot membership. Index definitions on store_index accelerate local log access but do not themselves select a shared winner.

## Other arrival-derived state that an ORDER BY store_index search misses

- Document latest binding/token selection: created_index DESC in put_document, mission resolution, latest_document_hash, document_bindings_at and latest_document_token. That cursor cannot choose the shared version. Repeated same-name/same-hash bindings also need a canonical binding claim representative rather than INSERT OR IGNORE arrival.
- Message ordering: message_index created_index, MIN(store_index), recipient mailbox order and exported filenames are local cursor metadata. The shared mailbox source, status and onset use canonical claim identity/time; compare logical message rows and recipient order independently of physical file names.
- ClaimMoment at 20229 compares (accepted time, store_index) for attention episode closure. current_harness_at compares work/runtime/harness physical indexes. usage_period_rows uses (observed time, index) and usage_summary_from_rows overwrites rollup/context fields as rows arrive. Canonical SQL alone is insufficient unless these comparisons change too.
- try_project_simple_replication_tx sorts run_updates only by accepted_at_unix_ms; equal-time ordering inherits the local scan. Its backlog scan may remain local, but shared updates must use the complete key. seat_queue_inputs_tx has an almost-canonical hand-written order that must use the same helper.
- Partial shared digests hide differences inside run/generation/step/proposal fields, even in already covered tables. updated_at from effective local lease renewal must be separated from the durable source timestamp rather than blindly hashed or silently used as a shared clock.

## Claim-derived shared readers without a materialized table

These are Shared: actual/effective subject state and selected runtime/resource owner; harness replicated snapshot; message lifecycle/unread/reminders; gate request/result/review facts; subscription and schedule request dispositions; observer refresh/state; loop state/round results; reconcile and delivery failure/recovery episodes; fleet member/invite lifecycle; seat queue movement; progress/result summaries; replicated usage snapshots; shared checkpoint protocol facts. Shared outputs at a fixed as_of/recipient need a canonical source representation and per-source/table digest coverage even when currently folded directly from claims. Local reachability, harness liveness, lease overlay and notification delivery do not define shared completion or disappearance.

Some readers already use CANONICAL_ORDER or sort IDs; retain their deterministic behavior and route their implementation through the common helper. Checkpoint proof canonicalizes physical indexes on a copy before comparison, which can hide an arrival-order bug on real replicas. Therefore the new test also compares real stores before proof normalization and after actual production tombstone/delete/replay operations.

## Derived-attention coordination

The attention builder confirmed that its reviewed proposal introduces one snapshot function, not an attention table. Design: `doc/fleet/smalltalk/attention-derived-design@e3a78d7c336f3aa068629121baa967c40e9ef7695885de62766667bc0c0975b3`; reply: `message/177021f57187a57f`, following audit coordination `message/60e73ec7b9f4374a`.

Its sources are person steps/human gates/revision approvals, launch candidate/preview review, unread person messages/reminders, held subscription source gates, current durable operational failures and checkpoint waits. Legacy attention.requested/resolved becomes historical. Proposed person asks extend step_runs and claim-derived person metadata; final DDL is pending in that mission. Hash new shared step columns automatically and add canonical source inventories for nonmaterialized dependencies. Do not hash the local checkpoints ledger as if every host had sealed/trimmed simultaneously.

The view must be tested separately at identical explicit as_of and recipients. Identity includes source + recipient + gate/episode, preserving multiple reviewers, attempt/generation replacement, original waiting age, reminder versions, preview replacement and failure/recovery episode fences. Sort by priority, waiting_since, source_id, person_id, episode. Ownership filtering precedes pagination. Final helper API and table inventory additions must be sent back when implemented. No need to wait for that mission to complete this audit.

## Regression written first

`crates/st3/src/store/canonical_audit.rs`, included under store::tests::canonical_audit, compares all 16 shared logical materialized tables with independent SHA-256 hashes of sorted full logical rows. It excludes only the explicitly local arrival/lease-overlay columns listed in SHARED_TABLES; selected document and message readers are compared separately, and replicated usage is checked. These hashes are a test oracle, not the planned incremental production digest API.

The history includes mission/work/run state, two immutable document versions, started/candidate/preview/approved planning, message send/close, cumulative usage and harness transitions. It uses equal acceptance times, reversed and interleaved single-envelope arrival, on-disk restart plus full replay, checkpoint proof, and real production tombstone/delete/replay on isolated stores. All mismatches are accumulated by phase and table instead of stopping at the first planning mismatch. No shared daemon or agent seat is restarted.

Validation command: `cargo test -p st3 --lib every_shared_projection_agrees_after_shuffle_restart_and_checkpoint -- --nocapture`.

Observed baseline failure: reverse delivery disagrees on planning_sessions, planning_candidates,
planning_previews, selected document bindings and usage summary in all three phases. Interleaved
delivery disagrees on planning_sessions, planning_candidates and selected document bindings in
all three phases. The six-table graph_digest still matches in every comparison. Both checkpoint
proofs pass, confirming that proof normalization alone does not expose the live-replica bug.

`every_persistent_table_has_a_projection_scope` requires classification of each persistent
table. The original source-text guard `shared_folds_never_order_by_local_arrival` was retired:
literal SQL inspection cannot distinguish shared winner selection from legitimate local
pagination metadata and requires an implementation-name allowlist. Behavioral shuffle,
restart, checkpoint, reader and digest comparisons enforce the invariant without pinning SQL
wording. Run the current checks with `cargo test -p st3 --lib canonical_audit -- --nocapture`.
At the original audit baseline, table classification passed; the source guard named 62 shared
arrival-order clauses; shuffle/restart/checkpoint comparison failed with the divergences above.

The required sync invariants and these enforcing test names are stated in
[replication](replication.md#sync-invariants), linked from the README and fleet join guide.

The fix step must extend this foundation with non-empty fixtures for currently empty proposal/dependency tables, multiple writers/equal-time conflict ties, every nonmaterialized shared source, derived attention at fixed as_of, and production per-table incremental digests. It must check incremental insert/update/delete results against a full digest oracle, name differing tables in replication status/doctor, and ensure an unchanged table never needs a whole-table rescan. Restart/checkpoint tests must compare logical rows and each production digest, not just the legacy six-table graph_digest.

## Required fix boundaries

1. Shared canonical helper + exhaustive local exceptions + behavioral coverage for shared winner selection independent of local arrival. Retain deterministic ancestry/operation selection and local cursor semantics.
2. One incremental digest per logical shared table/source covering all shared columns, with insert/update/delete maintenance in the same transaction, stable serialization/PK order, full-rebuild validation and migration. Cheap cache invalidation alone is not incremental hashing. Physical indexes, receipt metadata, local clocks and live overlays stay outside. Extend replication status, heal diagnostics and doctor with named mismatches, and keep format/version compatibility explicit.
3. Expand shuffle/restart/checkpoint CI coverage to every source and derived attention view. The audit commit intentionally contains the failing regression; it is not a green merge candidate. Only the completed fix PR with st/ci green on its own head may join the smalltalk merge train.
