# SLO breach obligations and missions

This first source change adds the canonical transition and ordinary mission-opening
seams. It starts no evaluator, timer, collector, profile, network publication or
runtime writer. The standing owner is `agent/fleet/smalltalk/slo`; the existing
implementation claim remains `01a1413009c97c3096561687d4993f8b/implement-source`.
The source anchor is `5ab05fb7a3b28cf6ecb4f99ec17583b76078f731`.

Nathan accepted `ask-d417200c75d57ef07ea5738b844c59e5`: **“Yes, within these
ceilings.”** The posted ceilings are 64 objective slots per member, two mission
openings per minute, 16 claims and 32 KiB per group, and design maxima of about
32,000 claims and 60 MiB per member per day. Those are design ceilings, not measured
cost or activation approval. There is no objective or threshold mutation here.
The accepted direction remains combined daemon plus actual workers at most one
core per member and positively eligible full-hour idle CPU at most 0.5 core.

## Authority and episode identity

`slo.breach-transition` is system-only, append-only and envelope-signed by the
observing member. The actor is `daemon/runtime`; the record's logical member must
match the claim origin and subject, including the reading's member/epoch/sequence
and predecessor cut. Existing envelope admission binds that origin
to the authenticated writer. Each node validates the bounded typed record before
projection; an origin signature attests provenance, not independent OS truth.
Older registries retain the new kind as UNKNOWN and do not publish its facts.

An episode key hashes logical fleet, member, objective and the predecessor recovery
claim. It does not include a credential, PID or build. Ordinary credential rotation
and daemon restart therefore do not mint another episode. A later episode needs the
prior recovery, a strictly later qualifying window, and the prior episode's mission
link. Conflicting same-event contents, conflicting equal-ordinal heads, missing
lineage or incomplete first sync fail closed. A restore with uncertain lineage
must not be activated as a fresh logical member. This seam does not certify restore
lineage or invent an independent genesis certificate.

Records are transitions only: pending first miss, dispatch/owner, first owner block,
first owner unblock and recovery. There is no heartbeat or sample claim. A missed
reading carries objective version, target, units, population, exact window,
coverage, source cut and a bounded existing counter cut. Missing and partial
coverage is UNKNOWN. Recovery needs the same effective objective and a complete
healthy window after the onset; it does not complete or cancel an admitted fix.
A busy member with no eligible idle hour remains unmeasured for that objective.
Maintenance, heartbeat, observation and anti-entropy CPU always count. Foreground
activity, sync catch-up, deploy or unknown eligibility prevents an idle PASS.

CPU records include bounded actual process identities, roles, births and monotonic
counter cuts. Both miss and recovery must cover the full window. Counter resets,
duplicate identities, unknown cohorts, missing births and uncertainty crossing a
target boundary are rejected. The origin must still establish that this is the
complete actual cohort. No existing 60-second process sample or worker lifetime
counter is promoted into an hour of combined coverage.

## Preparation and ordinary work admission

The three shared projections are `slo_pending`, `slo_episode_heads` and
`slo_episode_events`. Event selection uses minimum claim id and preserves a conflict
witness and maximum-ordinal witness; it does not depend on receipt order or store
indexes. Explicit record repair refreshes that episode from its canonical sources;
an objective index selects the surviving head without replaying other episodes. Opening a store only
creates empty tables and indexes. Explicit import/replay reduces canonical sources;
replicas never originate missions.

Preparation reads at most two oldest local pending obligations and parses the
fixed two-step mission outside the writer. Its caller supplies a bounded absolute
workspace explicitly; this seam does not invent a relative checkout or inspect the
host filesystem. Publication revalidates the exact onset
claim, episode head and member credential under the existing writer, then opens one
ordinary owner-assigned mission in one transaction. The default owner is
`agent/fleet/smalltalk/speed`; an explicit owner is retained. Owner lookup is a
point read. The source consumes the existing membership certificate only when its
fleet generation is current; it does not trigger a membership fold or snapshot
build. The same generation, fleet binding, first-sync state and projection health
are revalidated by point reads under the writer. Normal seat capacity and queue admission still apply to execution;
this code never calls `start_work` as the owner or chooses a person/CoS fallback.
An absent owner remains named pending work. Later owner flaps use ordinary owner
facts, with at most one block and first-unblock receipt per episode.

