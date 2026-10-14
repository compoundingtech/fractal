# Receive replay source audit and unrun controls

Review input for [the proposed receive replay design](receive-replay-design.md) and
[#2861](https://github.com/compoundingtech/smalltalk/issues/2861). This audit is pinned to
`24d78d71b9b99a2614de215c1f1660e1b2fa7d2e`. It changes no runtime path.
The table-name swap and randomized interleaving controls below are **UNRUN**. Neither a
production row/input witness nor a complete generation routing certificate is supplied.

## Counted source inventory

The reproducible literal census and all path/line entries are retained in
`doc/fleet/smalltalk/graph-projection-divergence/2026-10-14/receive-replay-source-inventory@bf7c00b4ec38efa9d34748540b59a106cb295a74deee3ecd93d1ce6ba6ea6f6d`.
It includes the scanner and a hash-bound compressed JSON inventory. Decompression is data
inspection, not an instruction to execute a control.

Scope: all 417 tracked Rust files in `crates/st3/src` and `crates/smallclaims/src`, including
`cfg(test)` code and fixtures. Rust comments and character literals are skipped; SQL comments
are removed before matching SQL-leading string literals. The census finds 3,530 literal sites,
of which 300 contain templates. Counts below are **literal sites**, not SQL statements,
production tables, installed objects, or expanded execution. Roles overlap. Identifiers can
include CTEs, aliases, test tables and placeholders. Macro expansion, conditionally compiled
reachability and dynamically assembled identifiers still need a reviewed production inventory.

| Pattern | Literal sites | Distinct lexical identifiers |
| --- | ---: | ---: |
| Read (`FROM`/`JOIN`) | 2,307 | 289 |
| Write (`INSERT`/`UPDATE`/`DELETE`) | 1,561 | 243 |
| `CREATE TABLE` | 146 | 231 |
| `CREATE TRIGGER` | 73 | 192 |
| `REFERENCES` within a table DDL literal | 11 | 11 |
| `CREATE INDEX` | 42 | 179 |
| `INDEXED BY` | 148 | 57 |

The verified entry-path walk is narrower than the census: eight graph tables are cleared,
24 logical tables are registered for shared projection digests, and seven post-projection
calls execute in order. None of those inventories equals the complete stage closure.

| Boundary at the pinned source | Routing and hold obligation |
| --- | --- |
| `smallclaims/src/store.rs::project_replication_backlog_chunks` | Native admission, foreground IMMEDIATE transaction, incremental savepoint, replay, hooks, frontier/health and commit; all expanded work belongs to the hold |
| `st3/src/store.rs::REPLAYED_GRAPH_TABLES` (30260) | `desired`, `mission_definitions`, `mission_run_deadlines`, `mission_run_after`, `step_runs`, `revision_proposals`, `run_generations`, `mission_runs`; route each read/write and its dependencies |
| `replay_graph_from_nothing_with_progress_tx` (30279) | Operations, base claims, mission runs, planning, resources, glass heads, directive notes, custom, arrangements, publication reset; eight clear targets omit several rebuilt families |
| `st3/src/store/runtime.rs::after_projection` (406) | In order: custom, resources, glass heads, directive notes, agent messages, limits, local work-lease renewals; seven calls, with queues/cursors and dependencies retained |
| `st3/src/store.rs::PROJECTION_DIGEST_TABLES` (59194) | 24 tables listed below; native blobs/documents/operations are also inputs, so do not duplicate or replace them merely because they are digested |
| `smallclaims/src/store/projection_digest.rs::{register,initialize,columns}` | Four digest state/cache tables; generated per-table triggers, scalar functions, sorted logical columns and initialization scans; route logical identity separately from physical generation names |
| `st3/src/store/publication_facts.rs::{create_schema,reset,flush,install_cache_observer}` | Six local publication tables, source triggers, bounded finalizer, committed cache epoch and native observer; a stage must not redirect them away from native/local evidence |
| `st3/src/store/runtime.rs::finish_schema` | Core/local schema helpers plus optional registered IVM views; every schema hook and dynamically registered view needs routing classification before conversion |
| `smallclaims/src/store/checkpoint_staging.rs` and checkpoint rules | Retained epoch/cut/cursor and proof copies are precedents, not a certificate for all replay inputs; keep trim, repairs and protected source coverage |
| `smallclaims/src/sqlite.rs::{observe_commits,WriterGuard::drop}` | Native finalizers/committed-authority observers complete before acknowledgment/connection return; include their work and failure in the acquired-to-returned hold |

The 24 digest table names are `operations`, `blobs`, `documents`, `desired`, `arrangements`,
`arrangement_registers`, `message_index`, `resource_observations`, `glass_heads`,
`person_directive_notes`, `custom_registrations`, `custom_sources`, `custom_dependencies`,
`mission_revisions`, `mission_definitions`, `mission_runs`, `mission_run_deadlines`,
`mission_run_after`, `run_generations`, `step_runs`, `revision_proposals`, `planning_sessions`,
`planning_candidates`, and `planning_previews`. Existing excluded columns remain part of the
semantics: notably effective local step lease/update dates are excluded from the shared digest,
but remain typed local rows and must survive staging. Cached digest equality cannot prove that.

An implementation audit must classify every census site as native source, staged output,
local overlay, disposable temporary relation, or unsupported dynamic route. It must close
cross-generation joins, raw readers, writable views, triggers, foreign keys, index names,
provider/IVM schema hooks, transaction maintenance and cache invalidation. The 300 template
sites and test-inclusive counts are explicit remaining work; they are not silently approved
production routing. No complete input/row/cache oracle follows from this source audit.

## Non-claim inputs and journal closure

These source consumers demonstrate why replaying the admitted claim bodies alone is incomplete.
The journal must represent authoritative source transitions; derived dirty queues/cache rows
are either reproducibly rebuilt or snapshot-bound, never treated as independent authority.

| Actual source or local input | Consumer and transition to cover |
| --- | --- |
| Batch origins/sequences, retained replica metadata, repair state and canonical positions | Base replay uses `claims JOIN batches`, repaired exclusions and `canonical_sql`; metadata repair/rank changes invalidate ordering even without a new claim body |
| Checkpoint claim/operation metadata and persistent repaired exclusions | `smallclaims/src/store.rs::{expected_operation,expected_operations,rebuild_operations_tx}` reads checkpoint records and `projection_digest_repaired_claims`; trimming a receipt must not erase its meaning |
| Content-addressed blob bytes, document bindings and mission revision content | `custom.rs::refresh_registrations` (223) reads `documents JOIN blobs` and validates the manifest hash; base mission/document selection also resolves retained content; bind content and retention, not only a document name |
| Quiet member-local work lease renewals | `st3/src/store.rs::reapply_local_work_lease_renewals_tx` (29017) reads `local_work_lease_renewals` and overlays only the matching attempt/owner/incarnation; it is not reproduced by replicated claim bodies alone |
| Local startup observations and publication bootstrap state | `publication_facts.rs` source triggers consume `local_observations`; native finalization and committed cache epochs must remain local and correctly fenced across replacement |
| Dirty/pending queues and cursor/version metadata | `custom`, `resources`, `glass_heads`, `directive_notes`, `agent_messages` and `limits` flush routines consume local queues or `meta` cursors; recompute from the fixed source or bind and journal their completion, without losing pending work |
| Native authority/admission/checkpoint and registered reducer identities | Runtime registry, executable/schema/storage/rules identities and protected retention are part of the certificate; a schema-hook or manifest change refuses resume rather than reinterpreting old scratch output |
| Preserved operational state outside replay outputs | Idempotency receipts, physical start results, local stops/attention and admission frontiers must not be erased by projection publication; classify any projected consumer before deciding whether it needs a journal entry |

This is a demonstrated input inventory, not proof that these eight categories close every
registered/custom provider. Nested validators and dependency edges in `custom.rs::refresh_source`
(499), coordination effects in `agent_messages.rs::flush_with_coordination` (156), and optional
IVM schema hooks require their own exact closure review. A caller clock or configured provider
must not enter a replicated row without a deterministic input contract. The tiny fixture oracle
must distinguish local overlays from shared digest columns and report the first differing
column, including whether it is a timestamp.

The full authored interleaving design is retained in
`doc/fleet/smalltalk/graph-projection-divergence/2026-10-14/receive-replay-interleaving-control-design@9510d9038a0084c8d3c501d514b5498a614812ec9527cb670541114b64cd1ec5`.
It uses disposable origin/replica/stage/reference stores,
seeds 0, 1, 17 and 2861, and at most 64 dependency-valid events per seed. Its finite input cap is
256 claims, 256 KiB payload and 4,096 reference rows. Events cover admission, duplicate/empty
receive, repairs, checkpoint/trim, late canonical order, document/blob retention, local renewals,
queue/finalizer completion and crashes at capture/build/copy/seal/commit/ack boundaries.
Actual native/page acknowledgments select scheduling points. Record the generated schedule
before applying it; authority refusal is a result, not permission to manufacture state.

At the finite target, compare only after origin, staged and independent replay inputs certify
**the same cut**, including repairs and local overlays. Compare typed rows, native receipts,
operation identities and independent fixture-only row hashes, then compare cached digests to
that oracle. Deliberately omit a local renewal, drop a repair invalidation and change the source
after sealing: each must cause the relevant row mismatch or CAS refusal. Also exercise wake in
Pending-to-wait and crash after physical commit before acknowledgment. Missing adapters prevent
execution; this specification is UNRUN and supplies no parent-fails/fixed-passes claim.

## Disposable table-name swap spike

F1 remains unproved. The exact separable source and recipe are retained in
`doc/fleet/smalltalk/graph-projection-divergence/2026-10-14/table-name-swap-source-recipe@60ad7abd01c052a05d52b93194461f6d6e27f730ea58d043f57ea6e54554dca4`.
Source SHA256: `74719d00f3fae152203f8772d6e34ce2f3564d1cd866afb3d973c5009a3cb578`.
Only Python `ast.parse` was used; the module was not imported or invoked.

The proposed future unit runs serially on a newly created disposable directory and accepts no
existing database path. Cases use 0, 1,024 and 16,384 rows with 128-byte values. The recipe proposes
a 90-second queue-inclusive cap, 64 MiB total owned disk, MemoryHigh 256 MiB and MemoryMax 512 MiB;
these are unadmitted ceilings, not measurements. Source, Python/SQLite binary, caller, lease,
unit and cleanup owner still require a concrete ordinary hosted qualification disposition.
No Nix/Cargo/native invocation, alternate executor or automatic retry is authorized here.

The source prepares live/shadow tables with differently named indexes, a referencing table,
a view and an insert trigger. It records the duplicate global-index-name refusal; swaps table
names inside one transaction; records `INDEXED BY`, view/FK/trigger rewrites and the cached-query
result; then separately measures populated index repair and populated retirement. Each timing
includes its COMMIT, with WAL/FULL durability retained. Schema definitions, SQLite version,
PRAGMAs, estimated VM progress and disk size accompany results. Connections close before removal
of only the owned temporary directory; cap termination still needs a terminal cleanup receipt.

Success requires unchanged logical reader/writer/FK/trigger/index semantics and a bounded final
native hold without deferring populated cleanup into another unbounded hold. A fast two-rename
transaction alone is insufficient. Python statement caching does not certify rusqlite's cache,
and this mechanism spike cannot prove the actual WriterGuard/admission/observer boundary.
Those interfaces need a separate native control after a viable alternative is reviewed. If a
fully qualified table-name swap meets the boundary, reconsider the generation/format proposal.
No impossibility or positive result is claimed from the unrun source.

## Conditions carried to the decision

| Finding | Proposal revision and remaining qualification |
| --- | --- |
| F1 | Name-swap alternative and bounded source recipe are explicit; mechanism/native results absent; no format-free impossibility claim |
| F2 | Exact source census and entry/hook inventory above; compiled routing/dynamic-provider closure remains required |
| F3 | Actual non-claim consumers and finite interleaving discriminator above; journal closure/native comparisons unproved |
| F4 | Existing 19→20 format-hold owner join proposed in the design; legacy-19 refusal is insufficient to fence all flat-only version-20 binaries |
| F5 | Proposed error/receive wire shapes and old-client behavior specified in the design; no emitted contract or generated client change |
| F6 | Land #2859 telemetry before implementation; per-guard cause/hold evidence stays distinct from a successful bound |
| F7 | Scratch, active/hidden/retired projections, WAL, retained inputs and ZFS-held blocks share an explicit disk budget with #2849 sorter spill; no quota approved |
| F8 | #2644 is independent and unlanded; retain native observer/permit/ack invariants without requiring its landing as the replay repair |

Docs may be reviewed as a proposal with these controls dormant. Nathan routing stays held until
Intake's required decision evidence exists. No native qualification, incident cause, graph
convergence, persisted-state repair, version reservation or implementation follows from the
source counts, recipe syntax or docs publication.

The previously accepted metadata diagnostic remains separately proposed: output must say
"not a row, cache or input oracle". Any 32-key point-field comparison uses the same certified cut
and key interval on both nodes and refuses a differing/unprepared cut or interval. It reports
whether the first differing field is a timestamp. No live capture or diagnostic implementation
is introduced by this audit.
