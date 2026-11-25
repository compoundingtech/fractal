# Light node and home node

This is the agreed architecture and staged implementation plan, not an available node mode or a measured CPU result. Existing full nodes keep their current authority until an explicit cutover.

## Topology and policy

The **home node** is one fixed, full graph node. It owns fleet planning, admission decisions, routing, lists, attention, roll and frontier reductions, and ordinary replication with other full nodes. Home selection is configuration, not automatic discovery or failover.

The **light node** owns physical execution: local seats, process-birth identity, PTYs, drivers, file/process watches, bounded local mail custody, and a durable command/receipt journal. It does not ingest or reduce the fleet graph or run a full-replica worker. The light node establishes one authenticated outbound bidirectional session to its home; this is a new protocol, not the existing dial-out replication route.

Decided policies:

- Existing seats keep running offline. Local observe, attach and emergency stop remain available; new fleet-dependent launches, assignments and controls wait for reconnection. Disconnect or laptop sleep is not evidence that a process exited and is not a lease that kills a healthy PTY.
- The home orchestrates **already-granted seats**. The light node rejects stale or mismatched operations and is the only physical execution owner. The host-execution private key and local secrets stay on the light node; the home signs with its own identity, never as the light node. A scoped grant must bind host, seat, operation, incarnation and allowed evidence.
- Failover is explicit and operator-selected, using a durable monotonically increasing control epoch. There is no automatic standby or full-node fallback.
- Production rollout is one coordinated **whole-light-node cutover window**, not a rolling seat-by-seat migration. Isolated fixtures and shadow planning precede that window.
- Local terminal/status paths remain local. Fleet views/actions use the home, with stale cached views labeled as such offline. Both CLI/TUI and app routes require parity; paired-client support alone does not establish it.

The home is a powerful trusted controller: authentic signed observations prevent forged host evidence, not denial of service or misuse of otherwise authorized commands.

## Executor boundary

The full-node graph remains the decision owner during extraction. It emits immutable fenced commands; the host executor admits them durably and returns physical process/PTY/watch/mail observations and receipts. It must not become another fleet planner.

A command binds a stable command/execution ID, selected control epoch, host and seat identity, operation, target incarnation and applicable launch/PTY/process-birth fences. A successor incarnation requires a new authorization; an old command cannot authorize it. Stop and cleanup require exact provenance, not a reused PID or absence of a response. Existing local cancellation, cleanup, marks and pending telemetry remain authoritative until explicitly converted and acknowledged; see [data authority](data-authority.md), [seat-state truth](seat-state-truth.md) and [node identity](node-identity.md).

Inside the current full node, `fractal-reconcile` separates command execution in `reconcile/host_executor.rs` from physical observation/watch acquisition in `reconcile/host_observations.rs`. Start admission and results still use the existing durable runtime-start journal; graph adoption stays in the planner. Retained native snapshots are fenced by registry events, command execution and existing safety/deadline wakes, not fleet graph writes. The `host-executor-observation` and `host-executor-process` performance labels count actual backend evaluations. The separate `graph-mail-observation` label belongs to the full-node shard adapter: it preserves remote mission dependencies and is not an own-seat mail transport or an offload saving. This extraction does not implement the remote control epoch/session described below.

Finite program/shell payloads, exact-generation signals, lane/root retirement and host recovery use typed resource commands in `reconcile/host_executor/resource_commands.rs`. Their existing native membership/root journals remain the admission and receipt authority; graph attempts and shell ownership are rechecked at physical release. Signal acceptance is not retirement proof, and asynchronous retirement invalidates host evidence before notifying graph adoption.

### Authorization, withdrawal and host admission

During Phase 1, graph-side adapters retain the existing last-moment checks in `start_jobs.rs` and `runtime_starts.rs`: current start intent, selected desired token, owned desired guard, helper lifetime, rollout state, publication and declaration contents. Extraction does not remove those checks or claim graph-free operation.

