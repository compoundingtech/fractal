# Whole-Mac light-node cutover runbook

This is the **Phase 4 delivery slice: documentation**, not rollout approval or an
implemented migration command. The [architecture's phase table](light-node.md#phases-and-exit-gates)
uses different numbering: this runbook covers its whole-node cutover and recovery
exit gates. The **home node is the explicitly selected fixed home**; the **light
node is the Mac**. There is no automatic home selection or full-node fallback.

Do not execute this runbook until every readiness gate below has concrete evidence
and the operator has approved the exact deployment window. Documentation landing,
a packaged runtime and passing isolated fixtures do not authorize production
custody transfer. Record exact supported commands from the admitted deployment
artifact in the private execution record before the window; no migration, Darwin
service or transfer command is invented here. Missing tooling is a stop condition,
not permission to edit journals or replace a fenced transaction with SQL.

## Ownership and execution record

Name the operator, home owner, Mac execution owner and rollback decision owner.
FPL reviews the exact documentation head; FPL/CIM owns landing. Deployment and
membership changes need their own applicable approvals. Keep endpoints, account
names, physical paths, keys and raw process arguments in private evidence, not in
this public runbook or PR.

Record artifact/build hashes, previous service definitions, current member/key
pins, selected home and epoch, graph decision cuts, custody inventories, durable
receipt positions, publisher sequences, operation IDs and timestamps. Preserve
that record across interruption. Reconcile an interrupted step from its durable
receipt and current native generation before resuming; do not blindly repeat it.

## Preflight: all gates must pass

- **Artifacts and actual Mac support:** admitted compatible home and agent builds
  have exact-head independent GOs and green required Linux/two-node/Bun/native
  fixtures. Separately prove the actual Mac binary, pinned PTY and Darwin service
  lifecycle on the intended build. A Linux user unit or Linux KVM result is not
  proof of Darwin packaging, startup, stop or orphan preservation.
- **Authority transfer:** production tooling can quiesce the old planner, export
  and import all durable local authority, install a monotonically newer home
  epoch, and produce durable transfer receipts. Capture cancellation, pending
  launch/backend entry, credentials/revocations, owned and borrowed custody,
  process-birth/PTY/invocation fences, watches, pending mail and unacknowledged
  outputs. No seat or helper may have two physical execution owners.
- **Home and routing:** the fixed home is reachable, authenticated, capacity-ready
  and caught up to a named current decision cut. Its planner has been compared in
  non-executing shadow mode. Inventory every caller: CLI, drivers, TUI, app,
  publishers and automation. Prove local attach/status/emergency stop and remote
  launch/stop/mail/fleet status, including typed offline refusals and stale-view
  labeling. Ordinary claim forwarding alone is not complete action/mail parity.
- **Native continuity:** enumerate every live seat, helper and retained unresolved
  producer, including borrowed person-owned invocations. Verify exact native
  provenance and current custody; no adoption from a grant or subject name alone.
  Prove restart, disconnect, sleep/wake, reused PID, duplicate receipt and lost
  response recovery. A borrowed invocation stops by detach, never shell kill.
- **Isolation and storage:** final agent dependency/runtime closure has no graph,
  replication worker or Store. Provision private physical paths and credential
  files without symlink aliases or credentials in build outputs. Check available
  disk against the operator's managed floor; do not copy the Mac graph, compact,
  delete or clean state to make the window fit. Keep journals and native records.
- **Rollback:** retain the previous executable/configuration and service state,
  an authoritative source for a fresh fenced graph decision cut, and the ability
  to incorporate all post-transfer journal output. Exercise rollback in an
  isolated fixture, including no-current-cut refusal. Neither an archived graph
  nor graph-plus-journal alone is a safe resumed planner.
- **Measurement readiness:** establish a same-build baseline at normal fleet/seat
  load with the census and counter method below. Record the workload, write/mail
  rates, power state and UI cases before selecting any acceptance windows.

If any gate is unproven, leave existing production ownership unchanged. In
particular, Observe/Stop-only provisioning cannot transfer a whole Mac that needs
new launches, and an agent configuration is not a custody migration receipt.

## One fenced window, not seat-by-seat migration

Use one stable window/operation ID. Pause new fleet-dependent admission and
publication changes at their owning boundaries, without stopping healthy native
processes or denying local emergency controls. Keep accepted work recoverable.

1. **Capture and quiesce the old owner.** Record the final old-planner decision
   cut, publication/source sequences and all pending commands. Fence dispatch and
   prevent old service auto-restart, graph appliers, scheduled publishers and
   replication/reduction workers from becoming competing authorities. Do not
   terminate a process tree containing native seats. Prove the planner cannot
   enter a backend after this fence; process liveness alone proves nothing.
2. **Settle or transfer every custody obligation.** Under that fence, use the
   reviewed transfer tooling to durably import the inventory and unaccepted
   outputs into the executor journal. Match every native generation and borrowed
   invocation; preserve exact cancellation/revocation revisions and receipt IDs.
   Unknown backend entry remains unresolved custody, not a stopped seat. A
   capacity reservation releases only on `cleanup_verified` or `producer_closed`,
   never on a projected terminal phase, logical withdrawal or lost connection.
3. **Install one selected controller.** The fixed home durably records the new
   selection/epoch and its allowed scoped grants; the Mac durably installs the
   same authenticated epoch and imported bindings. Drain old-epoch accepted
   outputs through their fenced receipt path. Require home and executor durable
   acknowledgements of the explicit transfer cut before permitting new dispatch.
   Never reuse an old command to authorize a successor incarnation.
4. **Start only the admitted Mac agent service.** Keep the old full-node planner
   fenced. Verify retained native seats and borrowed shells remain alive and
   attached to the same generations. Confirm the outbound authenticated session,
   journal recovery, exact home pins and a private local performance endpoint.
   Do not import graph execution keys into the home or infer adoption from an
   empty journal. A failed recovery keeps fleet admission closed.
5. **Switch every caller and publisher as one routing cut.** Local native
   attach/read/emergency controls use executor authority; fleet views/actions and
   planning publications use the fixed home. Disable the Mac's obsolete graph
   appliers, fleet publications, full replication and reductions without erasing
   their state. Capture acknowledgements and publication fences so an old timer,
   app launch, login item or service restart cannot silently reactivate them.
6. **Reopen fenced admission and prove parity.** Only after both ends acknowledge
   ownership and routing, admit new work through the home and the host's atomic
   authorization/capacity/native gates. Exercise launch, stop, mail delivery and
   explicit consumer acceptance, attach and status through CLI, drivers, TUI and
   app. Check duplicate delivery/lost-response recovery and borrowed detach.
   Record real results and identities; HTTP success is not consumer acceptance.
7. **Settle before retirement.** Keep the old graph archived **in place**, not
   deleted or copied on the Mac, and its service disabled. Prove the signed
   outbox/receipt drain and retained authority cut, then separately approve any
   former membership or health-expectation retirement. Retiring membership is
   not required merely to start observing the new mode and changes rollback
   requirements. Do not call installation or a quiet process list a completed
   migration.

On session loss or sleep, preserve healthy PTYs and admitted local custody. Keep
local attach/read/emergency control available; refuse new fleet-dependent work
without current authority. Never restart a stale full planner as an availability
fallback. Reconnect and post-wake catch-up must drain bounded retained work with
no duplicate spawn, false exit or lost accepted receipt.

## Rollback decision and ordering

Trigger a hold for uncertain ownership, provenance or durable acknowledgement,
missing caller parity, inability to recover local control, lost accepted output,
or failed CPU acceptance. A hold closes new fleet admission; it does not kill
healthy seats. Capture the failed step and native/counter evidence first.

Before former membership retirement:

1. Stop the home issuing dependent work, durably close/revoke its selected epoch
   and wait for executor installation and all required withdrawal/receipt fences.
   Terminalize undispatched work; resolve backend-entered custody positively.
2. Drain and incorporate all newly accepted journal observations, cancellations,
   revocations, deliveries and acknowledgements into the rollback authority.
   Preserve IDs and replay/tombstone fences; do not truncate or reset the journal.
3. Obtain a **current fenced graph decision cut** from the home or another
   authoritative full replica, including declaration, retirement, placement and
   policy changes that emitted no executor command. Reconcile it with the new
   journal output and native generation inventory under the dispatch fence.
4. Quiesce the agent's remote command entry without killing native processes;
   durably return each custody obligation to the restored full-node owner. Only
   then restore previous routing/services and reopen one planner's admission.
   Verify launch/stop/mail/attach/status and replay before declaring rollback done.

If the epoch cannot be fenced, custody cannot be transferred or no authoritative
current cut exists, remain local-only with fleet admission closed. Do not resume
stale planning. After membership retirement, returning to a full node instead
requires explicit admission and authoritative rehydration before this ownership
sequence; merely starting the old executable against its archived graph is
forbidden. Never clone host keys, reset pins/epochs or declare partial state full.

## CPU acceptance: three complete 600-second windows

The gate is **less than 5 percentage points of one logical CPU core** for total
Mac Fractal-owned background work, not 5% of the machine's total multicore capacity.
The fixed home's CPU increase is reported separately, never subtracted from the
Mac numerator. Offload is not evidence of reduced whole-system work.

- Start after **at least one continuous hour** on the final build, final routing
  and normal running fleet/seat load. Any daemon/agent restart, ownership change
  or workload intervention resets that settling interval. Record normal write,
  mail and seat counts; do not select an artificially idle fleet or pause useful
  work to pass. Preserve comparable same-build pre-cutover baseline evidence.
- Collect **three predeclared, consecutive, non-overlapping 600-second windows**
  on monotonic elapsed time. A sleep, clock/accounting discontinuity, missed
  process lifetime or unobservable restart invalidates the affected sequence;
  report it and establish a new settled sequence, not a shorter denominator.
- Count **daemon + agent + fractalite** and every other Fractal-owned background
  helper, replication worker, publisher/applier and service descendant by actual
  ownership, including any obsolete worker unexpectedly still present. Do not
  match arbitrary argument substrings or charge user workloads merely for being
  spawned by Fractal. Identify each lifetime with PID plus OS birth identity;
  follow births/exits throughout each window and retain final CPU on exit.
  Endpoint own-process CPU excludes children, so census them separately exactly
  once. An agent counter plus that same agent's OS CPU is one process, not two.
- For each window compute `points = 100 × sum(process CPU seconds) / elapsed seconds`.
  CPU time is user plus system across all threads. Each complete window must be
  **strictly below 5.0 points** (under 30 CPU seconds for exactly 600 seconds).
  Report individual totals, per-component contributions and the maximum;
  averaging a failing window with quiet windows is not acceptance. Missing CPU
  data, PID reuse or a counter reset is unknown, not zero.
- Cross-check the agent's private `/v1/performance` cumulative own-process CPU
  delta with UUID/PID, baseline and sample brackets, and correlate its real work
  counters with normal session/native events. Use the daemon's performance work
  counters for attribution, not as a replacement for the OS whole-process census.
  Report observer CPU/overhead and collection intervals; no busy diagnostic loop.
- Run the background gate with UI clients closed, then repeat the same three-window
  method with normal visible TUI/app usage as a **separate reported case**. Show
  interactive client CPU and background CPU separately; never relabel a visible
  client's contribution as a background saving. Capture local PTY latency, home
  RTT and fleet first-view/action latency independently of CPU.
- Exercise a separately recorded sleep/wake and home-disconnect/reconnect case.
  Show peak/cumulative catch-up CPU, backlog/work/bytes and time to normal service,
  then another settled three-window sequence. Do not include sleep in the elapsed
  denominator or hide catch-up behind the steady-state result. Failure or unknown
  evidence leaves the gate unproven and triggers the rollback/hold decision.

The acceptance record includes all six window results for closed/visible-client
cases, post-wake results, builds, cuts, epoch, census completeness, workload and
latency evidence. No result is asserted by this document. Membership retirement,
automatic failover, a different home or a future optimization still requires its
own reviewed decision; a passing CPU gate grants none of those permissions.
