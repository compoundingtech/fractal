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

## Agent/home session sketch

The following is a protocol contract to implement, not a claim about existing wire support:

1. Authenticate both ends and negotiate a versioned capability and scoped grant. Bind the session to the explicitly selected home and durable control epoch. Reject commands from any other home or an older epoch, including a returning old session.
2. Commands have per-epoch ordered sequence numbers and stable IDs. Persist admission and its receipt atomically **before ACK**; an ACK means durable custody, not successful execution. Reject new admission explicitly when bounded capacity is exhausted; never discard accepted work.
3. Deduplicate stable execution IDs against durable custody. Repeated identical commands return the retained result; reuse of an ID with different immutable contents is refused. Sequence gaps request replay rather than silently skipping work. After a lost response, query/replay the same ID, never manufacture a replacement launch.
4. Physical execution records exact provenance and observations locally before sending receipts. Receipt sequence/acknowledgement state and pending local mail survive process restart, sleep and session loss. The home deduplicates receipts, admits signed host evidence and acknowledges only durable acceptance. Retention follows acknowledgements and required recovery custody, not session lifetime.
5. On reconnect, reconcile durable command/receipt high-water marks and drain retained evidence before releasing new fleet-dependent work. Keep connection health, command backlog, local runtime health and last acknowledged sequence separate. Missing evidence makes fleet freshness, roll and mail coverage unknown, never spuriously healthy.
6. Explicit failover fences the old epoch on the light node, reconciles retained receipts with the new home, then grants the new epoch control. Reject an old home's commands even if its connection returns. Local emergency stops are recorded and reconciled, not bypassed by replay.

Terminal streams are separately flow-controlled, ephemeral data over the session; they cannot substitute for durable command receipts. Local process/PTY observations remain available without that session. Remote access and mail custody need explicit authorization and backpressure, not whole-store replication.

## Phases and exit gates

| Phase | Deliverable | Exit evidence |
|---|---|---|
| **0 — Baseline** | Continue reactive/incremental fixes and resolve mixed-version compatibility. Establish a same-build baseline before attributing offload savings. | Settled 600-second census of counted background processes at normal seat load; UI closed and visible-client cases separately, normal write/mail rates, sleep/wake behavior and deterministic work counters. |
| **1 — Physical executor extraction** | Extract the command/admission/observation boundary inside the existing full node. Graph decisions and deployment topology do not move. | Unrelated fleet writes cause **zero executor evaluations**; unchanged observations cause zero downstream work. Isolated restart, reused-PID, duplicate-command and cancel-before-admit cases preserve existing behavior. Measure executor-only CPU without claiming offload savings. |
| **2 — Home planning and session** | Implement authenticated reverse session, durable replay and the home adapter. Shadow home planning without execution; exercise authority transfer and real execution in isolated fixtures. | Compare commands/results at explicit cuts; count bytes/work per changed seat. Session loss, agent restart and sleep/partition fixtures prove no duplicate spawn, no lost accepted receipt and no false exit. Measure home RTT and fleet first-view/action latency separately from local PTY latency. |
| **3 — Whole-node cutover** | In one coordinated window, transfer all local seat planning to the home, route every caller, and stop light-node fleet publications, full replication and reductions. Preserve physical ownership and PTYs. Archive the old graph in place, without copying it on the light node. | Launch/stop/mail/attach/status parity across CLI, drivers, TUI and app; home recovery and partition cases; settled 600-second **total background CPU below 5% of one core** at normal load, plus post-wake catch-up. The split alone is not proof of this target. |
| **4 — Recovery and retirement** | Complete explicit epoch-fenced home rebind and retire obsolete full-node paths, membership and health expectations only after retained signed output is accepted. | Old-home-return/new-home fixture allows one authority; signed outbox drain and no unacknowledged former member authority; bounded reconnect work/backlog, steady-state light-node CPU and home CPU increase. |

Before membership retirement, rollback restores the old graph decision owner only after fencing the remote epoch and incorporating new journal output. Afterwards, returning to full-node operation requires explicit admission and rehydration, not starting against an old graph. Never clone execution keys, reset identity pins or present partial state as a full replica.