The mission id is `slo/EPISODE`. Its agentless dispatch step uses the ordinary Field
gate `dispatch_state is ready` on `resource/slo-breach/EPISODE`; its fix depends on
that step. The fact is derived on each node by the episode reducer, never a
`resource.observed` claim. Gate reads use a short source snapshot and fixed indexed
point reads. The real work selector and claim path also require the intrinsic
positive execution-admission receipt for a newly ready fix.

If recovery precedes dispatch, the one mission still gets created, with
`execution_ready=false` intrinsically stored in its dispatch transition and an
ordinary explanatory cancellation. No prefix containing only mission, onset and
dispatch claims can become executable while waiting for recovery or cancellation.
An already admitted fix keeps its positive receipt after recovery or a later breach;
its active claim, substantive work and history are preserved.

## Bounds, replay and retention

There are at most 64 objective heads and one outstanding undispatched obligation
per objective. Mission openings are capped at two per rolling minute and a stricter
700 per rolling day. Later obligations remain durably pending; recovery does not
let an undispatched episode disappear. There is no commit-triggered evaluator.

A record is at most 8 KiB; its reading/counter summary is at most 1 KiB. A receipt
has a conservative signed/base64 framing bound of 16 KiB. A mission-opening group
is at most 16 claims and 32 KiB with signature, batch and base64 allowances. At most
three independent receipts surround an opening group (onset, recovery, first
unblock). Thus 700 groups plus up to 64 carried obligations fit below the posted
60 MiB/day and 32,000 claims/day ceilings. This arithmetic is not writer, replica
CPU, memory or elapsed-time proof. The actual signed-envelope control checks wire
size; the first hosted cost control measures full writer and replica holds plus test-process
CPU deltas. Its cost run must use one test thread; those deltas include both the
calling thread and the actual store writer thread. They do not establish production
or hourly idle coverage.

Checkpoint rules version 16 includes all three tables in replay and shared graph
proofs. It keeps onset, dispatch, owner and recovery fences, including their signed
payloads and a conflicting body where needed. Only redundant receipts of recovered
episodes can compact once the recovery is at least seven days old and existing
reference/whole-envelope guards permit it. There is no seven-day total-storage
promise: the approved retained fences and ordinary mission/work history persist.
No startup history scan, local transition authority or destructive migration is
added. Roll forward; do not downgrade a fleet checkpoint participant to different
rules and assume it can trim under the new digest.

## Qualification and remaining integrations

Every new control is **UNRUN** until the exact delta is admitted by Intake and an
ordinary hosted run reports results. Local Cargo, native runs, load, host probes,
profilers, runtime activation and deployment are outside this implementation claim's
authority. The writer-hold number must be reviewed next to st2 #3080's measurement;
shared-replica CPU and worst-case transition cost are later GO/activation gates.

The prefix control enumerates every subset of the finite known/new claim group,
forward and reverse receipt order, current and old registries, checkpoint replay
and reopen. It calls the actual Field evaluator, work selector and normal claim.
The separate signed-exchange control checks old-registry retention and upgrade.
Missing ordinary prerequisites may defer projection; they must remain non-executable.
Neither fixture constitutes a deployment or independent measurement of production.

The future producer must reuse existing bounded SLO evaluations and #3074 accounting,
with #2930/#3080 writer observability first. This PR does not adopt those unmerged
heads or infer their approval. Fleet snapshots extend #2916 `slo.list/history` only
after the separate PUBLIC CONTRACT review of fields, limits and cursors. No fleet
selector, public route, policy/config or new sampling framework is in this PR.
