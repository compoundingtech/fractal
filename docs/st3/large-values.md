# Large SQLite values

New storage work must avoid unbounded large inline SQLite objects. Design an
incremental, bounded physical representation or a reference with equivalent
logical semantics. This rule covers variable columns **and index keys**: claim
bodies and predecessors, raw signed envelopes, blobs, payloads, metadata,
identifiers, JSON, aggregate rows, and their derived values. An INTEGER-keyed row
can still contain a variable object. TEXT affinity, a route limit, or a bounded
number of rows does not establish a bound on its encoded bytes or statement cost.

There is no accepted numeric definition of “large” yet. This change introduces no
input limit, size threshold, runtime guard, schema, storage format, or rejection
of previously valid data. All valid current and admitted historical values remain
in scope. Existing producer-specific limits remain in effect and must not be
removed or generalized to other producers. Chunk size is a physical format
choice to qualify later, not a logical input cap.

## Inventory and transitional exceptions

[large-value-sites.json](large-value-sites.json) is the machine-readable source
inventory. Each `sites` entry names a SQL template, unresolved execution/bind
sink, or included source; its adjacent Rust `allow-for-now` comment names the
same stable ID, owning **producer module**, and a breakup/reference TODO.
IDs use `lv-h<12hex>` / `flow-h<12hex>`: the alphabetic hash tag distinguishes
source fingerprints from numbered decision citations. Full content fingerprints
remain separate and unchanged by that namespace.
`declarations` records every column and index expression found by the declared
lexical catalog, including types that look small. A template that cannot be
resolved stays explicitly unresolved. No catalog entry proves a value's size.
`flows` additionally binds the whole non-comment function source of direct
append, local sealing, replication admission, blob promotion, backup import and
physical/historical copy, restore and trim entry points. These paths can bypass
client extractor limits while still enforcing their own authority rules.

These markers record transitional debt. They neither grant permanent permission
nor transfer source ownership. The source module owns the TODO until its normal
maintainer changes the representation through review. A changed statement,
function, owner, TODO or dependency set requires an explicit inventory review;
removing a site requires removing its exception. There is no “accept all” or
regeneration command in the check. The first inventory deliberately keeps
conservative candidates instead of silently treating an ambiguous call as safe.

The inventory uses one complete entry per physical line (including nested column
metadata), with site records ordered by `(path, id)` and manual flows by
`(path, function)`. Declarations follow deterministic source-path/statement order,
with table, added-column and index groups within each template. Keep this layout
when reviewing changes so unrelated additions can merge as independent records.

Use the read-only maintenance helper after a source edit:

```sh
python3 scripts/propose-sqlite-large-values > /tmp/sqlite-value-proposal.json
```

It prints new, changed and removed IDs with exact fingerprints, proposed owner/TODO
metadata, declaration changes and stale manual flows. An existing adjacent marker
is the only anchor used to pair a changed ID; without it a replacement stays an
addition plus removal, not a guessed match. The output is **a proposal**, not an
accepted inventory. Review the actual producer and value flow, owner and concrete
TODO, then manually reconcile the selected records, adjacent comments and column
metadata. Run the guard afterward. The helper edits no file, cannot make a failing
guard pass, and has no accept/write switch. Unchanged source records are not
rewritten merely because line numbers moved.

TODO codes mean the following concrete work:

| TODO | Required follow-up |
| --- | --- |
| `chunk-or-reference-variable-columns` | Identify all variable columns and index keys; choose bounded physical units or references and preserve query semantics. |
| `prepare-body-and-predecessor-values` | Prepare body/dependency data outside the writer, then publish a validated short atomic result; preserve canonical values and authority. |
| `chunk-or-reference-exact-wire-values` | Preserve exact signed/raw bytes, identities and historical decoding while changing physical value storage. |
| `stage-generation-and-keyed-reclamation` | Prepare exact generations/closure outside the writer, validate the visibility switch, then reclaim by certified dependency keys. |
| `trace-statement-and-value-representation` | Resolve the dynamic statement/bind/import source and connect its actual values to a reviewed representation. |
| `bound-or-reference-values-and-reclamation` | Establish the producer's complete value/key and cleanup costs, then remove the unbounded inline dependency. |

