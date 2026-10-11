# Condition-state local retention contract (pending integration)

This is the disk-first consumer of the coordination-free retention design, not a
second trim protocol. Pinned main: `8f4f47e0bb059ee4d1dab98bc0c5633326fff2c5`.
The local quorum-free payload/negative-set API is absent at that revision.
Speed confirmed in message/739b2d3c9f4e188a that there is still no usable
local-trim API, certified removal family or PR 3–6 authority. The consolidated
source-only design is
`doc/fleet/smalltalk/speed/2026-10-11/coordination-free-retention-design-v6@2b84ea59d6afd86476aedb27b9e65db4aa68a2443f0ddb1a5ea5a04770092098`.
Intake read v4 without a new source objection. V6 carries dated retired-set
scale and a repaired-original reader inconsistency; it supplies no implemented
API, family certification or payload-removal approval. The design distinguishes
advisory signature verdicts from the
actual live fold and requires the union of every reader's winner set, complete
carrier closure, permanent authenticated nonce/admission evidence, exact-ID/body
reference closure, immutable positions and real N-1 exchange. No family is
certified eligible. The closed majority-trim #2289 is not this dependency.

Operations interprets Nathan's kilobyte requirement as retained condition-state
history **per instance after local trim**, without checkpoint quorum. Count
permanent deleted IDs, messages, outbox, transport containers and daily replicated
writes separately. It is not a bound on all bytes a condition ever causes.
This interpretation must be in PR 1 so Nathan can correct it in review.

| Contract component | Required behavior |
| --- | --- |
| Domain/key | `(condition root, authenticated writer origin, instance name)`. The subject hashes all three; `host` must equal authenticated origin. Host selectors affect evaluation, not who may rewrite another origin. |
| Winner | Preserve the union of all readers’ winning claims/carriers, not just the newest condition head. Within a reader’s logical key, use the greatest shared canonical key: numeric accepted time, origin, replica sequence, batch ID, immutable wire position, claim ID. Local `store_index` is navigation only. |
| Full value | Each `condition.state` replaces all its fields. Missing optional `breach_since`, `value`, notification text or transition means absence, not a value inherited from an older record. Metric, thresholds, owner, phase, phase time, measurement time and sample ring belong to the full value. The root declaration remains its own durable intent. |
| Keep | Current full state plus three previous transitions, four records per authenticated instance in canonical order. Late arrival must not displace a newer state by arrival index. |
| Eligibility | First trace whether each actual reader uses advisory verdicts. A reader that does needs its reviewed signer-settlement contract. A live fold that does not needs stable semantic outputs under all admissible arrivals and unequal retained subsets; do not invent a verdict filter. Every indispensable field carrier, reference and fence must remain represented. Unknown authority, missing body/reference closure or unqualified transport refuses payload trim. System-only subjects or a green advisory verdict alone do not prove eligibility. |
| Permanent negatives | Commit compact deleted IDs and required authority/idempotency/reference evidence including permanent authenticated nonce, authority, operation and cardinality fences before payload removal, atomically; preserve monotonically across restart, adoption and old replay. Carried non-verifying signature rows must retain the same nonce-predicate participation and authenticated source provenance; no verifies-only policy or arrival-selected source is introduced. Full-history nonce retention differs from agreed-trim forgetfulness and must be disclosed by the eventual implementation. Bare unauthenticated peer IDs never authorize deletion. |
| References | Normal notifications no longer pin transition IDs as immutable evidence. Explicit evidence, exact-ID/body reads, historical cuts, repair targets, predecessor edges, operation/request identity, custom references and projection carriers need their payload or a verified compatible representation. Compact IDs alone do not satisfy a future historical-body request. Do not weaken their current semantics. |
| Pending delivery | Each local outbox row copies immutable source ID, authenticated origin, root, and notification body. Removal of the source payload must not erase a pending notification. Deterministic message identity survives retry. Message retention stays unchanged. |
| OTel | Atomically copy each newly authored transition into the existing local observation log with the same fields and timestamp. Existing OTLP cursor/backoff owns delivery; no exporter request runs inside a writer or evaluation request path. Peer transitions are not exported twice. Existing local observation retention can report export gaps. |
| Signed container | Removing a logical source cannot rewrite an immutable signed envelope. Persist wire positions before removing any earlier row of a batch. The reviewed transport adapter must preserve verification and old replay protection. |
| Mixed fleet | Every daemon understands condition facts before publication. The generic local trim mode must define checkpoint coexistence and old-binary exchange in both directions. No silent checkpoint fallback, no inventory-equality claim over intentionally unequal history. |
| Bounds | Per-pass row, byte and time limits; no writer transaction over 100 ms. Scope work to changed instance keys, not all historical claims. A pending-key backlog must itself have a measured bound. |

## Reader and reference closure still required