Before Phase 2 enables remote execution, replace those reads with a durably admitted local authorization record binding the command to its grant, epoch, host, seat/incarnation, desired token, declaration/publication content identities, helper lifetime and rollout permission. The home validates graph facts; the executor validates the signed scope and locally retained authorization/revocation revisions without reading the fleet graph. Payloads are immutable and content-checked, not instructions to resolve a mutable graph name later.

Withdrawal is ordered: the home durably records a revocation and stops issuing dependent work; the executor durably installs it under the same dispatch barrier used for launch, terminalizes accepted-but-undispatched work and resolves backend-entered custody; the home waits for that receipt before treating withdrawal as complete or authorizing conflicting replacement work. Dispatch checks the current epoch, authorization revision and cancellation state atomically with entering backend custody. A lost connection is not a revocation; previously admitted work remains subject to its recorded scope, and offline mode cannot create a successor.

Physical resource admission stays on the host: serialized lane/root fit, native membership and generation checks remain the final launch gate. A replicated grant-shaped claim is never sufficient to launch. The home selects program/shell intent; the host owns acquisition, release and recovery of actual resources.

Run credentials are minted locally for the exact runtime, incarnation, subject, run, step, attempt and authorization revision. `run_credentials::ExecutionBinding` separates optional run identity from native seat identity; non-program seats never acquire a synthetic run. The opt-in program issuer binds the actual engine instance, host generation and attempt and never falls back to graph capability issuance. Unconfigured full nodes retain their existing graph-checked credentials. Local resolution uses only hashed capabilities and durable journal authority, including after restart; it does not look up the graph attempt. Before minting, the home must acknowledge that exact admitted execution. Already acknowledged live authority can mint again while disconnected.

Local admission, revocation, credential issuance and native entry share the journal's serialized barrier. Versioned authority changes have per-epoch sequence numbers and durable home acknowledgements. A replacement first commits revocation, invalidating every predecessor credential immediately, then waits for the home ACK before admitting the successor. A pending native proposal is not authority: an ACK wakes targeted reconciliation, and fresh native/program validation must reoffer it. ACK handling never starts a stale proposal automatically. Retained revoked revisions cannot be resurrected by duplicate admission. Active identities, retained fences, unexpired credential hashes and change payloads have explicit capacity limits; full journals apply typed backpressure rather than drop identities or forget fences.

For fleet requests, the local adapter validates `ST_RUN_CREDENTIAL` against that admitted live execution and creates `RequestAssertion` version 1, signed with the admitted host execution key over the distinct `fractal.light-node.run-request.v1` domain. It binds selected home/epoch, host, subject, runtime/incarnation, run, step, attempt and authorization revision to method, exact origin-form target/query, payload SHA-256, stable request ID, issuance time and at most 30 seconds of validity. The bearer secret never enters the assertion, transport, logs or home store. This conveys a separate run identity, never person or agent impersonation; inherited agent ancestry does not override a carried run capability.

The Unix-only `POST /v1/internal/native/run-request/claim` route accepts `{request_id, claim}` only from a bound program credential. The configured journal/host-key/session service signs and durably forwards the typed claim; clients do not export their credential or construct an authenticated transport. The signed request identity is `POST /v1/claims` with canonical `serde_json` serialization of the original `ClaimInput`, before run attribution. The home invokes `Store::execute_asserted_claim`, reusing the ordinary claim pipeline and all its schema, rules, authority, evidence, CAS and projector gates. This operation covers graph claims only: cross-shard `message.sent` and local-observation kinds require their dedicated effect adapters and are explicitly refused, not silently routed around their gates.

After durably accepting a forwarded result, the same bound program explicitly
posts to `/v1/internal/native/run-request/acknowledge` with
`{request: {request_id, claim}, accepted: {request_id, result: {Ok: value}}}`.
For a durable refused result, `result` is `{Err: {code, message}}` instead.
The Unix response preserves the home's owned refusal code and message exactly;
it does not intern wire strings as static local error codes or erase their identity.
The original typed claim and current credential reproduce its immutable
execution/request digest; another run or changed request cannot retire that
result. The first acknowledgement must match the retained result exactly.
Acknowledgement atomically releases its payload/result capacity while retaining
the scoped request tombstone. A lost acknowledgement response is retryable for
that same logical request after restart; submitting it again returns
`result-expired`, never a second effect. Neither an HTTP response nor a session
send implies consumer acceptance. The Bun fixture first completes a real
`ctx.record`, proves the result remains retained, then explicitly acknowledges
it and admits the next request through a one-slot journal.

