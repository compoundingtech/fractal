# Declared conditions

A condition is a root KDL fleet object. Each daemon evaluates the instances on
its own host every 30 seconds, using local measurements. Owners receive one
notification per instance on entering breach and one on recovery. Samples and intermediate
hold phases do not wake an owner.

```kdl
version 2
condition "fleet/disk" {
  metric "disk.free-percent"
  scope "host"
  below 15
  recover 18
  for "10m"
  recover-for "5m"
  owner "agent/operations"
}
```

This checks up to eight local data filesystems on every host. A host with more than eight filesystems fails strict doctor until explicit paths cover the additional data mounts. The condition list/show reads cached instances; evaluator and overflow diagnostics are shown by doctor. An overflow is reported in doctor; use explicit path declarations for additional data mounts. Add `host "alder"` to
restrict evaluation to that member, or several `host` children for several
members. Each matching host evaluates and notifies independently. Add `path "/srv/data"` to check the filesystem containing one absolute
path.

| Scope | Metrics | Units |
| --- | --- | --- |
| host | `disk.free-percent` | percent |
| host | `disk.free-bytes` | bytes |
| member | `db.size-bytes` | physical database plus WAL bytes |

CPU, process, memory, spend, authored-byte attribution and SLO conditions are a
separate follow-up PR. This disk-first daemon refuses those declarations. Physical
24-hour database growth also remains a later adapter of the shared hourly sampler;
operations must supply a measured last-week baseline before that declaration.

Use exactly one of `above NUMBER` and `below NUMBER`. Crossing is strict;
equality does not enter breach. `recover NUMBER` supplies hysteresis, defaulting
to the breach threshold. `for` is required and is the continuous hold before
entry. Both holds must be at least 60 seconds. `recover-for` defaults to the entry hold.
After recovery an instance waits five minutes before starting another entry hold. A missing reading interrupts a
hold and keeps an established breach. A sampling gap over 75 seconds or a clock
reversal also interrupts holds; machine sleep does not count. After a daemon restart, holds begin again
and an established breach continues without sending another entry message.

Disk probes use native kernel filesystem facts. macOS reads native mount facts
with `getfsstat`. Selectors must name absolute paths on native filesystems.
Network, autofs, FUSE and symlink selectors are unavailable. One disk worker
reads at most 256 native mounts and refreshes a cache every 30 seconds. A kernel
call taking more than ten seconds produces a diagnostic; disk readings expire
after 90 seconds. The worker waits for that call before starting another.
Filesystem IDs deduplicate bind mounts where the kernel reports the same ID;
btrfs subvolumes may have distinct IDs. Database size reads actual database file
and WAL lengths on the background evaluator worker, cached hourly, never on a request path.

`owner` accepts `agent/NAME` or `person/NAME`. Agent notifications identify the condition and instance without pinning the state
claim as evidence, and include the value, threshold, breach start, and series location.
Person breaches appear on their home and clear after recovery. Optional
`link "https://metrics.example.com/series?host={host}"` points to an external
series; `{host}` and `{instance}` are substituted. The default is the local
`st conditions show` command.

Read declarations, instance states and their recent samples with:

```sh
st conditions ls
st conditions show fleet/disk
st conditions ls --json
st doctor
```

These views read cached values and recorded transitions. Kernel probes, folds
and notification retries run in the daemon background. Routine samples and
intermediate hold phases never append claims or wake the reconciler. Each
local instance keeps its eight most recent exact samples; notification values
are formatted to three significant digits. Only entry and recovery append
`condition.state`, on
`condition-instance/SHA256(condition-root)/SHA256(origin)/SHA256(instance-name)` with the
condition root in its fields. Latest-state startup seeks skip historical
claims using the existing subject index. Remote views show the last transition;
only the evaluating daemon has fresh routine values.

The daemon accepts at most 32 declarations, 32 host selectors per declaration,
and eight local instances per condition. Its transition budget is 16 attempted
writes per tick, with deferred transitions retained for later ticks. Local
heads retain eight instances per authenticated origin and at most 256 total per declaration. Their key includes origin, and local covering indexes hold phase and local arrival order. Clear heads are evicted before standing breaches; any omitted or evicted breach leaves a doctor diagnostic. Keyed fleets seek current members directly and prune departed origins; legacy discovery considers up to 256 origins and reports overflow. A changed declaration or membership set re-seeds indexed state, including transitions received before declarations. Discovery resumes across ticks after 128 discovery units (including terminating probes) or one second of work, checked between origins; doctor reports incomplete reconstruction. Each origin considers at most 64 instance namespaces, prioritizing breaches before newer Clear heads; overflow is diagnosed. Each fold/seed write contains at most 50 rows; a tick folds at most 4,000 rows. It does not scan their history. Names, selectors,
links, notification text and sample rings have fixed size limits. Retired
conditions lose their local caches. Vanished instances leave the active tracker
set; an established breach remains as a stale last-known breach within the bounded instance budget, since absence
cannot prove recovery. Current local observations and newer transition heads have priority when retired mounts exhaust that budget. A returning instance restores that breach silently. Inspect the evaluating host to distinguish an absent instance from a stalled observation; withdrawing the declaration or retargeting it to another host clears its alert without inventing measured recovery.