The direct restart reader uses `condition_tracker` and canonical `latest_claim`;
condition discovery/fold, cached views, attention and notification delivery have
separate retained-head/payload requirements. Generic `latest_actual_at` also
reads these subjects: its `fold_latest_values` clears registered fields for
`StateTransition`, then merges fields. V6 records that this actual-state reader
still includes repaired originals while other readers exclude them. That is a
source-reported inconsistency at the generic owner's later pin, not an
independently reproduced condition ordering failure or observed production
incident. The condition consumer does not repair it or gain trim eligibility. That supports full-value absence for
registered condition fields, but is not proof of closure for every cross-kind,
custom or historical-cut reader. The actual reader union and full projection
must agree with untrimmed replay after shuffled arrivals and different trim cuts.

A pending delivery copy and OTLP copy close those consumers only. They do not
make arbitrary future `claim_by_id`, evidence, mission-input or repair requests
safe after deleting a body. Until the reviewed generic contract closes those
paths, current plus three is the requested tail, not a certified deletion set.

## Reader table and fold-before-trim fence

| Reader | Source it needs | Required retained winner or closure |
| --- | --- | --- |
| Heads, views, attention and Home | Newest body per authenticated origin/instance | Same-origin full replacement in canonical order; preserve every reader winner. |
| Restart `condition_tracker` | Canonical latest `condition.state` via indexed `latest_claim` | Retained newest full body, not merely a head pointing at a retired row. |
| `fold_condition_heads` | Every newly arrived row past `condition_heads_cursor` | Keep the entire unfolded tail until head and delivery closure commit. |
| `st conditions show` sample ring | Values inside the newest body plus local routine overlay | Ring fields replace as a whole; old claims do not supply missing samples. |
| Generic `latest_actual_at` | Registered fields of `StateTransition` | Full replacement clears registered old fields; preserve unknown/custom/reference carriers until audited. |
| Local observation / OTLP | Atomic copy made at new own append | Independent copy with existing age/count retention and export cursor; received peers are not copied. |
| Pending delivery | Immutable outbox copy made by seed/fold | Copy must commit before retiring a source that still requires a wake. |
| Exact-ID/body, historical cut, evidence, repair and custom readers | Referenced source payload | Keep it or a reviewed compatible authenticated representation. Unknown future-reference closure refuses deletion. |

No current application reader uses the three previous transitions. That tail is
Nathan's retention policy, not an additional reader winner or an eligibility
proof. For this family there is no merge of different authenticated origins:
a later same-origin complete value replaces its instance's current value,
subject to every reader/fence, actual verdict dependency and generic nonce/
authority/transport contract. Repaired-away records and replacement-kind rules
must remain outside normal winner selection under the generic reviewed contract.

The trim adapter must not remove a `condition.state` row beyond
`condition_heads_cursor`, or an own agent-owner transition in the one-hour wake
window whose required outbox copy has not committed. Head/outbox writes and
cursor advance share one fold transaction. The atomic observation copy alone
is insufficient to authorize deleting an unfolded delivery source. A cached
source that vanishes between read and fold is skipped rather than poisoning the
whole page; this tolerance does not grant permission to delete the unfolded tail.

## Smallest bounded stand-in proposed

A local four-transition projection can reuse the existing subject/kind canonical
index and condition head capacity (32 declarations, eight local instances and
256 heads per declaration). Copy the four newest full bodies for admitted heads,
prune entries when a head/declaration is removed, and expose the same four bodies.
Pending notification payloads and the existing OTLP path already avoid depending
on an old state payload for delivery or historical export.

This stand-in is bounded logical state, **not payload reclamation**. It must be
reported that way. It does not create tombstones or erase claims/envelopes and
therefore cannot satisfy the retained physical graph-byte requirement by itself.
Do not queue either replacement until the exact reviewed local trim adapter
reclaims source payloads and the controls below pass. This proposal has not been applied or validated as production code.

## Required consumer controls

- Many enter/recover transitions retain exactly the four canonical records for
  each independent origin/instance; initial and late-arriving older states agree.
- A restart restores the standing breach and its `breach_since` without a second
  entry wake. Remote reconstruction agrees after differently sized trim passes.
- A copied pending delivery remains deliverable after its source payload is gone;
  partial notification retries and replay remain idempotent.
- Every authored transition reaches the fake OTLP collector; an outage preserves
  the cursor and a later accepted batch carries all source bodies unchanged.
- Late old history, authority changes and a retry of a retired operation cannot
  resurrect or repeat an effect. Unknown/provisional validity refuses trim.
- Faults before/after the negative-fact commit and payload removal; interrupted
  immutable-position backfill; no partial batch reordering.
- N-1 exchanges in both directions, late writer, checkpoint coexistence and
  monotone negatives, using the generic owner's reviewed controls.