The home retains the admitted host key, exact execution binding and graph authorization revision. A delayed report of an ended execution is an inactive tombstone that can be acknowledged, not a renewed start grant. Installing an authenticated newer epoch atomically fences all older execution authority; lower epochs are refused. Signature, scope, current live step/attempt/generation/revision and replay checks run in the same graph writer transaction as the actual effect and durable response receipt. Revocation uses that same writer order. An already-forwarded assertion therefore cannot create a new effect after revocation, even before expiry. An identical signed retry can recover its already committed response after revocation or assertion expiry without creating another effect; conflicting request-ID reuse, including across epochs, fails closed. Retained response receipts and home authority fences are bounded and are not age-deleted. Full-node bound run writes also carry authenticated scope through blocking admission and recheck the attempt in the actual claim transaction, closing the local authentication-to-commit race.

Closing a home epoch freezes new effects immediately while its old authenticated transport drains authority changes and receipts. The home persists a closed epoch fence and revokes all older or equal execution authority in one writer transaction. Reinstalling that same closed epoch cannot reopen it, including after restart; only an explicitly authenticated higher selection can open new authority. Delayed execution reports can settle as inactive tombstones during the drain, but cannot issue a new graph effect.

### Local session resolution

Phase 1 is not complete until an executor-owned resolver can resolve subject, runtime and incarnation from bounded durable local binding metadata, without `live_session`'s graph-running prerequisite. Local read/attach/emergency-stop authority uses that binding plus exact PTY/process-birth provenance; ambiguous, stale or reused identities fail closed. Stop remains terminal-bound, not PID-bound. Restart fixtures must recover a running local session and exercise observe/attach/stop while the graph/session is unavailable. Bindings remain until physical custody and required receipts are settled; retention then follows the journal checkpoint, not arbitrary age.

The durable binding also records lifecycle and custody ownership: executor-owned runtime versus adopted person-owned terminal/harness, including its invocation fence. As required by [terminal binding](terminal-binding.md), stopping an adopted bound seat detaches its graph runtime; it does not kill its harness, shell or terminal. The offline resolver journals that fenced detach for later graph reconciliation without fabricating process-exit evidence. Physical stop is available only for a binding whose recorded custody grants that authority. Phase 1 restart/offline fixtures must recover both ownership classes, prove that a bound-seat detach leaves the person's shell and harness running, and replay the detach exactly once when the home returns without re-adopting the detached invocation.

The core and resource extraction PRs are portions of Phase 1, not satisfaction of this resolver gate or implementation of the Phase 2 wire protocol.

The local resolver implementation is `fractal_reconcile::reconcile::local_session`;
`stop_local_session` uses the existing native generation-fenced stop for owned
runtimes and journals a logical detach for borrowed invocations. The owner-local
`local_session_custody` table retains bounded subject/runtime/native-generation,
launch-operation, lifecycle, custody and invocation metadata at verified launch
and adoption boundaries. Local API reads and terminal attachment consult this
binding before graph routing; remote subjects retain graph routing. A detached
invocation cannot be adopted again, and its `runtime.action.succeeded` detach receipt uses a
stable idempotency key before acknowledgement. No process exit is inferred.
Checkpoint removal requires custody settlement and all required local receipts.
An ended observation is not a request to repeat native control: managed PTY
custody recovery requires this executor's durable exact-generation retirement
receipt. A stopped projection alone retains custody, including when the runtime
ID has already been reused. Unchanged ordinary observations reuse their existing
deduplication reads rather than rewriting the custody binding.
The real-process/PTY fixtures in `reconcile/local_session_tests.rs` reopen the
store, make graph tables unavailable, exercise native reads/attachment and fenced
stop/detach, and retain the borrowed shell. These fixtures require execution in
the integration lane before treating this gate as verified. This is neither a
Phase 2 session implementation nor a CPU/offload measurement.


