# Condition-state local retention contract (pending integration)

This is the disk-first consumer of the coordination-free retention design, not a
second trim protocol. Pinned main: `8f4f47e0bb059ee4d1dab98bc0c5633326fff2c5`.
The local quorum-free payload/negative-set API is absent at that revision.
Speed confirmed in message/8b4f833734c294fd that there is no usable dependency PR
or implemented trim API. The current source-only design is
`doc/fleet/smalltalk/speed/2026-10-11/coordination-free-retention-design-v3@9f5e695ade76c9c3869d2e3a1cc3f3762468c3dd5f0e1fff0fe89ddbb3bc52c8`,
carrying Intake's v2 review
`doc/fleet/smalltalk-intake/2026-10-11/retention-design-v2-review@b25afe4b26753c5a62b5577af63f33dccb3acbd32d34f645e004b67f9737547f`.
V3 is still under review and certifies no family eligible for removal. Signer-key
settlement, permanent authenticated nonce/authority/admission fences, signed
payload removal, immutable positions and real N-1 exchange remain prerequisites.
The closed majority-trim #2289 is not this dependency.

Operations interprets Nathan's kilobyte requirement as retained condition-state
history **per instance after local trim**, without checkpoint quorum. Count
permanent deleted IDs, messages, outbox, transport containers and daily replicated
writes separately. It is not a bound on all bytes a condition ever causes.
This interpretation must be in PR 1 so Nathan can correct it in review.

| Contract component | Required behavior |
| --- | --- |
| Domain/key | `(condition root, authenticated writer origin, instance name)`. The subject hashes all three; `host` must equal authenticated origin. Host selectors affect evaluation, not who may rewrite another origin. |
| Winner | Greatest shared canonical key: numeric accepted time, origin, replica sequence, batch ID, immutable wire position, claim ID. Local `store_index` is navigation only. |
| Full value | Each `condition.state` replaces all its fields. Missing optional `breach_since`, `value`, notification text or transition means absence, not a value inherited from an older record. Metric, thresholds, owner, phase, phase time, measurement time and sample ring belong to the full value. The root declaration remains its own durable intent. |
| Keep | Current full state plus three previous transitions, four records per authenticated instance in canonical order. Late arrival must not displace a newer state by arrival index. |
| Eligibility | A dominated source is removable only when the retained winners are finally valid under the reviewed admission contract. Unknown, provisional or invalid winner/authority refuses payload trim. State subjects are system-only; that fact alone does not prove generic admission finality. |
| Permanent negatives | Commit compact deleted IDs and required authority/idempotency/reference evidence including permanent authenticated nonce, authority, operation and cardinality fences before payload removal, atomically; preserve monotonically across restart, adoption and old replay. Bare unauthenticated peer IDs never authorize deletion. |
| References | Normal notifications no longer pin transition IDs as immutable evidence. Explicit person/agent evidence, repair/verdict targets, predecessor edges, operation/request identity and projection references must retain the required negative or rich metadata. Do not weaken their current semantics. |
| Pending delivery | Each local outbox row copies immutable source ID, authenticated origin, root, and notification body. Removal of the source payload must not erase a pending notification. Deterministic message identity survives retry. Message retention stays unchanged. |
| OTel | Atomically copy each newly authored transition into the existing local observation log with the same fields and timestamp. Existing OTLP cursor/backoff owns delivery; no exporter request runs inside a writer or evaluation request path. Peer transitions are not exported twice. Existing local observation retention can report export gaps. |
| Signed container | Removing a logical source cannot rewrite an immutable signed envelope. Persist wire positions before removing any earlier row of a batch. The reviewed transport adapter must preserve verification and old replay protection. |
| Mixed fleet | Every daemon understands condition facts before publication. The generic local trim mode must define checkpoint coexistence and old-binary exchange in both directions. No silent checkpoint fallback, no inventory-equality claim over intentionally unequal history. |
| Bounds | Per-pass row, byte and time limits; no writer transaction over 100 ms. Scope work to changed instance keys, not all historical claims. A pending-key backlog must itself have a measured bound. |

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
| Pending delivery outbox | Immutable payload copied once per queued local transition. The queue is uncapped, so an outage has no finite retained-byte bound. Only the oldest 256 rows are considered per tick. |
| OTel/local export log | One local observation per authored transition. Existing local log retention, cursor and exporter backoff apply; outage gaps remain possible under that retention. It is not replicated condition history. |
| Replicated writes and containers | Source transition plus ordinary notifications and transport overhead. No write-volume reduction is claimed. Each replica receives other writers' transitions too. `claims.body`, `replica_records.raw`, signed envelopes and their indexes must be counted separately by the trim dependency. |

The old 100–200 MB/day statement must therefore disappear as a promise of
permanently accumulated **retained condition-state bodies** only after physical
local trim is implemented and measured. It remains an honest approximate daily
write estimate. No deletion has occurred in this draft and no kilobyte physical
retention result is claimed. Disk-first's planned two declarations have a much
smaller instance population, but baseline thresholds and actual counts belong to
Operations and must not be invented here.
