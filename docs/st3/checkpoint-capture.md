# Checkpoint capture in bounded memory

Status: steps 1 and 2 below merged (#2651, #2652); steps 3 and 4 are one change, #2683; step 5
follows its deploy. Issue #2492. Read [Checkpoints and trimming](checkpoints.md) first; this
note changes what a checkpoint pass costs, not what it decides. Seals, verifications, digests,
rules, the proof and the trim keep their meaning, and two members on different sides of this
change agree on the same checkpoint.

## What goes wrong today

A checkpoint pass reads the sealed set in pages of 64 rows, each page on its own snapshot, but it
keeps every page. `checkpoint_sealed_set_attempt` returns a `SealedSet` holding every envelope
before the cut, every admitted claim with its parsed JSON body, every canonical order key and every
tombstone of every earlier checkpoint. `plan_drops` then indexes that set several times over, and
the `DropPlan` it returns holds every new tombstone. The proof copies claim IDs out of the set,
and the trim records tombstones from the plan's vectors. Nothing is released until the pass ends.

On a member with a 10 GB store this is several gigabytes of heap. Three members reached 6–7 GB
within minutes of the daily cut. A restart does not help, because the scheduler deletes its scratch
directory and starts the capture from nothing at startup and every ten minutes after that, so the
memory regrows within minutes. The seal pass also builds a `BTreeSet` of every envelope identity
every ten minutes, which is smaller but is also proportional to history.

## The budget

**A checkpoint pass holds a fixed working set plus the claims of one slot, whatever the size of
the history.** Specifically, at any moment it holds at most:

- one page (64 rows) of captured rows on their way to disk;
- the claims of one slot, or the status-history sources of one seat, while the planner decides
  them;
- the reader answers of one page of subjects while the proof compares them;
- fixed-size digest state, SQLite's page caches (bounded by `cache_size`) and its sorter (spilled
  to disk with `temp_store = FILE`).

Everything proportional to the sealed set lives in a staging file on disk: the envelopes, the
compact claim rows, the drop marks, the new tombstones and the proof's saved answers.

The test (`crates/st3/tests/checkpoint_memory.rs`) states the number. On a synthetic history of
4,800 claims in many small slots, a full pass (capture, plan, prove, trim) raises live heap by at
most 8 MiB above its starting point, measured by a counting allocator. The same pass on four times
the history raises it by less than 1.5 times as much. The staged pass holds under 1 MiB at both
sizes; the old capture held 14 MiB and 54 MiB, about 2.7 KB a claim, so it fails both assertions.
The second assertion is the important one: it fails for anything proportional to history, whatever
the constant. The sizes are small because the test runs in a debug build in CI, where the proof and
the trim cost milliseconds a claim; a run at 200,000 and 800,000 claims took 74 minutes there and
also stayed under 1 MiB.

**Time and I/O.** The capture reads each envelope, record and claim body once, a claim by its ID as
today's capture does. The plan reads bodies again only for slots whose rule reads bodies (harness
observations, usage, account limits, loop state) and for the status-history sources of each seat,
so those kinds are read twice. The staging file is written once and read a few times by the plan's
fixed point. The measurement on two members across a daily cut records, before and after the
change, the wall time of each pass's capture, plan and proof, the process's bytes read and written
(`/proc/PID/io`), and peak RSS. A pass more than twice as slow as before is a reason to stop the
rollout, not a cost of the fix.

The largest slot is the one term that still grows with history. A slot is one subject's claims of
one kind (and incarnation, code or request), so it is small on every member we know of, and each
trim shrinks it. The note names it rather than hiding it; a later change can stream a slot if one
ever becomes large.

## The design

### Seal identities stream in key order

The seal needs a digest of the envelope identities before the cut, sorted by (writer, sequence,
hash). Both sources, `replica_envelopes` and `checkpoint_envelopes`, have that key as their primary
key. The seal pass reads each in key order, 64 rows per snapshot from the last key read, merges
the two streams and feeds the digest as it goes. Each page checks the capture epoch, as capture
pages do today, so a trim or repair between pages restarts the read instead of tearing it. No
staging file is needed and nothing is held.

### Capture stages to disk, with a durable cursor

The capture is pinned to a **store-index cut**: the seal row (`seal_rowid`, the highest
`replica_envelopes` row it covers), the cut time, and the value of `checkpoint_capture_epoch`.
That table and its triggers already exist, and they persist: any change to a row the sealed set
could include bumps the epoch, and nothing else does. A capture whose epoch still matches is
therefore a capture of exactly the same set, however long ago it started and however many times
the daemon restarted since.

The capture writes into a staging SQLite file, `capture-<checkpoint>-<seal_rowid>.sqlite3`, in the
checkpoint scratch directory, which the scheduler no longer deletes at startup (it deletes only
proof copies there). The file's `meta` row records one identity: the staging format version, which
names the staging schema, and the cut, seal row, epoch, rules digest, checkpoint build and the
digest of the running executable. The runtime computes each claim's rule and slot key from its
body, so a build that changes a rule or the claim registry is a different executable and never
resumes an older file. A file whose identity differs is deleted and started again; one that
matches resumes.

**One capture per directory.** A capture takes an exclusive, non-blocking `flock` on
`capture.lock` in its directory for as long as it holds its staging file. A second capture of the
same directory fails at once with "another checkpoint capture is running"; its pass ends with that
error and the next pass tries again. It never waits. The dry run (`st replication checkpoint
plan`) captures into a directory of its own under the scratch directory and deletes its file when
it is done, so it never replaces the scheduler's resumable capture.