## Agent/home session

The full target contract follows. The first opt-in Phase 2 slice implements durable **Observe and Stop only**, as detailed below; it does not enable remote Start, terminal-stream transport, mail forwarding or graph/deployment cutover.

1. Authenticate both ends and negotiate a versioned capability and scoped grant. Bind the session to the explicitly selected home and durable control epoch. Reject commands from any other home or an older epoch, including a returning old session.
2. Before first transmission, the home transactionally records epoch, sequence, stable command/execution ID, immutable payload and its association with the planning decision in a durable outbox. Recovery replays that same record, even after a lost ACK; replanning cannot mint another execution ID for the same decision. The executor persists admission and its receipt atomically **before ACK**; an ACK means durable custody, not successful execution. Reject new admission explicitly when bounded capacity is exhausted; never discard accepted work.
3. Deduplicate stable execution IDs against durable custody. Repeated identical commands return the retained result; reuse with different immutable contents is refused, including across epochs. Sequence gaps request replay rather than silently skipping work. IDs bind their original epoch and cannot be reissued in another epoch: an explicit transferred-custody record is required instead. After a lost response, query/replay the same ID, never manufacture a replacement launch.
4. Physical execution records exact provenance and observations locally before sending receipts. Receipt sequence/acknowledgement state and pending local mail survive process restart, sleep and session loss. The home deduplicates receipts, admits signed host evidence and acknowledges only durable acceptance. A joint durable recovery checkpoint binds the home outbox and executor command/receipt high-water marks and settles all referenced physical custody before either side reclaims payloads/results. Retain monotonic retired sequence floors per epoch after compaction; requests below a floor return explicit `result-expired`, never execute again. Persist closed-epoch rejection fences rather than retaining unbounded per-command cross-epoch tombstones. Retention follows that checkpoint, not session lifetime.
5. On reconnect, reconcile durable command/receipt high-water marks and drain retained evidence before releasing new fleet-dependent work. Keep connection health, command backlog, local runtime health and last acknowledged sequence separate. Missing evidence makes fleet freshness, roll and mail coverage unknown, never spuriously healthy.
6. Epoch installation is a durable execution barrier, not just an incoming-message filter. Under the dispatch barrier, fence the old epoch and terminalize all its accepted-but-undispatched work. Reconcile retained receipts and settle backend-entered custody, or durably transfer its exact incarnation/custody to the new home. Conflicting seat effects remain blocked until that resolution; only then grant replacement control. Reject an old home's commands even if its connection returns. Local emergency stops are recorded and reconciled, not bypassed by replay.

Terminal streams need separate ephemeral flow control and cannot substitute for durable command receipts. This slice leaves existing terminal-stream and mail routes unchanged. Local process/PTY observations and emergency control remain available without a home connection.

### First Phase 2 slice: existing native custody

`fractal-peer::light_node` implements a versioned SQLite command/receipt journal and
an authenticated bidirectional websocket opened **outbound by the seat agent**.
`light_node_session` is explicit opt-in configuration: the home role names its
`listen` address and bounded `grants`; the agent role names `home_endpoint` and one
`grant`. Both ends pin the same home/host identities, Ed25519 public keys, positive
control epoch, exact subjects and explicit `observe`/`stop` operations. It requires
the existing admitted fleet identity and fleet authentication configuration.
Unencrypted `ws` is restricted to literal loopback or tailnet addresses; other
routes require `wss`. Version, key, grant and epoch mismatches fail closed.

Both upgrade verifiers require a valid asymmetric member signature as well as
the exact advertised pin; the shared fleet HMAC alone is not identity proof.
The request signs its actual WebSocket challenge and the response signs a fresh
home nonce bound to that exact request. Every application frame is pinned-member
signed over this channel transcript, its direction and its strictly increasing
sequence before any journal/native effect. Replayed upgrade headers cannot
reserve the seat lease without a fresh signed Hello, whose wait has a deadline.