`st doctor` checks the local evaluator heartbeat and sample age. Breaches,
recovering states, invalid declarations and stale data warn; a pending hold or
an instance awaiting its first reading is informational. Other members' breaches
do not fail the local strict doctor check. Person attention labels stale local breach measurements; remote values describe the last transition and carry no freshness assertion. Stale Clear observations do not warn. A panicking tick is caught, reported and retried.

The local notification outbox retries recorded transitions across restarts;
deterministic message identities prevent duplicate wakeups. Startup only queues
own-host transitions accepted in the last hour. Each queued delivery copies its immutable payload, so local trimming of a source claim cannot lose a pending delivery. A failing notification receives
three data or deterministic append-validation attempts, then leaves the queue with a diagnostic visible in doctor for one hour, so
it cannot block other owners. Writer contention keeps the row without consuming attempts; a clean flush clears the transient diagnostic. At most four notifications per owner are selected before the global 16-row limit. A failing row preserves partial deliveries and does not block another owner. Authenticated claim origin and instance identity
must agree with the host fields; received state cannot send as this member.

The per-instance state subject and the condition root field are the seam for
future waits on a breach. This feature does not implement those waits or accept
OpenObserve alert webhooks. Upgrade every fleet daemon before publishing
conditions; daemon before CLI, and upgrade clients before a person-owned
condition is declared. An older registry's unknown-subject-family rejection is
retried once when its schema digest changes; genuinely invalid subjects remain
invalid. Operations publishes declarations after confirming every member runs
the new schema. Existing disk and watchdog missions remain until a real host
has fired and recovered and operations agrees to retire them.

The complete conditions endpoint remains bounded to 32 declarations x 256 heads (8,192 instances). Pagination is deferred; the capacity cost fixture measures 1,024 remote breached instances with tenfold history independently of the existing history-growth guard. Deployments with large caches should account for the full response size. Condition state is a full replacement record. Locally authored transitions are
copied atomically into the existing local observation log and exported as
`condition.state` OTLP logs through `[observations.otlp]`, with original fields
and timestamps. Received peer transitions are not exported again. Idempotent
retries create no second source observation. Collector failures leave the
existing export cursor unchanged and use its bounded backoff; local retention
may eventually discard unexported history and the exporter reports gaps.
Without configured OTLP, full external history is unavailable.

**Local-trim dependency is pending on main 8f4f47e0b.** The target is current state
plus three previous transitions per authenticated instance, trimmed locally with
permanent compact deleted IDs, without checkpoint quorum. The disk replacement
must not be queued while that requirement is unmet. A checkpoint-only rule is
not an acceptable fallback and this PR adds none. Intake's coordination-free
retention review still requires final admission/authority, immutable wire
positions, signed payload removal and N-1/checkpoint interoperation contracts.
A conditions-specific DELETE must not bypass those requirements.

The smallest bounded stand-in proposed for review is a host-owned four-state
logical projection using the shared canonical order, plus the already copied
pending notification payloads and OTLP source. It can bound a view/cache without
changing transport or inventing a second replication protocol. It does not
reclaim source claims, signed envelopes or rich references, and cannot establish
the requested retained graph-byte target. The source payload/negative-set adapter
must use the reviewed retention mechanism before this feature is approved.
The proposed local projection remains a proposal until its exact shared-hunk
contract is checked with the retention owner.

Normal notifications identify the condition/instance in text instead of citing
an immutable state claim as evidence. Their existing durable message retention
is unchanged. Explicit person pins and admission/operation references keep their
normal semantics. Report four-state bytes, permanent negative IDs, notification
messages, pending outbox, signed transport containers and replicated daily write
volume separately. A kilobyte figure for a cache must not be reported as physical
graph storage after trim. Production CPU/SQL claims require Operations'
[finite hetz measurement](conditions-measurement.md); local or hosted fixtures do
not substitute for those numbers.

Restarted local instances restore directly from their indexed state subjects while remote discovery continues. Anchored fleets reconstruct keyed current members; unkeyed legacy peers are outside that reconstruction contract (their new transitions are still folded). Departed members lose standing alerts without a recovery claim. Capacity diagnostics aggregate up to eight causes and stay visible until a new full seed starts: a lost remote breach might not transition again, so a clean fold cannot prove recovery. Narrow or replace the declaration, then verify discovery; a daemon restart also forces reconstruction and resets the warning. Replacing a declaration with identical text does not change its digest. Notification flushes materialize at most the oldest 256 queue rows and deliver at most four per owner/16 total; older queued transitions are retained, so owner fairness applies within that bounded window. Unexpected non-validation storage errors keep their rows indefinitely with diagnostics; no broad retry limit discards them.

Retry rows currently have no cooldown: persistent failures can occupy all selected owner slots and delay younger owners. Per-row backoff and counted per-declaration capacity causes are follow-ups. A tick that repeatedly panics can restart large discovery rather than make progress; its watchdog diagnostic remains visible.