**Failure and disk.** A failed staging transaction, such as on a full disk, ends that pass with its
error. The staging file keeps its last committed cursor, and the store is unchanged: the capture
writes nothing to the store but its epoch bounds. The next pass resumes. A trim that is already
`trimming` finishes from the store's own tombstones and needs no staging file. A trim that must
record tombstones again after a crash plans again, which needs the staging file's disk, as the
proof's copy needs disk today.

**Lifetime.** A directory holds at most one staging file. Opening a capture deletes every other
staging file in its directory. A trim deletes its file when it has trimmed, and when its plan no
longer matches the certificate. A pass that has no checkpoint to capture deletes the file. So an
abandoned capture lasts at most until the next pass that captures or finds nothing to do.

Each phase reads one page from the store on its own snapshot (as today, with the epoch check) and
writes that page, together with the phase's cursor, in one staging transaction. A crash leaves a
consistent prefix and its cursor. The phases, in order:

1. **Envelopes.** `replica_envelopes` by rowid up to the seal row: identity, accepted time and
   record count.
2. **Records.** `replica_records` by rowid: the canonical order components, the record's envelope
   and whether it was admitted valid. Records of envelopes at or after the cut are marked late,
   and their envelopes are excluded, as today.
3. **Claims.** The staging file sorts the records in canonical order (SQLite's sorter, on disk).
   The capture walks that order a page at a time, reads each claim's body from the store, and
   writes one compact row: canonical position, ID, envelope, kind, subject, actor, operation,
   validity, whether it is a first occurrence, its field names, and its rule and slot key or
   status seat, which the runtime computes from the body. It also writes the claim's blob hashes,
   the IDs it cites as evidence or pins as a run input, and any request it closes. The body is
   dropped when the page is written.
4. **Protection.** The protected-claim queries, by rowid, into a table of protected IDs.
5. **Earlier tombstones.** `checkpoint_envelopes` and `checkpoint_claims` before the cut, by
   rowid, as today's filter selects them.

### The plan reads the staging file

The planner keeps its rules and its order of decisions; it stops indexing an in-memory vector.

- First occurrences, occurrence counts, the newest envelope of each writer, the newest measurement
  of each limits account group, closed requests and cited IDs are SQL over the staging file.
- The rules walk the staging file in (slot, canonical position) order and load one slot at a time.
  For a slot whose rule reads bodies, the planner reads those claims back from the store by ID,
  in epoch-checked pages, and calls today's keep function on them unchanged. The status-history
  rule does the same one seat at a time. Their decisions become drop marks in the staging file.
- The guards and the fixed point (witnesses within each slot, whole operations, whole envelopes,
  record counts) run as SQL updates and a slot-ordered walk of the field names, until a round
  changes nothing.
- The digests stream: the sealed digest over the envelopes table in key order, the drop digest
  over the union of earlier and new tombstones in key order, and the retained digest over the
  kept first occurrences in ID order. These are the orders today's `BTreeSet`s sort into, so the
  digests are byte for byte the same.
- New tombstones are written to the staging file, reading their fields from the compact rows.

`DropPlan` keeps its counts, `by_kind` and digests. In place of its tombstone vectors it names
the staging file they are in.

The plan is durable too. A staging file holds its plan once it is made, so the next pass after a
proof backoff, and the trim after verification, reuse it while the epoch matches, instead of
capturing and planning again.

**What fences a reused plan.** The trim reuses a staged plan only when both hold:

- the file's identity matches the store now: the same epoch, so no row the set could include has
  changed since the capture (a repair, a late envelope or a trim moves it), and the same rules
  digest and executable, so the same planner made it; and
- the plan's sealed and drop digests equal the certificate's, which every participant published
  after proving its own plan.

Otherwise the trim captures and plans again; and when that plan still differs from the
certificate, the node marks the checkpoint `plan-changed` and adopts a peer's manifest, as today.
The trim does not prove again, as it does not today: the certificate is the proof that every
participant's plan passed.

### The proof and the trim read pages of the plan

- The proof attaches the staging file to its scratch copy and fills its sealed-claims,
  sealed-blobs and sealed-envelopes tables with `INSERT … SELECT`. It records and deletes the
  dropped rows on the copy from the staging tables a page at a time. It reads the readers' answers
  one page of subjects at a time, saves the answers before the drop in the staging file, and
  compares the answers after the drop with them. The global answers stay in memory: they are one
  value.
- The proof-input identity streams over the staged claims in canonical order.
- The trim records tombstones from the staging file in its 20 ms chunks, carrying a cursor as it
  does now. Deletions already read tombstones from the store.

### What does not change

The sealed set, the drop set, every digest, the proof's answers and the trim's effect are the
same functions of the same rows. The tests show this by planning the same stores with the old
in-memory planner, kept as a test-only reference, and requiring identical plans. Manifest
adoption from a peer keeps holding the manifest in memory; it is bounded by one checkpoint's
drops and is a separate change. So is the unbounded per-envelope select in
`checkpoint_trim.rs` that #2492 also names.

## Cost on disk

The staging file holds about 200 bytes per sealed claim and per envelope, roughly 1 GB for a
store with three million claims, plus the sorter's temporary space while it sorts. It is deleted
when the checkpoint is trimmed or abandoned, or when its store-index cut no longer matches.
The proof's copy needs disk as it does today.

## Order of changes

1. This note.
2. Seal identities in key order. The seal pass runs every ten minutes, so it is the first fix.
3. The staging capture, the staging planner, and the memory budget test.
4. The proof and the trim reading the staged plan, the scheduler keeping the staging directory,
   and reuse of a durable plan. Steps 3 and 4 shipped together, since the plan in step 3 has no
   reader until step 4.
5. A measurement before and after on two members across a daily cut, posted to #2492.