Start grants, either configuration role containing Start, command enqueue and
seat admission explicitly refuse with `light-node-start-not-supported`, including
attempts to reuse retired execution identities. There is no remote launch adapter
or transported prepared-start payload. Native Observe/Stop resolves exact current
local custody and generation without a seat graph read; a borrowed terminal Stop
detaches the invocation and never signals its shell or harness. These commands
control native local sessions, not the separate program engine's VM lifecycle.

Authoritative session state is under `$STATE/light-node-session/v1/`, with private
directory/database permissions. Decisions, accepted commands, entered native
custody, receipt sequences, closed epochs and retained identity/floor records are
durable before their protocol acknowledgement. A replay returns retained receipt
custody, never repeats an effect. Joint checkpoint reclamation can return explicit
`result-expired`; it is not evidence that a command never ran and must not cause a
replacement execution ID to be invented. Bounded queues and identity/authority
tables apply explicit backpressure rather than deleting accepted or unresolved work.

Native signal acceptance is not whole-scope retirement. A durable native
finalization phase retains the exact retirement snapshot and blocks terminal
receipt transmission, ACK, checkpoint reclamation and epoch drain until local
custody finalization succeeds. Both a returned Store error on a live connection
and a daemon crash retain that witness for retry; bootstrap repairs it before
transmission. Logical session identity and physical native incarnation are
distinct, and finalization uses the latter. If entry committed before its proof,
a positively identified still-running generation can reconcile the retained
Stop under the current serialized authority/cancellation barrier. A replaced or
ambiguous generation cannot be signaled; missing evidence never proves closure.

Native custody admission uses the existing exclusive SQLite writer and an atomic
owner-local transaction, without preparing graph projection finalizers. It
continues to work when the graph tables are unavailable. Exact owned stops retain
`native_retirement_verified` in `local_session_custody` only after positive
whole-scope native proof, fenced against the complete current binding (logical
and native incarnation, lifecycle and custody). A changed binding or new native
generation starts without that proof. Existing databases gain the column with a
false default while retaining their authoritative bindings.

Unknown-entry recovery consumes this retained physical proof without signalling
an already-ended PTY again. Direct native stop without retained proof, absence,
leader exit and a reused runtime ID cannot manufacture closure. A crash between
the physical stop and proof persistence therefore remains conservatively
unresolved. Physical proof alone does not settle custody: the separate durable
completion receipt and local-finalization barrier still gate settlement,
transmission and reclamation.


Epoch replacement first closes old authority. The old home connection remains
available to drain real terminal receipts and authority changes/ACKs; queued home
commands are not declared settled merely because control is changing. Both peers
durably confirm their drained epoch. A signed, pinned retained-epoch response is
barrier-only and recovers a lost final acknowledgement; it cannot execute old work.
The seat retains the exact prior endpoint across restart. Unknown entered native
custody blocks replacement until positive exact-generation closure, not absence.
The home closes its actual graph-writer epoch before accepting further effects.
Keep the old home and its signing identity available until this drain completes;
this does not implement unattended home discovery, key rotation or failover.

Home command administration is owner-local Unix only:
`POST /v1/internal/light-node/commands`,
`GET /v1/internal/light-node/commands/{execution}?host=...`,
`GET /v1/internal/light-node/executions`, and
`POST /v1/internal/light-node/commands/{execution}/cancel`.
The existing serving boundary rejects bound harness/program callers; these routes
are not mounted on the TCP client gateway or home session listener. Reported
execution bindings describe admitted identity, not current runtime health.

The concrete program issuer uses the selected local journal and host key, waits
for its own admission and predecessor revocation ACKs, and revokes before the
engine forgets an attempt. Newly admitted full-node attempts keep their existing
canonical graph admission gate; resolving/signing an admitted credential is local.
The guarded native Unix route
`POST /v1/internal/native/run-request/claim` forwards request-bound host assertions
through that same outbound session. The home reuses the real ordinary claim
pipeline, current authority checks and atomic effect/retry-receipt writer fence.
The bearer credential never goes on the wire. Stable Unix HTTP retries recover
their durable result even when signed envelopes have fresh timestamps; returning
an HTTP result does not invent a consumer acknowledgement. This is ordinary claim
forwarding only, not message/resource action parity or whole-node cutover.

