# Lossless wire archive and bounded hot history

Status: concrete design proposal, 2026-11-19. The first implementation is an independently landable, inactive `smallclaims::wire_archive` primitive. Production archival, hot-row retirement and checkpoint-policy activation are separate later changes. This document is not evidence that construction, restore, compatibility, sizing or latency tests have run. No live mutation is authorized.

## 1. Decision and scope

Keep original historical evidence in immutable, content-addressed binary segments, with indexed reads available to the native store before any hot copy is removed. Construct a bounded interactive store using **both 30-day and 14-day candidates**, each augmented by a protected-witness closure. Select a production window only from capacity-approved offline measurements and correctness results. Age makes data a candidate for relocation; it never establishes permission to delete evidence or remove a reader's witness.

The archive is a real additional durability domain, not a renamed SQLite database. Its bytes, growth, staging space, replication, restore responsibility and retrieval latency remain visible. Moving bytes does not make total storage growth disappear. In particular, an archive is not a substitute for the other representation and derived-state changes required by the database objectives.

### First PR: complete primitive, no production cutover

Implement sealed segments holding exact SQLite typed cells from precisely these tables:

- `replica_envelopes`;
- `replica_records`;
- `replica_envelope_signatures`.

The caller supplies explicit sorted envelope identities `(writer, sequence, envelope_hash)`. Export reads their envelope rows and associated record/signature rows in one source read snapshot. The reader opens a segment against an **externally supplied expected SHA-256**, validates it, provides an indexed identity-to-offset lookup and restores only these wire rows transactionally. It does not perform admission, repair, graph replay, claim reconstruction, ledger restoration or checkpoint activation. A transactional wire-row restore is not a complete product backup restore.

There is **no daemon default archive writer**, no age selector, no hot deletion, no storage schema/version upgrade, no checkpoint-rule change, no fleet feature activation and no production archive converter in this PR. Merely linking the module or opening the normal store must not produce archive files or alter persistence defaults. No release-dependent byte saving is claimed for this inactive primitive.

### Second PR: inactive native archive-addressed reads

`ArchiveReaders` pins explicitly supplied verified segments, indexes full identities and rejects contradictory duplicate coverage. Native `replica_envelopes_with_archive`, `replication_signatures_with_archive`, `replica_record_with_archive` and `export_replication_difference_with_archive` provide original typed peer envelopes/signatures, envelope-plus-position repair witnesses with raw bytes, and the existing bounded replication difference protocol over a merged compact inventory. Existing entrypoints stay hot-only: no daemon configuration, catalogue loader or persisted activation is introduced.

The merged inventory preserves all fork hashes and payloadless checkpoint identities; authoritative retirement still prevents serving archived payloads. Exact hot/cold duplicates collapse, while differing receipt/repair/signature snapshots fail closed pending the overlay-aware stage rather than silently selecting one tier. Repair reads do not mutate cold state or perform repair. The temporary merged inventory is not persisted or fed back into the hot inventory cache. Tests relocate wire rows only inside offline fixtures and exercise bounded empty-peer catch-up, byte-identical payload transfer, native repair witnesses, signature ordering, conflict refusal, retired non-resurrection and unchanged default readers.

Signature responses independently read the hot signature table even when its envelope is absent: follow-up receipt legitimately retains signature-only rows. They union those rows with cold signatures by full envelope identity plus member key, collapse identical signatures and refuse contradictory signature text. Envelope presence is never a prerequisite for exporting a retained hot signature.


### Third PR: separate native filesystem-capacity gauge

The daemon's existing hourly database sampling lane also measures the dedicated `wire-archive` directory beside its graph database. It never creates that directory or enables an archive writer. A missing never-measured root is `not-activated`, not zero; a previously measured missing root in the same scope, unknown root category, symlink, special file, incomplete listing/stat or capacity overflow is unavailable. Copying or moving a graph does not transfer activation history to its new archive scope. An existing and successfully inventoried empty root may report zero.

The layout has `segments`, `index`, `overlays`, `staging`, `orphans` and `backups` categories. Segment filenames are lowercase SHA-256 plus `.wire`; private `.wire-segment` files are staging and other segment-directory names are orphans. Hardlinks count once by device/inode, with deterministic category ownership. `archive_bytes` counts segment-file, index and overlay lengths; `capacity_bytes` additionally includes staging, orphan and backup-owned lengths. Sparse-file allocation and remote replicas are not inferred from these logical file lengths.

Hourly derived samples persist under `wire_archive_size_samples/v1`. Net growth needs a baseline within 30 minutes of one day ago, same scope, no unavailable sample and no observation gap greater than 90 minutes; no one-hour extrapolation. Thus a missing hourly write also breaks coverage when its unknown result could not persist. Clock rollback or scope movement starts a new series. Missing/stale baselines remain unknown. The native `database-archive` row is a separate informational gauge with no passing objective budget; it keeps the existing hot `database` row unchanged.

This first accounting integration explicitly reports `integrity: not-assessed` and null catalogue coverage. Filesystem scans are not a common catalogue-generation snapshot, do not verify payload hashes, and cannot establish whether every expected archived object is available. The catalogue-integrated generation/availability proof below is still mandatory before hot retirement or M1 acceptance. The sampler is not that proof and never labels named segment files as cryptographically verified sealed objects.