- Report real per-table/index bytes on a scratch store and separate retained
  state, negative IDs, messages, outbox, transport payloads and daily writes.

## Separate scale counts (arithmetic, not measured trim results)

The prior full-capacity estimate was two transitions per seven minutes, about
411 per instance per day. At 32 declarations times eight local instances this
is about 105,216 newly authored transitions per host per day. With illustrative
1–2 KiB source bodies, that is about 103–206 MiB/day of source body writes per
writer; signed transport, indexes and duplicate repair payloads add bytes.
Local trim bounds retained state; it does not reduce these daily writes.

| Category | Proposed bound or remaining count |
| --- | --- |
| Retained state bodies | Four per authenticated instance: illustrative 4–8 KiB/instance. At 256 local instances, 1–2 MiB; at the full 8,192 remote-head capacity, 32–64 MiB per replica. This is a proposed bound, not measured reclamation. |
| Compact retired IDs | At full local flapping capacity, about 105,216 new IDs/day/writer. Raw 32-byte hashes alone would be 3,366,912 bytes/day; hex, indexes and authenticated evidence add more. The actual format is unqualified. |
| Nonce and admission fences | Permanent population includes every signed claim, not just trimmed transitions. Exact row/index bytes remain a generic-retention input; no net savings promised. |
| Notification messages | Up to one ordinary durable agent message per transition, about 105,216 messages/day/writer at that extreme. Additional normal delivery/read claims remain unchanged. Person attention is projected from the current breach. Message retention is unchanged. |
| Pending delivery outbox | Immutable payload copied once per queued local transition. The queue keeps at most 1,024 copied rows after enqueue; excess oldest rows and retry counters are discarded with a one-hour doctor diagnostic. Loss of queued wakes is explicit. Only the oldest 256 rows are considered per tick; each copied body still has the condition claim field bounds. |
| OTel/local export log | One local observation per authored transition. Copies are written even without OTLP. Existing hourly trimming defaults to seven days and at most 20,000 rows per subject/kind; the newest row of each subject/kind always remains. There is no global byte cap or lifetime-key cap. At 411 transitions/day, seven-day retention is about 2,900 rows (roughly 4.2 MiB of payload at 1.5 KiB/row) per continuously flapping instance, or about 1.05 GiB at 256 instances, before indexes and hourly sweep overshoot. Export cursor/backoff and possible outage gaps remain unchanged. It is not replicated condition history. |
| Replicated writes and containers | Source transition plus ordinary notifications and transport overhead. No write-volume reduction is claimed. Each replica receives other writers' transitions too. `claims.body`, `replica_records.raw`, signed envelopes and their indexes must be counted separately by the trim dependency. |

The generic design's observed rich retired-claim cost is about **865 bytes per
claim**, not 32 bytes. At 411 retirements/day/instance that is about 355,515 bytes
(347 KiB) per instance per day permanently, or roughly 86.8 MiB/day at 256 local
instances, before the nonce ledger and other indexes. Depending on compactness,
the review estimates roughly 100–350 KiB/instance/day, about 36–125 MiB per year.
These are extrapolations from the existing negative-set measurement, not a
measured future compact ledger. Its authenticated format is still unqualified.
Keep this next to the 105,216/day figure: a kilobyte state tail excludes material
permanent growth and requires Nathan's informed interpretation in review.

Before source trim exists, storage **increases**: every new transition also adds
an observation copy, even with no configured collector, and a temporary outbox
copy for agent delivery. Local observation age/count trimming is independent of
replicated payload removal. Doctor warns when declarations have no configured
`observations.otlp`; the warning is configuration, not collector-health proof.

The old 100–200 MB/day statement must therefore disappear as a promise of
permanently accumulated **retained condition-state bodies** only after physical
local trim is implemented and measured. It remains an honest approximate daily
write estimate. No deletion has occurred in this draft and no kilobyte physical
retention result is claimed. Disk-first's planned two declarations have a much
smaller instance population, but baseline thresholds and actual counts belong to
Operations and must not be invented here.

The v6 dated single-node snapshot (2026-10-11 17:01Z) reports 615,894
retired-claim rows and 605,872 dropped-envelope rows, with seven table/index
objects totaling 527,216,640 bytes (~856 bytes per retired claim including
envelope negatives). These are retained negative rows, not domain-deleted entity
counts, a fleet total, payload savings or family-eligibility evidence. Full
build/tool binding remains PARTIAL: daemon/tool byte identities are missing and
the elapsed-origin shift is unqualified. Compact nonce-ledger size remains
UNKNOWN. No extra production read follows from carrying these supplied numbers.

The OTLP doctor warning reflects a daemon runtime flag set during `run_up`.
A standalone `Store` or a doctor served before configuration initialization
defaults to not configured and can warn even if another configuration would
provide a collector. Interpret it with that daemon's configuration; it is not
collector-health evidence.