Owned custody uses the actual native generation unchanged; the borrowed-invocation
fixture separately proves a distinct logical identity and a detach that leaves the
real native PTY alive across authenticated receipt replay.
The real native two-node fixtures exercise retained receipt replay, duplicate
identity fencing, reused native generations, offline observe/stop and paired epoch
replacement without a seat graph. The real Bun fixture uses the production issuer,
guarded Unix listener, host signatures, outbound session and home graph effect
pipeline, including own-ACK waiting, exact-once retry and revocation on completion.
Run the Linux integration lanes and obtain an exact-head independent review before
treating these exit gates as verified; no CPU/offload or deployment result is claimed.

**Next reviewed slice:** implement graph-free remote Start with locally admitted,
withdrawable authorization replacing the remaining full-node graph preflight
reads, while preserving workspace leases, launch-freeze, kernel/capacity and native
producer/cleanup custody. This Observe/Stop slice does not weaken those start gates.

### Independently packaged physical runtime

`fractal-light-node-agent` is the light-node process, not another mode of the full
daemon. Its normal Cargo dependency closure contains the shared session/journal
core, member authentication and native runtime, but no graph Store, replication,
fleet reductions, planner, reconciliation daemon or full CLI. The full node uses
the same canonical wire, authentication, authority, custody and session types;
the graph-backed home effect adapter remains on the home.

This package is an independently reviewable prerequisite for Phase 3, not the
whole-node cutover itself. It supports the landed Observe/Stop protocol only.
Start configuration and grants still fail with `light-node-start-not-supported`;
this package neither imports an unlanded Start implementation nor changes shared
capacity reservations. Graph-free remote Start remains the separate next slice
above. Existing graph membership, deployment, caller routing and production
planning are not changed or silently disabled by installing this binary.

Run `fractal-light-node-agent --config /absolute/private/light-node.json`.
Configuration version `1` contains these explicit operator-selected fields:

| Field | Meaning |
|---|---|
| `state_dir` | Private agent journal, exclusive process lock and `agent.sock`; this is not a graph directory. |
| `native_state_dir`, `pty_root` | Existing native exec/log and PTY records; the agent opens no graph there. |
| `pty_binary` | Optional absolute pinned PTY executable; otherwise use only the packaged sibling `bin/pty`, never an ambient full CLI. |
| `fleet_id`, `fleet_secret_file`, `member_key_file` | Existing fleet identity and privately provisioned credentials. The member key must match the pinned host grant. |
| `session` | Canonical `role: "agent"` session with exact home endpoint, member pins, epoch and Observe/Stop scope. |
| `native_sessions` | Explicit canonical `LocalSession` custody metadata, not a list of subjects to discover or adopt. |

Configuration and credential files must be owned regular files inaccessible to
group/other users; final-component symlinks are refused. The state directory is
created private (`0700`), and retained files are never permission-repaired or
truncated. An exclusive directory process lock prevents a second agent from
replacing a live API socket. Fresh native custody requires positive exact native
generation/provenance evidence and current grant authority; reoffering identical
retained custody is restart-idempotent, including after a completed Stop. An empty
journal does not claim processes merely because their subject appears in a grant.
This provisioning interface does not implement production custody transfer from
the full graph or permission to run two decision owners over the same execution.

The Nix package is `packages.<system>.fractal-light-node-agent`. Its standalone
Home Manager module is `homeManagerModules.fractal-light-node-agent`, with
`services.fractal-light-node-agent.enable` and an absolute external string
`configFile`. Do not use a Nix path literal or place credentials/configuration in
the Nix store. The opt-in Linux user service uses only this binary and its pinned
PTY sibling. Composing an enabled full `services.fractal` for the same user is
refused because opaque external configurations cannot prove disjoint custody.
No Darwin service or automatic production cutover is provided. Service stop or
transport loss stops the agent, not healthy native processes: `KillMode=process`
preserves independent PTY/process custody, and restart repairs retained native
finalization before exposing session receipts. Reconnect/backoff belongs to the
real session client; the service adds no sampling or watchdog poller.