A new exception's linked review must explain its actual producer, value
columns/keys, reason for remaining inline, and next representation or cleanup
step. Record the responsible maintainer and that explanation in the review,
linked to the inventory ID. The marker's `owner` is a deterministic source-module
code and its `TODO` is a deterministic work-category code; the guard requires
those codes to match the source and does not accept free-form reviewer names or
rationale in their place. It checks neither the linked review's existence nor
its approval. Owner/TODO strings alone are not cost, correctness, or storage
approval; do not replace a TODO with a permanent exemption.

### Cold runtime reduction inventory (PR #1706)

The `fractal_store::store` maintainer owns `lv-h5911198f6cf3` and `lv-ha1002d83b7ac`:
the first selects existing integer claim identities in descending canonical order, and the
second hydrates one existing kind/body at a time until consumed fields resolve. Claim bodies
and canonical subject/time keys remain inline; a decoded-row bound is not a byte-size bound.
The existing reference/chunking representation work remains required. The obsolete full-history
source reader `lv-hcd8a9be75153` is removed in favor of the existing indexed per-origin reader.

The `fractal_store::store::tests` maintainer owns `lv-h13b02e7126c4` and `lv-hf8af949bb730`.
These rewrite isolated in-memory claim fixtures to exercise historical sparse/legacy records;
they do not add a production ingestion path. Body/predecessor preparation remains the owning
module's representation work, with no new size guarantee or admission relaxation.

### Person rename and review reassignment inventory review

