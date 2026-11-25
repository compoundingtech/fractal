# Checkpoint capture

Read [Checkpoints and trimming](checkpoints.md) first. This note describes how a member's daily cut
captures what it plans from, in bounded memory and bounded disk, and how the capture resumes after
a restart. It changes what a trim costs, not what it drops.

## The budget

**A trim step holds a fixed working set plus the claims of one slot, whatever the size of the
history.** At any moment it holds at most:

- one page (64 rows) of captured rows on their way to disk;
- the claims of one slot, or the status-history sources of one seat, while the planner decides
  them;
- one page (256) of tombstones while the trim records or deletes them;
- fixed-size digest state, SQLite's page caches (bounded by `cache_size`) and its sorter (spilled
  to disk with `temp_store = FILE`).

Everything proportional to the history lives in a staging file on disk: the envelopes, the compact
claim rows, the drop marks and the new tombstones.

`crates/fractal/tests/checkpoint_memory.rs` states the number. On a synthetic history in many small
slots, a full pass raises live heap by at most 8 MiB above its starting point, measured by a
counting allocator, and the same pass on four times the history raises it by less than 1.5 times
as much. The second assertion is the important one: it fails for anything proportional to
history, whatever the constant.

The largest slot is the one term that still grows with history. A slot is one subject's claims of
one kind (and incarnation, code or request), so it is small on every member we know of, and each
trim shrinks it.

## What a cut captures

A cut drops only the kinds in `LOCAL_TRIM_KINDS`, so its capture is scoped to them
(`checkpoint_staging/scoped.rs`). It stages only what judging a claim of those kinds reads, so its
plan drops the same claims and envelopes as a capture of the whole store would:

- the envelopes that hold a record of such a claim, whole, with their records and claims, and the
  envelopes of the claims that share an operation with one, so slots, operations, duplicates,
  envelope and record-count rules see what they would see in a full capture;
- the same for every claim of a context kind (`LOCAL_TRIM_CONTEXT_KINDS`), so the status-history
  rule judges each seat's harness observations with its runtime resets and diagnostics;
- the protections, looked up for the staged claims by each table's claim index, or read from the
  few rows a repair writes;
- each staged writer's newest envelope before the cut, for the writer's-newest guard;
- the citations of the claims the cut could drop, from the citation cache (`local_trim_citations`),
  caught up first, counting a citer only if it is present and inside the cut.

Every kind outside `LOCAL_TRIM_KINDS` that the capture stages is then protected
(`protect_all_but`), and the plan may drop a claim from an envelope it keeps (`split_envelopes`).
`a_local_cut_s_scoped_capture_stages_every_kind_its_rules_read` in `local_trim_tests.rs` checks that
the scoped plan equals a full capture's.

### The citation cache

Citations live in claim bodies (evidence, inputs, review references, lineage) with no index, and
reading every body each day would cost what a full capture does. So each cut first catches the
cache up with the claims that arrived since the last, by a store-index cursor in arrival order,
committing each page of 256 claims with its cursor, then looks its candidates up there. A row is a
hint: a citer counts only if it is still present and inside the cut, and deleting a claim
removes its rows in the same transaction. A cursor that is missing, unreadable, ahead of the store
or written under other rules is not trusted, and the cache is rebuilt.

## The staging file

The capture is pinned to a **store-index cut**: the highest `replica_envelopes` row it covers
(`seal_rowid` in the code), the cut time, and the value of `checkpoint_capture_epoch`. That table's
triggers move the epoch on every change to a row the captured set could include, and nothing else
does. A capture whose epoch still matches is therefore a capture of exactly the same set, however
long ago it started and however many restarts came between. The guard's columns and triggers are
described under "Local checkpoint capture storage" in [schema.md](schema.md).

The capture writes into `capture-<checkpoint>-<row>-scoped.sqlite3` in the member's checkpoint
scratch directory. The file's `meta` row records one identity: the staging format version, the
cut, row, epoch and scope, the rules digest, the build and the digest of the running executable.
The runtime computes each claim's rule and slot key from its body, so a build that changes a rule
or the claim registry never resumes an older file. A file whose identity differs is deleted and
started again; one that matches resumes at its cursor.

Each phase reads one page from the store on its own snapshot, checking the epoch, and writes that
page with the phase's cursor in one staging transaction, so a crash leaves a consistent prefix. A
change to the epoch between pages discards the attempt; three in a row fail the step, and the next
step tries again. The phases, in order: envelopes, records, the canonical order of the records
(SQLite's sorter, on disk), compact claim rows (canonical position, ID, envelope, kind, subject,
actor, operation, validity, field names, rule and slot key or status seat; the body is read once
and dropped), protections, writer heads and citations.

**One capture per directory.** A capture takes an exclusive, non-blocking `flock` on
`capture.lock` for as long as it holds its staging file. A second capture of the same directory
fails at once with "another checkpoint capture is running"; it never waits.

**Lifetime.** A directory holds at most one staging file: opening a capture deletes every other
one. A trim deletes its file when the cut is trimmed or stopped, and a step with no cut to capture
deletes it, so an abandoned capture lasts at most until the next step.

## The plan and the trim read the staging file

- First occurrences, occurrence counts, each writer's newest envelope, the newest measurement of
  each limits account group, closed requests and cited IDs are SQL over the staging file.
- The rules walk the staging file in (slot, canonical position) order and load one slot at a time.
  For a slot whose rule reads bodies, the planner reads those claims back from the store by ID, in
  epoch-checked pages. The status-history rule does the same one seat at a time. Their decisions
  become drop marks.
- The guards and the fixed point (witnesses within each slot, whole operations, record counts) run
  as SQL updates and a slot-ordered walk of the field names, until a round changes nothing.
- New tombstones are written to the staging file. The trim records them from it a page at a time,
  in its 20 ms chunks; deletions then read the tombstones from the store.

A staging file holds its plan once it is made, so a restart before the trim reuses it while the
file's identity matches.

## Disk

The capture is admitted before it creates its scratch directory (`SpaceGuard::admit_scoped_capture`
in `checkpoint_space.rs`). Admission reads the store's page size and count in one SQLite read
snapshot, and the free bytes on the live, scratch and SQLite temp devices. It always leaves the live
writer a reserve of 8 GiB plus max(8 GiB, the allocated live database), and a 256 MiB reaction margin
on each device.

A scoped capture is charged the store's SQLite image scaled by the scoped kinds' share of the
store's claims, times two, at least 1 GiB and at most the image. A staged claim takes far fewer
bytes than a live one (about 1.4 KiB against 17 KiB on a 53 GB member): such a member, whose scoped
kinds hold 15% of its claims, is charged about 16 GB and stages 1.3 GB. The capture checks space
again before each page, so a share that undercounts is stopped, not overrun.

A refusal names the controlling device and term and ends the step with that error. The staging
file keeps its last committed cursor and the store is unchanged (the capture writes nothing to it
but its epoch bounds), so the next step resumes once there is room. A cut already `trimming`
finishes from the store's own tombstones and needs no staging file.

The staged binary answers `--checkpoint-space-gate-capabilities` with one JSON line,
`{"kind":"checkpoint-space-gate-capabilities","version":2}`, before any configuration or database
access, so a deployment can check which admission contract (`CHECKPOINT_SPACE_GATE_VERSION`) it
ships.