`GET /v1/performance` is available only on the private `state_dir/agent.sock`
Unix socket (`0600`), not a TCP listener. Its first-class physical counter measures
CPU nanoseconds across all agent threads and excludes native children and the
home. It identifies each accounting instance with a fresh UUID and PID, an
explicit CPU baseline, monotonic accounting elapsed time, and sample brackets.
Accounting initialization is not advertised as OS process birth. Clock failure
returns a structured unavailable response, never a fabricated zero. Requests
sample on demand; no background timer is needed to maintain the counter.

Bounded work counters describe actual observed session events and native
Observe/Stop/recovery entry, not an exhaustive attribution of all CPU. Synchronous
attribution measures only the current thread, does not include asynchronous
suspension, and cannot drop or repeat a native effect when accounting fails.
These counters make the later whole-system measurement attributable; they do
**not** demonstrate the 600-second total-background under-five-point gate, replace
the full process census, or transfer CPU attribution for native children/home.
Real two-process home/agent fixtures and native generation/proof/restart tests
must pass in the Linux lanes before the package is considered ready.



## Phases and exit gates

| Phase | Deliverable | Exit evidence |
|---|---|---|
| **0 — Baseline** | Continue reactive/incremental fixes and resolve mixed-version compatibility. Establish a same-build baseline before attributing offload savings. | Settled 600-second census of counted background processes at normal seat load; UI closed and visible-client cases separately, normal write/mail rates, sleep/wake behavior and deterministic work counters. |
| **1 — Physical executor extraction** | Extract the command/admission/observation boundary inside the existing full node. Graph decisions and deployment topology do not move. | Unrelated fleet writes cause **zero executor evaluations**; unchanged observations cause zero downstream work. Isolated restart, reused-PID, duplicate-command and cancel-before-admit cases preserve existing behavior. Measure executor-only CPU without claiming offload savings. |
| **2 — Home planning and session** | Implement authenticated reverse session, durable replay and the home adapter. Shadow home planning without execution; exercise authority transfer and real execution in isolated fixtures. | Compare commands/results at explicit cuts; count bytes/work per changed seat. Session loss, agent restart and sleep/partition fixtures prove no duplicate spawn, no lost accepted receipt and no false exit. Measure home RTT and fleet first-view/action latency separately from local PTY latency. |
| **3 — Whole-node cutover** | In one coordinated window, transfer all local seat planning to the home, route every caller, and stop light-node fleet publications, full replication and reductions. Preserve physical ownership and PTYs. Archive the old graph in place, without copying it on the light node. | Launch/stop/mail/attach/status parity across CLI, drivers, TUI and app; home recovery and partition cases; settled 600-second **total background CPU below 5% of one core** at normal load, plus post-wake catch-up. The split alone is not proof of this target. |
| **4 — Recovery and retirement** | Complete explicit epoch-fenced home rebind and retire obsolete full-node paths, membership and health expectations only after retained signed output is accepted. | Old-home-return/new-home fixture allows one authority; signed outbox drain and no unacknowledged former member authority; bounded reconnect work/backlog, steady-state light-node CPU and home CPU increase. |

Before membership retirement, rollback restores the old graph decision owner only after fencing the remote epoch, incorporating new journal output, and acquiring a current, fenced graph decision cut from the home or another authoritative full replica. That cut includes declaration, retirement, placement and policy changes even when they emitted no executor command. The restored planner cannot start from the archived graph plus journal alone. If no authoritative current cut is available, remain in local-only offline operation; do not resume stale planning. Afterwards, returning to full-node operation requires explicit admission and rehydration, not starting against an old graph. Never clone execution keys, reset identity pins or present partial state as a full replica.