Owner review for [PR #1185](https://github.com/compoundingtech/fractal/pull/1185):
the PR-owning fleet seat reviewed the following transitional exceptions. The
producer modules named by the markers retain responsibility for their TODOs.
These accept existing inline representations, not a value-size or cost bound.

| Inventory ID | Producer and reviewed value flow | Reason and next step |
| --- | --- | --- |
| `lv-h17335d660f3b` | `fractal_store::store::attention` prepares the effective-reviewer pending-request query and hydrates request bodies and canonical episode timestamps. | Preserve exact episode and canonical ordering; trace composed SQL and reference/chunk variable request values. |
| `lv-h9b347ba24aa6` | `fractal_evidence::store::evidence` reads request ID, owner and effective reviewer through the generated reviewer fold. | Trace composed query and reference representation while preserving exact request identity and historical authority. |
| `lv-h1a7e1457e965` | `fractal_store::store::human_review_reassignment` reads the effective reviewer for one request, including canonical repair and receipt selection. | Trace generated SQL and move variable reviewer/request keys to a reviewed reference representation. |
| `lv-h12810b4d74c0` | `fractal_store::store::person_rename` hydrates a receipt by idempotency key. | Preserve original report and actor/people validation; prepare a body/reference representation with identical replay semantics. |
| `lv-h273f7a4b7c06` | `fractal_store::store::person_rename` reads the canonical receipt destination by old-person subject. | Both reducers must choose the same winner; reference variable person keys without changing canonical ordering. |
| `lv-hfbb687ae03ed` | `fractal_store::store::person_work` evaluates receipt-winner SQL for a reassignment claim. | Trace composed SQL and variable claim/person keys toward a reference representation without changing winner selection. |
| `lv-hf6f5b79d7be9` | `fractal_store::store::person_work` updates assignee and its index key, fenced by step and previous person. | Preserve atomic episode-fenced assignment; bound/reference values and account for old cell/index-key reclamation. |
| `lv-h2c8b0107116f` | `fractal_store::store` hydrates the first authorized exact-request answer with effective-reviewer predicates. | Reference/chunk hydrated values while preserving answer authority at its canonical moment. |
| `lv-h82624af80f95` | `fractal_store::store` schema adds request, answer, reassignment, reviewer-routing and exact episode indexes. | Existing variable inline keys enable indexed episode seeks but are not bounded-size proof; reference/chunk variable columns and index expressions without changing fold semantics. |

### Conversation-owner inventory review (PR #3116)

The [owner-once change](https://github.com/compoundingtech/fractal/pull/3116)
retains the existing inline claim representation. The responsible maintainers
are the maintainers of the producer modules named below; ownership does not
transfer to the offline command or this inventory. All entries remain
`allow-for-now`, including conservative read sinks and the empty schema probe.
An indexed seek or a bounded row count does not bound TEXT keys, decoded JSON,
or returned claim bytes.

- `smallclaims::store`: `lv-h9722d4635b44` adds the transport-pair index
  over origin, subject and accepted-time TEXT; `lv-h93965131ebb0` and
  `lv-hb4ef462c895d` split the existing core schema around that shared index.
  Existing claim bodies, predecessors, metadata, blobs and variable index keys
  remain inline with unchanged logical semantics. The module maintainer owns
  `chunk-or-reference-variable-columns`: qualify bounded units or references,
  including index keys and old-value release. `lv-h5a3d29781577` replaces the
  transport scan with recursive pair seeks; origin/subject, status and timestamp
  values still come from historical claims. Its
  `trace-statement-and-value-representation` work must connect the canonical
  query expansion and extracted values to that representation.
- `fractal_store::store`: `lv-hd02768fb2b9c` and `lv-ha749c7998d2f` install
  runtime, actual-state and identity indexes. Runtime identifiers, subjects,
  origins and accepted-time keys remain variable; registry-derived predicates
  do not bound their bytes. The module maintainer owns
  `chunk-or-reference-variable-columns` while preserving canonical ordering,
  transition resets and historical identity semantics.
  `lv-hb05e486c0515` reads one canonical runtime claim per origin, including
  its body; `lv-h5b2f62a33993` and `lv-h884c57c5eacf` prepare and bind the
  fallback source query returning claim ID/origin. Their
  `trace-statement-and-value-representation` work must account for formatted
  predicates, subject bindings, variable identities and full claim bodies;
  these reads introduce no physical size guarantee.
- `fractal_store::owner_indexes`: `lv-h735c5d58ef79` and `lv-h0f440b8d87b1`
  execute the shared transport/runtime DDL against an explicitly stopped store.
  The command retains existing key representations to match startup exactly.
  Its maintainer owns `trace-statement-and-value-representation`, linking those
  execution sinks to the schema producers above and their eventual conversion.
  `lv-h3ad40afefaf0` creates the empty in-memory claims table used to compare
  expected definitions. No claim is inserted there, but its TEXT declarations
  remain catalog candidates, not a general size exemption; the maintainer owns
  `chunk-or-reference-variable-columns` if that probe acquires value producers.
- `fractal_store::store::conversation_reads`: `lv-h81c8d6a2e0f8` binds the
  subject/existence query; `lv-h07dae6c16278` and `lv-h2752cd7a95e2` prepare
  and bind registry-derived identity reads. The module maintainer owns
  `trace-statement-and-value-representation`: trace subject parameters, JSON
  extraction and returned runtime/incarnation strings to bounded physical
  storage without changing historical identity semantics. A boolean existence
  result does not establish a bound on its subject/index inputs.

## What the static guard checks

Run these lightweight source controls; neither opens SQLite or compiles Rust:

```sh
python3 scripts/check-sqlite-large-values-test
python3 scripts/check-sqlite-large-values
```

The same commands run through `scripts/ci-genie-freshness` in ordinary hosted CI
and the existing Nix check. The pass line prints guard elapsed seconds from its
actual source scan. In the initial authorized static check, the guard subprocess
took 11.441 seconds wall time and the 19 original controls took 0.061 seconds
including Python startup; these are source-check observations on that executor,
not native storage measurements or a hosted-runtime guarantee. The proposal helper
is not added to CI. Retain hosted guard elapsed separately when it runs.
The guard scans Rust files under `crates/**/src/**`.
It detects literal INSERT, REPLACE, UPDATE, DELETE, CREATE, ALTER and DROP
statement templates, including those inside triggers. It separately inventories
ambiguous `.execute`, `.execute_batch`, `.prepare`, `.prepare_cached` and
`.query_row` arguments in crates declaring rusqlite, libsqlite3-sys or sqlx.
Prepared-statement bind calls stay unresolved even when a parameter is a string
beginning with SELECT. New SQLite dependency crates change the coverage inventory.
`include!` is inventoried everywhere; `include_str!` is inventoried in those
SQLite crates. A new standalone SQL file under `crates` fails until its coverage
is added. New covered sites without matching comments fail, as do removed,
duplicate, malformed, orphaned or changed exceptions and altered column catalogs.
The guard compares exact template bytes and comment-free dynamic argument tokens;
it does not infer receiver types or evaluate Rust/SQL.

Explicit inline `#[cfg(test)]` items are excluded. Arbitrary cfg expressions are
not evaluated; separate test-only source files and diagnostic strings may be
conservatively included. This makes the manifest a candidate source inventory,
not a count of production writers. The column parser handles literal CREATE
TABLE/INDEX lists and ALTER TABLE ADD COLUMN declarations, not an evaluated SQL grammar: generated table/column names,
other ALTER-derived shapes, implicit/automatic indexes and runtime schema introspection
need source review. A literal declaration is not proof of all installed schemas.

**Blind spots remain:** macro expansion, SQL assembled from fragments, generated
OUT_DIR files, external Runtime implementations, raw FFI/other database APIs,
indirect sinks outside the declared dependency crates, and value growth through
bindings, serialization, JSON expansion, aggregation or indexing. A read-looking
template can be changed dynamically. A source reviewer must extend coverage when
introducing one of these paths. A green guard is source-site coverage within this
declared boundary, never complete type/dataflow analysis, an all-mutator evidence
certificate, a size proof, or a latency result. The manual flow inventory is finite;
new delegation paths need human review even if a lower covered sink already exists.

## Value and producer review checklist

| Value family | Producers and paths to follow |
| --- | --- |
| Claim body, predecessors, actor/subject/kind/operation/request IDs | Direct Core insertion and append; application/native admission; local document writes; registry-valid generic fields; SQL repair and historical restore. |
| Raw envelope/header/payload and dependency lists | Local signed sealing, current and legacy replication decoding, backup pages and offline imports. Preserve raw identity independently of derived JSON. |
| Blob bytes, staged values and promotions | Direct blob put, staged-to-durable promotion, legacy separation, backup/copy/reopen. A digest key does not bound the referred value. |
| Tombstone and checkpoint metadata | Manifest adoption, exact dependency closure, capture, proof staging, trim rollback/redelete and generation publication. |
| Projection/cache/document/response values | Actual parameter bindings, local observation and idempotency writers, runtime finalizers and repair paths; separate logical authority from local storage. |
| Variable keys and aggregates | Every declared index/uniqueness/foreign-key expression, dynamically generated DDL, JSON/string aggregation and serialized metadata; review both insert and delete costs. |

HTTP extraction and exchange/decompression limits are specific to their entry
points. Local append/sealing, direct native receive, backup and physical clone
have different contracts. JSON escaping, base64, CBOR and signed wrappers change
encoded size. SQLite's compiled/runtime limits constrain rows as well as values;
they are not a demonstrated reachable application maximum. The inventory makes no
claim about the largest currently stored value or actual native build limits.

## Cleanup and migration plan

Immediate logical hide or inactive generation publication must preserve the whole
logical source and all authority/certification predicates. Prepare exact contents,
manifest and dependency closure away from the live writer. Publish only a short,
validated atomic visibility change. Old readers must continue to see their pinned
cut; partly verified certificates, admission, inventory, capture, replay, digest,
repair, export and resume consumers must all use the same visibility semantics.
Inactive physical writes may be logically neutral but still require automatic
source-owned mutation evidence. An ordinary legacy/raw write to a masked key must
not evade evidence using a caller-supplied housekeeping flag.

Physical cleanup comes later, durably and incrementally, after reader/proof/page
pins close. Expiring a cursor rejects new accesses using the existing typed error;
it does not release a page already in flight. That page must finish or return its
typed error and close its owner before reclamation. Bounded deletion by row count
alone is insufficient: one existing cell's overflow chain is released inside one
DELETE or size-changing UPDATE. Replacing its body with a pointer or NULL also
frees that old chain. Triggers, cascades, metadata/index keys, digest maintenance,
runtime finalizers, commit/fsync, rollback and actual connection return are part
of the cost. Logical row cleanup, WAL checkpointing and physical file space
reclamation are distinct obligations. No smaller cap, longer timeout, disabled
functionality or indefinite legacy fallback completes this plan.

A forward conversion must cover the largest valid current and admitted historical
cells, including the old representation's release. Compare stopped in-place
conversion against isolated replacement/retired backing using the **actual all-DB-
opener** contract, disk budgets, crash states and write-preserving recovery. An
old snapshot restore can lose acknowledged later writes and is not a default
rollback plan. Preserve exact raw bytes, canonical JSON/merge semantics, indexes,
streaming reads, native authority, repair exclusion, digests, cursor identity,
copy/backup/open paths and fail-closed recovery before activation. Missing backing
must return an explicit open/access error; it must never silently select an older
root. External data, if selected, must become durable before its directory and
main-database pointer publication. There is no migration, stop window or execution
permission in this rule change.

## Accepted source directions and remaining decisions

The source-only directions below authorize design/control preparation. They do
not select real DDL, fields, indexes, thresholds, wire changes, a schema version,
an executor, a migration or an operational window:

- **S1:** inactive SQL generations (D) with certified logical hide and dependency-key
  reclamation (C); evaluate one forward versioned conversion, staged readers and
  one old-binary fence. No live-writer primary-key rebuild.
- **S2:** compare a hybrid first: small inline values preserve SQL/index semantics;
  large values use SQLite chunk rows with exact extracted/index metadata. External
  extents are for an evidenced residual. No threshold or chunk size is selected;
  a hybrid does not remove any consumer/JSON/index/trigger/authority obligation.
- **S3:** compare a stopped in-place converter with isolated replacement/retired
  backing, first obtaining held operational pricing, all-opener and write-preserving
  recovery facts. No tool, copy, stop or window is authorized.
- **C1:** design additive negotiated manifest identity in responses and
  continuations, preserving existing page-cursor-expired 410 behavior. Exact new
  semantics still require contract review; this change alters no wire field.

Real representation/conversion decisions require separate storage/contract review
and the required human decision before implementation. Native cost controls,
large-cell fixtures, long-reader/WAL checks and crash/reopen tests remain future
qualification work. This source guard supplies none of those results.

## Completion-health candidate review

`fractal_store::store::completion_state` owns the new health-v1 namespace and its
inventory exceptions. Schema site `lv-hde7868afa605` retains claim identifiers,
subjects, canonical sort keys and three selected JSON fields inline. Field count
is bounded; encoded identifier/key/reason sizes are not. No cursor, resource facts
or signed envelope is copied into this namespace. The producer maintainer retains
`chunk-or-reference-variable-columns`: qualify physical references for variable
keys/values, including predecessor selection and deletion, without imposing new
logical input limits.

The module's remaining inventoried sinks create body-free queues, process changed
claim identities, read one changed authoritative body and insert/delete its narrow
candidates. `lv-hb0cb6dfa9cec` and `lv-h73e9590d9234` are the prepared query and bind
for that single-body read; `lv-hde7c87402fcd` binds indexed field-winner reads.
They remain transitional exceptions, not byte-size guarantees. Inline narrow
candidates preserve existing canonical ordering and avoid copying historical
cursor payloads on completion reads; eventual physical breakup must preserve
explicit clearing markers versus JSON null and indexed repaired predecessors.
Queue/reclamation and dynamic-source TODOs remain with this same producer.

Cold-store hydration runs after both APIs bind. Its persisted subject/kind/index
cursor seeks at most sixteen source identities per independent background writer
loan, yielding after ten milliseconds between candidates and resting outside the
writer between loans. Bootstrap does not copy the historical inventory into the
pending queue. Live changes behind its cursor are drained in a bounded final phase.
Until hydration completes, preparation refuses the projection and canonical reads
retain the original fold. A single source body and COMMIT remain indivisible, so
the chunk budget is not a hard elapsed-time bound. Local-observation retention does
not bound replicated observer claim history.

`fractal_store::store::completion_state_tests` owns the associated fixture-only
exceptions: invented time/origin/position mutations, repair membership, deletion,
savepoints and preparation closures. These exercise invalidation and rollback;
they are not new production value producers or permission to relax wire limits.

## Deploy verification inventory review (PR #3823)

The [deploy verification change](https://github.com/compoundingtech/fractal/pull/3823)
reads existing claim, signature and receipt representations without adding a
production writer or changing their storage format. The responsible maintainers
are those of `fractal_store::store::deploy` and its `tests` module. These
transitional exceptions do not prove byte-size or execution-cost bounds.

- `lv-hcd60d83de7b8` resolves a claim by ID and decodes its existing inline body,
  predecessors and variable identity fields. `lv-he4d379218d03` binds GO/local
  origin strings and reads claim IDs/origins for authenticated clock evidence.
  `lv-h5e7d6e0f3a53` reads IDs/kinds in store order for revocation, consumption,
  STOP and lift verification. The formatted predicates retain the existing
  unrepaired-claim and replica exclusions. All three remain inline to preserve
  exact canonical authorization and historical claim decoding; the producer
  maintainer owns `trace-statement-and-value-representation`: connect formatted
  statements, variable TEXT/index keys and decoded claim values to the shared
  bounded/reference representation while retaining snapshot, signature and
  eligible-origin/local-clock ratchet semantics.
- Fixture-only `lv-h493aa8cab034` and `lv-h63250a64d298` mutate/remove the
  host-b replication receipt; `lv-haeca40cf61e3` inserts an unsupported raw
  revocation envelope and variable timestamp/state fields. They deliberately
  retain the actual inline receipt/transport representation to test stale,
  missing and unreadable evidence. The test-module maintainer owns
  `chunk-or-reference-exact-wire-values`: adapt these fixtures to the shared
  representation while preserving exact raw-envelope rejection coverage.
- Fixture-only `lv-h8c4ce07dd4ee` changes principal-migration state to legacy;
  `lv-h455635f8de24` inserts a variable claim ID into repaired membership.
  These direct mutations exercise incomplete authority and repaired-release
  rejection, not new production value producers. The test-module maintainer
  owns `bound-or-reference-values-and-reclamation`: carry those assertions to
  the bounded membership/state representation and its cleanup semantics.

## Harness receipt lookup inventory review (PR #3916)

`fractal_store::store` owns schema site `lv-h806ead738248`, replacing
`lv-h82624af80f95` solely for the additive receipt actor/key index. The index
extracts the existing receipt digest and retains existing origin/subject TEXT
keys; it stores no new question content. Canonical first-resolution and local
owner checks remain authoritative after the indexed candidate lookup. Existing
declaration metadata and the producer's `chunk-or-reference-variable-columns`
work remain; an indexed seek is not a byte-size proof for the retained keys.

Fixture-only `lv-hc58fa3f9b3bb`, owned by `fractal::main_tests`, includes the
existing client capability envelope to prove CLI answer fences come from the
caller, not a new question read. It adds no production writer or SQL value.
Its `trace-statement-and-value-representation` work remains with that module.