### Fourth PR: offline 30-day and 14-day candidate construction

`st replication archive-candidates --source-copy PATH --output PATH --as-of-unix-ms N --offline-copy` is an explicit offline analysis command, not a daemon RPC or a deployment path. It requires a supplied consistent offline copy and fixed observation time, constructs **both** windows, and never discovers or captures a live host database. The output stores lack default-hot-reader history and must not be opened by a daemon or used as deployment/backup artifacts. A completed report explicitly denies activation authority.

The source database and its SQLite sidecars are never opened by SQLite in place: copy their bytes privately, verify source stability, and open SQLite only on that private capture. A standalone snapshot feeds both windows. Refuse unsafe/symlink/overlapping or preexisting outputs, malformed/inconsistent evidence and unsupported inputs rather than silently repairing the source. Private file modes and exclusive publication protect original payloads.

WAL physical allocation is not its committed length. When a copied WAL-index is present, validate both checksummed index-header copies, their generation and the last committed frame's checksum/database size before accepting the committed prefix. Only a different-generation physical tail beyond that independent bound may be stale allocation; corrupt committed frames and current-generation frames beyond the bound remain errors. Without a WAL-index, require every supplied physical frame to validate and the final frame to commit. This conservative input contract does not recover or truncate ambiguous evidence.

Reuse the bounded staged native checkpoint planner and its rules digest. Only whole envelopes that the existing planner permits dropping can leave a candidate's wire tables; protected witnesses remain hot. Further restrict that set by the selected hot window and by admission, repair and signature-observation times, not merely the envelope's historical timestamp. Unknown time cannot prove old age. All claims, projections, nonce membership, ledgers, shards and checkpoint authority remain outside the wire-only removal.

Before candidate removal, export sealed immutable segments with external seals and indexed full identities. Restore each bounded segment into an exact-schema scratch database and re-export the same identities; require byte-identical seals/bytes. Keep proof scope explicit: wire-evidence preservation, not the full product backup/restore acceptance gate. Close store handles before compaction and report the compacted hot file, archive segments/index metadata, staging and source separately. Report actual measured GB even when it exceeds the interactive objective; this experiment selects a defensible hot target rather than forcing one.

Fixture proof precedes any real-store run. Real-copy construction additionally needs approved isolated disk above the host floor, admitted heavy work with an honest profile, reviewed exact-head code and an OPS-supplied consistent offline input. Capacity approval does not authorize live capture, credentials, fleet activation or hot-row removal. Neither candidate measurements nor a seven-day production growth result are claimed until their respective executions actually occur.

### Later production scope

Archive-aware native reads, current-state overlays, catalogue publication, complete backup integration, separate archive native metrics and offline construction proofs precede activation. Historical claims, signature-candidate authority, global nonce membership, dependency invalidation and operation receipts need their own reader/representation changes; archiving three wire tables does not solve them implicitly.

## 2. Evidence and current contracts

Source inspected for this proposal:

- `crates/smallclaims/src/store.rs:650–748`: envelope, record and signature schemas. An envelope has mutable receipt/error fields; a record has `pending`, `valid`, `unknown`, `invalid` and `repaired` states, original `raw`, replacement ID and update time. A repaired copy excludes its original claim. A single writer sequence may have more than one envelope hash.
- `crates/fractal-store/src/store/checkpoint_rules.rs:1–10,19–57,94–104`: deterministic same-slot witness replacement, rules version 26 in this checkout, sealed-set exclusions for repaired originals, projection/replay contracts and whole-envelope/reference guards. This is a checkout observation, not a deployed-fleet version assertion.
- `crates/fractal-store/src/store/backup.rs:9–61,100–149`: backup header roots and ledger digests, fully projected admission-frontier requirement, local batch sealing and graph/shard snapshot discipline.
- `crates/fractal-store/src/backup.rs:69–121,300–450`: offline export may migrate a private copy; restore stages an empty offline destination, verifies the stream, appends checkpoint pages, receives ledgers, finalizes graph before shard admission, validates referenced bodies and seals WAL-independent files before publication.
- `crates/fractal-store/src/store/database_size.rs:1–23,62–91,122–184`: one hourly database sampler, persisted history, separate occupied-page and physical-file accounting. Existing live growth may extrapolate from a span as short as one hour; physical growth uses a baseline within 30 minutes of one day.
- `crates/fractal-telemetry/src/slo.rs:373–390,530–566`, with daemon call site `crates/fractal/src/main.rs:22623–22627`: native database sample ownership, two-hour gauge freshness and the rule that unknown growth cannot prove a healthy conjunction.
- [PR #6765](https://github.com/compoundingtech/fractal/pull/6765): reader-first signature-reference staging leaves default storage v20/inline TEXT while widening supported reads; a later release can activate only when its published N-1 contains the reader. Its reported CI and fixture measurements are that PR's evidence, not archive proof.
- [Fleet compatibility](compatibility.md): N/N-1 coexist for at least 14 days, a returning member offline up to 30 days must not corrupt or block the fleet, and matching checkpoint rules digests remain distinct from release ordering.

The read-only M0 census measured approximately 58–62 GB of occupied database btrees, with fresh native live samples approximately 58–63 GB. Envelope families cost 13.4–13.7 GB, records 10.5–10.9 GB, and envelope signatures 1.2–1.3 GB including indexes. Historical selected envelope/record/claim bytes older than seven days total 23.4–24.1 GB; recent-week selected bytes total about 2.9–3.0 GB. Neither figure includes the full protected working set and neither proves an 8 GB fit.

Complete-host short-interval wire envelope+record growth is about 273–291 MB/day. Even making those hot families stationary does not by itself meet the 300 MB/day whole-store target. The census's additional judging/dependency/claim reductions produce a quantified design envelope, not measured post-change rates. Preserve the original census distinction between logical bytes, occupied btrees, physical allocation and rates over different intervals. Missing per-index growth on one census member must remain missing; do not impute it from another host.

## 3. Identity, exactness and sealed format

### 3.1 What lossless means

Lossless means identical stored SQLite cell types and values for every selected wire row, including malformed or unknown content, original signed payloads, nulls, hints, errors, relay metadata, repair references and textual timestamps. It does not promise identical SQLite pages, rowids, btree packing or compression. It also cannot recreate network transport bytes that were never stored.

Do not parse and reserialize JSON/CBOR to produce archive authority. Preserve `payload` and `raw` byte-for-byte, even when a parsed record is valid and could be regenerated. Preserve original signature text without Unicode normalization, key ordering changes, newline conversion or formatting changes. Do not derive an invalid/unknown record's raw bytes from a decoded claim. A seemingly redundant raw copy remains independently stored authority unless a later reversible byte-slice representation is separately specified and proved.

Archive identity is the exact triple `(writer, sequence, envelope_hash)`, not writer/sequence alone and not a batch ID. Preserve forks and conflicting envelopes rather than collapsing them. The source snapshot's entire selected envelope row and all matching record/signature rows travel together.

### 3.2 Versioned binary layout

The first primitive uses the 16-byte magic `fractal-wire\0\0\0\x01`, with version 1 embedded in the marker. This version is independent of SQLite `user_version`, checkpoint rules and peer protocol versions. Table and column descriptors are fixed by the versioned codec, not encoded as a second negotiable schema.

The concrete layout in `crates/smallclaims/src/wire_archive.rs` is:

1. The 16-byte magic/version marker.
2. Entries in strict envelope-identity order. Each entry is an unsigned 64-bit big-endian CBOR byte length, the CBOR encoding of `(identity, [envelope rows, record rows, signature rows])`, and its 32-byte SHA-256. The entry digest covers the encoded identity and row contents; the full-file digest also binds length framing.
3. A CBOR sorted address index containing `{id, offset}` for each entry.
4. An unsigned 64-bit big-endian index length at the end of the file.

The externally retained `SegmentSeal { sha256, bytes, envelopes }` binds the entire file, including the footer/index and all framing. Offsets locate length-prefixed entries; entry identity must match the index identity. The file does not need to contain its own digest. A future production manifest retains the seal and source coverage. Unknown versions and invalid cell/table shapes fail closed.

CBOR preserves SQLite NULL, signed 64-bit INTEGER, REAL numeric values, UTF-8 TEXT and native byte-string BLOB as distinct cell types. Empty text/blob and NULL remain distinct. The initial codec rejects invalid UTF-8 stored in SQLite TEXT rather than replacing bytes; the SQLite schema's normal text fields are UTF-8. Supporting arbitrary invalid-TEXT encodings or preserving every noncanonical floating-point bit pattern would require an explicitly extended codec and proof. It must never be claimed implicitly. Original payload/raw BLOBs and signature UTF-8 bytes are preserved without parsing or reserialization. Restore into an incompatible schema is an error, not permission to coerce values or fill absent fields.

Canonical entry order is writer in binary byte order, numeric sequence, then envelope-hash bytes. Record order is numeric position with deterministic primary-key tie checking; signature order is binary member-key order. Column order is the fixed versioned descriptor order. Do not inherit database locale/collation or insertion order. Export validates strict input ordering/uniqueness and refuses a requested identity absent from the snapshot. It never silently produces a shorter archive. Exporting the same selected snapshot rows produces the same segment bytes; receipt state changes legitimately produce a different content address.

The first primitive bounds a segment to 512 explicit nonempty selected identities, each encoded entry to 128 MiB and the CBOR index to 1 MiB. Entry bounds also cap each contained cell/row; the combined upper bound is large (roughly 64 GiB), so these are safety ceilings, not a production target size. Enforce checked integer arithmetic and file-remainder bounds before allocation; malformed nested CBOR also needs bounded decoding tests. Stream file hashing and disclose index memory cost. Oversized source evidence fails export without truncation. Production should choose a much smaller measured total-byte target and split only between whole envelopes. No compression is used in version 1; a later compressed format needs bounded decompression and a separately supported codec.

### 3.3 Sealing and trust

The primitive writes staging output, syncs it, computes the full digest and publishes without clobbering through a hardlink, marks the segment read-only and syncs the parent directory. Its caller chooses the destination and retains the returned seal; production must choose a content-addressed destination and durably retain the seal. A read-only mode bit alone is not protection from the file owner, so production ownership and immutable access remain necessary. Never overwrite contradictory content at a digest path. The catalogue reference becomes visible only after the segment is durable. Interrupted export leaves unpublished data, not a discoverable half-archive.

Opening requires the expected external digest, verifies the whole file before returning a usable reader and verifies index/entry integrity. Unsupported versions, wrong digest, malformed framing, duplicate/out-of-order identities, bad offsets, altered entries and trailing data fail closed. A missing object is not an empty range. For large segments, whole-file verification may be amortized by an already-verified immutable open handle, but cannot be skipped merely because a pathname is hash-shaped. The primitive must not reopen a different path after checking it. Production publication must enforce immutable ownership so verified contents cannot mutate under a read handle.

A SHA-256 proves content identity, not origin, authorization or a valid claim signature. Normal admission and current authority checks still apply when replaying/exporting to peers. Archives receive the same access restrictions as the source database: these files contain original evidence and may contain sensitive payloads. Hash-derived filenames do not grant read permission.

## 4. Primitive snapshot and transactional restore

Export holds a single source read transaction across all three tables and the entire explicit selection. Use indexed per-envelope queries or bounded canonical keyset scans, not OFFSET pagination or independently opened snapshots per table. Export must not seal batches, repair rows, migrate schemas or write progress into the source. The caller owns read-worker admission and WAL pin duration; production-sized exports must use bounded segments and checkpoint capture fencing, not pin an unbounded live-history snapshot. An offline private copy avoids live-WAL pinning during sizing.

Restore validates the externally addressed segment before applying rows and uses one destination transaction for the requested operation. An error in the last record/signature rolls back earlier rows. The first primitive is insert-only and fails destination key conflicts; it does not use replacement inserts or silently merge contradictory evidence. A future idempotent duplicate policy would require comparing every typed column. Do not reconcile mutable state by timestamps during primitive restore. This API restores a captured wire snapshot, not an inferred latest state.

After all inserts, restore reads every selected row back inside the same transaction and compares the exact typed cells against the sealed evidence before committing. SQLite affinity coercion or destination triggers that change bytes/types fail the restore and roll back every inserted envelope, record and signature. Successful SQL execution alone does not prove a lossless restore.

No claims, batches, graph projections, nonce-use tables, runtime ledgers or shards are implicitly fabricated. The caller must distinguish successful wire restoration from product readiness. A fresh product restore still needs its ordinary validated backup/replay orchestration. Successful checksums do not authorize producer starts, membership changes or old operation replay.

## 5. Production archive catalogue and native reads

Production adds a transactional catalogue, scoped by store/fleet/shard identity, mapping each envelope triple to segment digest, offset, length and base-state generation. It stores an exact writer/sequence/hash range index for ordered replication, plus mappings for record reference/claim ID/batch consumers where needed. Indexes are derived and rebuildable from verified segments, but a temporarily missing index cannot mean history does not exist. Catalogue generations are pinned for a native read, backup or replication page.

The ownership boundary is simple: the archive codec knows rows and files; `smallclaims` owns the wire catalogue/read union and admission semantics; `fractal-store` owns product witnesses, folds and backup orchestration. Do not make frontend/reporting code reconstruct archive state. Inventory every direct SQL reader of these three tables before cutover, including exports, diagnostics, signature/provenance queries, repaired-original exclusion, checkpoint capture, backups, batch ordering, log digests and replication gap detection. Completion means all semantic readers consume the union, not just the normal sync endpoint.

Native lookup reads hot rows and archive rows under one logical generation. Exact duplicates across tiers collapse by identity and equal base content; contradictory evidence errors rather than selecting whichever tier answers first. A tombstone/move marker is a storage placement fact, not a claim deletion. Missing catalogue targets or failed reads surface structured unavailable/corrupt results and keep relevant health unknown/bad. Never answer not-found, an empty page, a successful digest or caught-up from an unavailable archive.

### Range replication and cold peers

Merge hot and archive indexes in canonical writer/sequence/hash order, retaining all hashes at a fork. Cursor semantics include the full identity and pinned catalogue generation; no page-size boundary may skip a second hash for the same sequence. A sequence gap must distinguish never observed, intentionally checkpoint-covered and archived-but-unavailable. Keep writer heads and predecessor/chain evidence sufficient to answer these questions.

A peer returning after the hot window must receive the same original payload/signatures via ordinary authenticated replication. The archive location is local implementation detail, not a required new wire message. Where current checkpoint adoption is valid, send the certified checkpoint plus retained history according to existing protocol; where original history is requested or checkpoint adoption is unavailable, serve indexed original segments through native reads. Do not force a cold peer to trust reconstructed JSON or a new authority model. Exercise both a 30-day-offline N-1 member and an initially empty peer. For a 14-day hot candidate, the 30-day return requirement is a direct cold-read acceptance condition, not grounds for shortening compatibility.

## 6. Mutable state, repairs and delta overlays

A sealed base captures mutable state at a snapshot; it does not freeze the truth forever. New signatures, validation results, hints/errors and repair replacements can arrive after archival. Never rewrite a content-addressed segment in place or let a stale base restore resurrect a repaired original.

Production maintains a small transactional overlay keyed by envelope triple and, for record changes, exact `record_ref`/position. Each overlay records its base segment digest, monotonically ordered local revision, exact replacement cells or newly appended signature row, and the committed state transition. It preserves repair provenance and replacement references. Row interpretation is base plus the latest committed overlay at the read's generation, not wall-clock last-write-wins. Timestamps are preserved evidence, not conflict-resolution authority. A later signature does not require resealing every earlier payload.

Route cold mutations through the same owning transaction/admission rules as hot mutations. The mutation and overlay/cursor state commit atomically. Pending/degraded envelopes and unknown/invalid/repaired records remain hot initially; resolution must be explicit. They may become cold only after overlay-aware repair/query/export paths and their proofs exist. A cold repair pins its replacement and all exclusion witnesses required by current readers. Global repaired-original exclusion must query both tiers and overlays.

A later compactor may merge a verified base plus a fixed overlay revision into a new immutable segment. Publish the new catalogue generation atomically, preserve old generations while readers/backups reference them and retire old objects only under a separate retention policy. Compaction does not delete original evidence. If intermediate state transitions are required historical evidence, retain their delta log rather than treating a current-state merge as a lossless history archive. Version 1 promises the exact stored snapshot, not state transitions the database never retained.

## 7. Hot selection: protected-witness closure, not age-only SQL

The existing rules are authoritative starting constraints. They retain a claim until a later kept same-slot claim replaces it for every reading fold, plan canonically and bind the rules digest to version plus engine description. Their guards include person actors, once-cardinality, non-valid records, repair replacement, projection references, claims in multiple envelopes, evidence citations, mission inputs/discovery, shared operations, newest writer envelope and whole-envelope closure. Status/incarnation boundaries, credential runs, resource attribution/history, questions, usage baselines, review bindings and ordered membership have reader-specific witnesses.

For a candidate window W and fixed cut/frontier:

1. Retain all recent admissions required by W. Receipt/accepted-time disagreements, malformed times and unresolved clock issues conservatively pin evidence; do not sort textual timestamps as chronological numbers.
2. Seed the current checkpoint planner's retained set and every protected current/reference/repair/security witness. Include active work and operation dependencies, authoritative runtime-start/generation/format-cut state, current signature roots/delegations/revocations and exact all-history nonce membership or its proven equivalent. No age rule can shorten operation replay policy.
3. Expand references to a fixed point: referenced claims, original and replacement repair evidence, their envelopes and signatures, writer chain/checkpoint carriers, and any additional fold-specific witness. Retaining one member of an envelope retains the envelope and required associated rows. Multi-envelope claims retain all needed provenance.
4. Treat an unknown kind, unclassified reader, incomplete dependency, failed proof or unavailable archive as protected, not disposable. Report the bytes and blocking reason.
5. Only the complement, backed by verified durable archive coverage and migrated readers, is eligible to leave hot storage. Archive retention is independent; leaving hot is not destruction of history.

There are two different operations: relocating exact wire evidence can preserve all logical history without changing which claims a checkpoint drops; trimming accepted claim history changes replay inputs and must extend the existing checkpoint plan/proof. Do not silently apply a wire relocation policy as a claim-drop rule. Any changed semantic witness or proof contract gets an explicit rules-version/digest transition and participant compatibility gate. No rule change belongs in the first primitive PR.

Proof compares current fold answers, canonical graph/projection roots including empty arrangements/membership/reverse edges, signatures and authority behavior, replay, repaired exclusion and cold read/export answers. Deterministic hot placement under a fixed cut is useful; proof equality must not depend on local receipt timing or a member's segment boundaries. Archive manifests record the source cut, source frontier/capture epoch, rules digest, archive format and catalogue/overlay generation. A changed relevant mutation during multi-page capture invalidates the candidate rather than producing a mixed snapshot.

## 8. Backup and restore invariants

The production backup is self-contained or carries an explicit complete, authenticated object manifest plus durable object availability obligations. A backup of just hot SQLite is incomplete after activation. Prefer a portable backup mode that includes all referenced segment bytes, pinned deltas and catalogue coverage rather than relying on a mutable remote path.

Extend existing backup verification/version negotiation before any hot removal:

- Pin graph admission/projection frontier, checkpoint capture and catalogue/overlay generations together. Preserve current batch sealing and fully projected graph checks. Independently pin shard state with the existing graph/shard ordering; do not call unrelated snapshots a common instant.
- Export checkpoint manifests completely with matching identity, cut, cursor termination, certificate/rules evidence and referenced objects. Preserve ledger digests/rows, including local authoritative runtime-start operation, generation and immutable publication-cut state. Claims-only recovery cannot authorize unresolved external producers.
- Maintain graph and log digests, per-table counts and quarantine/history coverage. A cold object omitted from a digest is a correctness failure, not an optimization. Shard envelopes/signatures/local rows remain scoped to the declared shard and fleet; a graph segment cannot stand in for a messages-shard backup.
- Preserve external message-body identity/hash and ownership checks. The archive primitive does not contain those bodies and cannot certify them present.
- Verification refuses missing objects, bad digests, incomplete checkpoint pages, unexpected versions, contradictory duplicate rows and trailing data before publication. Whole-file and entry hashes supplement, not replace, existing backup header/end and graph/ledger verification.

Restore to private staging with empty offline graph and shard destinations and locks. Verify object hashes and manifests; restore wire rows/catalogue/deltas to the selected generation, then run the ordinary product admission/repair/replay and ledger/checkpoint finalization. Finalize graph before shard admission as today. Close and seal WAL-independent files. Durably publish archive objects and required shard before the graph catalogue can expose references; unwind only objects created by this restore if graph publication fails. Never overwrite another store's shard or publish a graph with absent archive objects. A crash between multi-file publication steps must yield either the old complete store or recoverable staged state, not a nominally healthy partial restore.

Old backup readers must explicitly refuse a new required archive representation. Continue producing the old portable shape while mixed-reader compatibility requires it, resolving cold bytes through new native readers rather than dropping them. Version evolution is independent of the archive codec and needs its own reader-first gate.

## 9. Native archive bytes/growth row

Keep the existing `database` native row and target semantics unchanged: occupied SQLite live pages, daily live growth, physical main+WAL diagnostics. Do not subtract archive bytes from a combined historical baseline or relabel the current 60 GB file as an archive to make the database row green.

Add a separate native **`database-archive`** gauge row owned by the store's archive subsystem, sampled by the daemon and rendered through `fractal-telemetry::slo::store_report`. It is not a frontend approximation, claim producer or second competing database sampler. Proposed payload:

- `measured_at_unix_ms`, `scope_id`, `catalogue_generation`, `format_versions`, `available`, `error`;
- `sealed_bytes`: sum of file lengths of unique content-addressed sealed segment objects retained in this local archive scope, counting each object once, including old generations not yet eligible for retirement;
- `index_bytes` and `overlay_bytes`: separately owned archive-side catalogue/index/delta storage, including their WAL where applicable; do not include the same SQLite pages twice if housed in the hot DB;
- `staging_bytes`, `orphan_bytes` and `retained_backup_bytes`: separately visible capacity categories, disjoint from sealed/index/overlay ownership. A hardlink to an existing object is not a second logical object; physical allocated bytes, where supported, are a separately named diagnostic rather than inferred from logical lengths;
- `archive_bytes = sealed_bytes + separately_owned_index_bytes + separately_owned_overlay_bytes`, plus a separate total local capacity view including staging/orphans/backup-owned copies;
- signed `growth_bytes_per_day`, `growth_span_ms`, `baseline_at_unix_ms`, `coverage` and `expected/reachable_objects`. Growth is `(current archive_bytes - baseline archive_bytes) * 86400000 / elapsed_ms`, not bytes written by the archive worker. Export cumulative sealed-write bytes separately if producer throughput is useful.

Measure a consistent catalogue generation and complete object inventory; partial listings/stat failures, missing roots, catalogue targets that cannot be verified, stale samples and absent baselines are unavailable/unknown, never zero. A configured and successfully inventoried empty archive may report zero bytes; an unconfigured archive reports not activated/unknown, not passing. Keep accounting-generation changes and moved roots explicit. Never compare a new partial scope to an old complete one as a negative growth success. Remote replicas need separate scope-labelled inventory; local reachability does not prove independent durability.

Persist hourly samples over at least 49 hours with bounded count, restart continuity and clock-rollback handling. For the new daily archive-growth gauge choose a baseline nearest 24 hours within ±30 minutes, record the actual span, and leave growth unknown without one. Hold samples at most two hours, matching the current gauge freshness discipline. Do not claim the existing database live-growth sampler already enforces that tighter daily baseline; report its actual span during M1 acceptance.

Initially expose the archive row observationally with unknown/unconfigured target status. Before activation, capacity owners approve explicit byte and growth budgets for the archive scope, set from measured candidates, required replica count and retention horizon; no invented universal archive cap or automatic threshold relaxation. Once configured, health requires complete fresh size **and** known growth within those budgets, while a known byte breach independently reports bad. Unknown growth/size cannot prove success. Keep existing database 8 GB / 300 MB/day objectives and cold-read/latency correctness visible alongside archive capacity. No M1 claim can hide archive growth or missing archive telemetry.

## 10. Capacity-approved offline candidates and target choice

No sizing run is performed by this design task. M0 does not authorize a live copy or VACUUM: its capacity observations found the portable member could not fit a second full database, and one runner's margin above its floor was smaller than its database. Fresh free space is not proof of admission or future headroom.

Before construction, obtain capacity-owner approval for a named offline storage location and budget covering source snapshot with WAL/shards/bodies/ledgers, immutable archive output, **both** 30-day and 14-day candidate stores, overlays/indexes, temporary construction/sort space, restore scratch, rollback source and a growth margin. Model peak coexistence, not just final size. Reserve the budget through the established capacity gate; record declared memory/disk, floor, available capacity, hold and receipt. Sequential candidate construction is acceptable only with a durable retained source and fully measured peak; do not delete someone else's data to fit a candidate. No live database mutations, checkpoints, VACUUM or schema operations are part of the experiment.

Use one consistent approved source snapshot/cut per member for both W=30d and W=14d. Include the whole source state, not only the largest three relations. Construct exact archive segments and witness-closed hot stores, with current representation changes explicitly identified. Keep normalized signature-reference savings disjoint from cold-removal savings; measure new dictionary/index/overlay overhead. Unknown/incomplete repair or security closure remains hot.

Required result table, one row per member and window:

| Field | Measurement required |
| --- | --- |
| Source | Exact revision/schema/rules, snapshot cut/frontier, complete source inventory and digest |
| Candidate hot | Occupied btree bytes by table/index, live-page bytes, file/WAL bytes after offline sealing, protected-old/recent/other byte categories |
| Archive | Sealed exact bytes, catalogue/index/delta bytes, duplicate retained generations, required replica/backup bytes |
| Capacity | Maximum simultaneous allocated bytes and memory, temporary peak and remaining approved floor |
| Correctness | Exact wire coverage and restore comparisons, fold/authority/security/replay roots, cold-peer range results |
| Performance | Native hot/cold read and repair/export latency, writer holds, construction duty/WAL exposure under representative demand |
| Growth | Observed representative ingest/steady-state bytes and modeled margin, with source intervals and limitations |

Select **30 days if it meets every correctness, capacity, latency and ≤8 GB hot criterion with agreed margin**; otherwise select 14 days only if it independently meets them and supports the required 30-day cold-peer return. If both fit, the longer hot window reduces cold dependencies. If neither fits, report the measured retained-set blocker; do not silently shrink below 14 days, relax 8 GB, shorten security history or invoke age-only removal. A single member that fails blocks fleet activation. Static candidate size cannot prove ≤300 MB/day: after a separately authorized rollout, require seven covered days on all members at unchanged representative demand, native growth spans disclosed, fresh archive accounting and no durability/replay/latency regression. The primitive PR claims neither M1 nor M2.

## 11. PR stack and reader-first release gates

Each PR must have an independently reviewable contract and pass its own required evidence. Landing the primitive does not authorize later phases.

1. **Primitive and design:** exact sealed wire codec, external-digest reader, indexed lookup, transactional restore and codec/restore acceptance cases. No production consumer/default/marker/retention changes.
2. **Native read union and overlay machinery, inactive:** catalogue/index contract, all wire consumers migrated, cold mutation and repaired exclusion semantics, explicit unavailability behavior. Default stores still write/read ordinary hot rows with no automatic archive conversion.
3. **Backup and native metrics readers, inactive:** portable archive-complete export/restore, checkpoint/ledger/shard/body invariants, separate archive native row and budget semantics. Publish reader capability and any new backup-format support without requiring new output by default.
4. **Witness/planning and offline construction:** deterministic 30d/14d planners extending existing guard/proof contracts, capacity-approved measured candidates and cold/N-1 test evidence. If semantic rules change, stage the new rules reader before enabling new seals.
5. **Published reader release and compatibility qualification:** ensure the supported published N-1, not merely main or an open PR, includes every native read/backup/repair capability needed by the future format. Preserve the current default storage writer and persistence layout throughout this stage, analogous to #6765's v20 default. Advertised support alone is insufficient without exact-version compatibility evidence and active-member coverage.
6. **Separate explicit activation PR/release:** owning authority enables bounded archival and publication only after approved budgets, reader coverage, complete backup/restore proof and measured selection. Archive-only shadow operation precedes any hot removal. Recheck active-member capability, matching required checkpoint terms and in-flight work at the activation transaction.
7. **Separately reviewed hot relocation/retirement:** verify durable segments, catalogue coverage and pinned witnesses, then remove only eligible hot duplicates in bounded transactions with capture/generation fencing. Compact/swap the hot database only under a capacity-approved deployment/rollback plan; freed SQLite pages are not filesystem savings until measured. Historical claim/cache retirement follows its own migrated-reader gates.

Release N and N-1 must coexist for at least 14 days. A member returning after 30 days offline must either operate through supported native history transfer or receive an explicit compatible checkpoint/refusal flow that does not corrupt or permanently block work. Never trim evidence an admitted old reader still needs. A rules digest mismatch can legitimately defer sealing; it must not block ordinary replication, schedules or ownership transfer. Storage, backup, wire capability and rules versions are independent axes, not a single release-tag comparison.

### Reversibility

Before hot removal, disable the new writer/converter and continue serving unchanged hot rows; retain segment files for diagnosis. No user_version editing, relabelling or dropping archive references is rollback.

After hot removal, a feature flag can stop further moves but cannot make an archive-unaware binary safe. Roll back only to a supporting reader release while continuing base+overlay reads, or rehydrate a complete hot store offline from verified segments plus deltas and validated ledgers/shards under an approved producer-fencing plan. Preserve the pre-cutover store and boundary evidence until the rollback policy expires. Rehydration must include post-cutover changes; swapping an old snapshot alone loses work and may restart unresolved producers. If required data is missing, hold/fail closed and roll forward with repair rather than claiming rollback succeeded.

## 12. Authority constraints

This design grants no live operational permission. Code authors do not approve their own capacity floor bypass, archive retention/destruction, format activation or producer-state recovery. Product/storage owners approve reader/witness contracts; capacity owners approve location/peak budgets; operations owns snapshots, canary/rollout, fencing, activation and recovery; the release authority owns published N-1 compatibility and required evidence. Current root/delegation/revocation and membership controls remain authoritative. Content addressing does not create trust or authorization.

No automatic source export may silently call a migration-capable `Store::open` on a live database. The existing offline backup path explicitly requires a private copy because opening it may migrate/repair/replay. Archive engineering must not bypass that boundary. No daemon deployment, live copy, delete, checkpoint, VACUUM, restart or metric-target change is requested here.

## 13. Acceptance and test matrix

These are required proofs for the named phase, **not executed results**. Main implementation/CI owners choose the exact existing harness commands. This authoring task ran no builds, tests, linters, formatters, smoke tests or live operations.

| Phase | Cases | Required outcome |
| --- | --- | --- |
| Primitive exactness | All columns, NULL/empty/TEXT/BLOB distinctions, integer edges, REAL storage values where applicable, arbitrary raw bytes, noncanonical signed encodings, unknown/invalid/repaired records, all signatures | Typed-cell and byte-for-byte equality after restore; no payload/signature reserialization |
| Primitive snapshot/order | Explicit sorted selection, missing/duplicate/unsorted IDs, forks at same writer/sequence, shuffled source insertion, concurrent source changes outside the pinned snapshot | Deterministic segments for equal snapshot rows; no missing selection or cross-table torn snapshot |
| Primitive integrity | Wrong external digest, corrupted header/entry/index/footer, truncation at framing boundaries, overlap, bogus offset, count/length overflow, oversize cell, trailing bytes, unknown version | Fail before returning an accepted read/committing restoration; bounded memory and arithmetic |
| Primitive index | First/last/absent identities, every fork, empty selection behavior as documented, largest permitted index | Exact lookup and absence; no linear full-payload scan per lookup; limits enforced |
| Primitive restore | Conflict in final record/signature, destination constraint errors, repeated restore according to documented duplicate policy | Whole transaction rolls back; no REPLACE overwrite or silent coercion; only three wire tables affected |
| Primitive inactivity | Normal new/existing store open and ordinary writes with module linked | No archive files, schema/version change, retention or daemon writer activation |
| Publication | Crash before/after file sync, rename/link, directory sync, catalogue commit; digest-path collision | Only complete durable objects referenced; no overwrite; unpublished data never mistaken for a seal |
| Native reader union | Hot-only, cold-only, duplicates, forked history, catalogue rebuild, missing segment, stale generation | Same complete answers; contradictions/unavailability explicit; no false not-found/caught-up |
| Range/repair | Page boundaries at forks, predecessor gaps, empty peer, >30-day-offline peer, cold unknown-to-valid/repair, new signatures, concurrent compaction | Exact original transfer, stable cursors, current repair exclusion and no resurrection/lost delta |
| Witness closure | Every existing checkpoint guard and rule, transitive citations, multi-envelope claims, resource/status/incarnation history, active runtime and operations, unknown kind | Reader-selected witnesses retained, deterministic fixed point, same graph/fold/proof answers |
| Security | Revocation/delegation changes affecting cold claims, losing signature candidates, nonce reuse against old history, operation replay/expiry | Same current authority and global replay rejection; no cache/age bypass |
| Backup/restore | Checkpoint pages/certificates, ledger roots and immutable format cuts, shard ordering/local state, bodies, quarantine, missing archive object, interrupted publication | Complete verified product restore or explicit refusal; no healthy partial graph or unauthorized producer restart |
| Compatibility | Exact published N/N-1 in both peer/owner roles, 14-day coexistence, 30-day offline return, unsupported required codec/backup/rules, restart and rollback | No lost/duplicate work or blocked schedules; explicit refusal where required; no marker downgrade |
| Offline sizing | Every member, common source per 30d and 14d candidate, full table/index inventory, archive/delta/replica overhead and temporary peak | Measured target selection, approved capacity floors, ≤8 GB claim only when demonstrated |
| Native accounting | Empty configured scope, absent root, failed stat/list, missing target, duplicate references, moved scope, restart/clock rollback, absent/stale baseline | Correct disjoint bytes; fresh complete coverage; unknown never passes; actual growth span exposed |
| Activation | Archive shadow coverage, witness revalidation, checkpoint mutation epoch, canary and seven-day covered growth/latency observation | No hot removal before all gates; ≤300 MB/day only from measured covered demand; archive growth remains visible |
| Recovery | Disable before removal, supporting-reader rollback after removal, offline full rehydration with post-cutover deltas and unresolved producers | Reversible under stated boundaries without history loss or unsafe execution |

## 14. Completion boundaries

The first PR is done when the exact three-table primitive, integrity/read/restore contracts, inactive-default behavior and its acceptance cases are implemented and independently reviewed. Production M1/M2 remain unachieved until native consumers, overlays, witness closure, backup, archive accounting, measured 30d/14d construction, reader-first published compatibility and separately authorized activation all meet their gates. Missing proof is reported as missing proof, not converted into a green objective.
