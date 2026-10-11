# Dormant managed-boundary fixtures

This private module is compiled only through the cfg(test) native_source module.
Only its fixtures call install_transaction_hooks on a private file-backed native
Store. Nothing changes Runtime, Store factories, sqlite.rs, public Doctor, migrations
or the adapter-slot contract. It neither bootstraps nonempty history nor supplies
production authority/coverage, cost, latency or feasibility evidence. #2077/#2091
and the independently owned replica admission cadence remain separate.

Prepare runs once before the outer transaction's jobs, and finalize once after all
surviving savepoints and before COMMIT. The adapter owns one immutable NativeSource
and counters; callbacks own no Store/WriterConnection. Installing over an occupied
adapter/finalizer slot fails. This cannot be privately installed alongside a future
production registry: one shared source adapter and slot ownership need review.

Bootstrap is a closed factory with an explicit one-use token bound to the calling
thread, armed before the first managed lent transaction. An uninitialized cell does
not itself permit work. Premature queued jobs fail prepare without running; an armed
transaction which omits initialization fails finalize. The token is not reusable.
The caller attaches the existing empty-source schema, puts the source in the callback
cell, sets its transactional scope marker, and optionally runs the native seed helper.
No reader/caller is exposed during that sequence. A failed factory is discarded;
an in-memory cell cannot attest a rolled-back schema. Store::open has already created
its normal schema before this closed test factory, but no fixture source attestation
is issued until the hooks are installed and the existing full emptiness proof passes.

The private guard uses versioned managed_scope=0/1. Prepare checks committed pending,
gap, scope and immutable identity before jobs. Preexisting raw pending or leaked scope
makes a permanent persisted source gap; it is never extracted as this transaction's
eligible input. Logical coverage failures gap the source while valid native admission
may commit. Propagated SQLite errors (including the control's post-operator root publication fault),
injected finalizer failure/panic and failed COMMIT
abort the whole batch and every caller acknowledgement. Rollback/drop of a lent
transaction does not finalize; a caller savepoint rollback removes its capture too.
Read-only pinned connections reject pending, gap or active scope without hooks or
repair, even when an old Installer root still exists. No line ever says pass.

The batch control holds a lent writer and synchronously sends three public WriterJob
messages. Each send has returned before the lend is released: enqueueing is known,
not inferred from sleep. The jobs use a native claim-record helper and captured native
updates, including a later dependent exit and a failed savepoint. The real 50 ms writer
batch window can split them under contention. Up to four fresh SETUP attempts may
establish one batch from the library's batch counters; split layouts and actual
acknowledgements are printed. Only layout setup is retried. Once one batch is observed,
semantic/output/failure assertions run once and any failure is final. The two-caller
finalizer failure/panic cases use the same bound. This is not a CI assertion retry or a
new scheduler. Direct native Store::append_claim exercises the default commit-context
path separately, including local render.applied and latest workspace.observed branches.
The fixture does not claim signed-message/authorization closure coverage.

Raw autocommit, SQL BEGIN/COMMIT, unchecked/direct rusqlite transactions, a separate
configured native connection and raw COMMIT inside a managed wrapper all bypass hooks.
Capture leaves unknown, and the next managed prepare permanently gaps the source.
A raw COMMIT already made cannot be undone by the wrapper's subsequent error.
Hooks are in memory: Store::open may do captured projection work BEFORE reinstalling
them. The reopen control forces the actual small private native open_projections replay
by deleting its canonical-rules meta marker, then checks pending and a permanent gap.
Clean reopen is conditional on no captured open-time writes. A future production
factory must install its hooks before any managed writer input; installation by itself
is not readiness and cannot reinterpret unscoped open-time work as certified.

Eight managed_gate_* controls are separate from the unchanged original fourteen
native_gate_* name roster and canonical oracle. Work control sizes are 0 / 1024 /
100000 unrelated rows. It reports each prepare, finalize marker read, source finish,
marker clear, whole operation and finalize count. Targets: clean prepare
<=3 statements/120 VM/zero fullscan; finalize marker read 1/40/0; old idle source finish
1/40/0; full source maintenance <=96/20000/0; marker clear 1/40/0. The marker read is
INCLUDED in finalization, separately from source maintenance and clearing. No figure
claims the complete finalizer is the old one-statement idle finish. Whole-operation
accounting includes BEGIN, jobs, prepare, finalize and COMMIT on the lent caller thread;
lend wait/return, setup and reader proofs are excluded. It reports actual sorts and
autoindex rows too. Nothing silently raises a ceiling on a failure. No-binding uses
the already reviewed library loads, not a claimed zero-cost or new callback.

The fixture retains header-only typeof/octet_length reads, byte/row/decode quotas,
unsupported-shape unknown, replacement capture, rank/repair/checkpoint gaps and the
native oracle. Storage-version, canonical_audit/digests, old-binary/rollback policy,
production trigger cost/index loss, exhaustive writer lifecycle and deadline/hold/ACK
proofs remain held for a separate production bridge review.

Intake asked that controls 3-5 execute against whichever of #1909 or #1865 lands first.
At the source base e1d7b8330 both are open/unmerged. Their API contract is unchanged;
no shared library edit is taken here. Current-main composition and a new natural hosted
run after the first landing must cover the affected batch/failure/lent controls before
merge. Existing executions on older source never transfer to this slice.

Library limitation retained: Installer::record catches an operator.apply error and
fences its root, including errors originating in operator SQL. This fixture does not
change that policy or claim typed operator-storage classification. Its real storage
negative injects the error into root publication AFTER apply, where it propagates.
The future production requirement to abort on storage faults while fencing only
logical coverage failures still needs a typed/error-disposition seam review.

At the prior #2293/#2301 source, while pending remains set, prepare repeats its gap
UPDATE and Installer::source_gap on every managed transaction. The private successor
below adds a transaction-local skip predicate; its savings remain UNMEASURED. A saved-binding mismatch currently makes prepare error
and fails every write on this private Store; this abort behavior must not carry into a
production coverage design. Neither limitation is production-approved by this slice.

Hosted run 38129863239 at public c17814ab2 / archive source 5d69e5bf8 /
tree bd9f10bc executed all fourteen original native controls with initial PASS and
seven of eight managed controls with initial PASS. The native append control failed
all three attempts before its first append: it incorrectly cited desired.revision
(the content digest) instead of desired.claim_id (the stored declaration). The
successor fixes that fixture link and adds an explicit missing-evidence admission
refusal with full committed-state equality; its outcomes are not yet executed.
The old failure and its two retries remain attributed to c17814ab2.

The c178 work control initially passed at all three sizes. Measured clean prepare
was 2 statements / 57 VM, finalize marker read 1 / 8, idle source finish 1 / 23,
and marker clear 1 / 33. Whole idle was 7 / 127 at every size; whole unrelated
update was 8 / 399 at zero and 8 / 1276 at 1024 and 100000; whole semantic no-op
was 57 / 2288 at zero and 57 / 2289 at both larger sizes. No-op source finish
was 50 / 1092 at zero and 50 / 1093 at the larger sizes. All measured paths
had zero fullscan, sorts and autoindex rows. These are private fixture SQL work,
not production or elapsed-time evidence, and do not transfer execution to the
corrected successor. Required linux-tests was red only on the append fixture;
optional perf-cost separately failed messaging_faults::link_minutes recovery.

Private operator disposition successor (all new outcomes UNRUN): managed fixtures
now install a TerminalGates-delegating operator with a versioned policy fingerprint
and one adapter-owned error cell. The original fourteen still use the unchanged
TerminalGates and default NativeSource::attach. No Installer/Operator API, hook
library, Runtime, Store factory or public Doctor is changed. The private source
factory accepts the managed fixture's Installer before its EMPTY publication;
matching reopen uses the same fingerprint and cannot adopt the old policy.

The callback's primary SQLite code, with the extended code preserved, determines
the private result: BUSY/LOCKED/NOMEM/READONLY/INTERRUPT/IOERR/CORRUPT/FULL/CANTOPEN/
NOTADB preserve the original error and abort through the existing outer finalizer;
CONSTRAINT and its extended variants fence loudly and retain UNKNOWN while valid
source admission commits; other typed SQL/no-code rusqlite errors abort as
UNCLASSIFIED. Pure logical shape/completeness/cap and pre-apply version errors keep
the existing logical fence behavior. Errors in Installer/source SQL outside apply
retain their existing propagation policy. This is not a blanket change to all
SQLite failures; the old root-publication ABORT remains an outer transaction
failure control, not callback infrastructure proof.

Prepare clears stale error memory before any job. Finalize drains the cell EVEN
IF source.finish failed, so an original FULL wins over subsequent missing-savepoint
cleanup errors. First abort-required error wins; reentry and poisoned cells refuse.
There is no global cell, Store/connection retention, error-history collection or
read-side reset. Constraint metadata additionally persists a fixed bounded numeric
reason in the private guard/root (two extra error-path SQL writes); normal work
ceilings remain unchanged. Logical faults never become healthy or silently vanish.

Six new managed_operator_error_* controls cover typed/extended/synthetic inputs,
real FULL/all-caller rollback/recovery, real witness CONSTRAINT/admission/UNKNOWN,
first-fault/reentry/lent reset/isolation, bootstrap/reopen/lifetime, and old-policy
refusal. The real FULL probe is one fixed 64KiB zeroblob INSERT in a private probe
table INSIDE apply, not a real-disk or witness-insert-specific failure. Setup creates
the table after native seed, VACUUMs the small private fixture, requires zero free
pages, then caps max_page_count to page_count. It prints page_count/max/free/page
size/journal mode before and after fault and recovery. An earlier native failure,
wrong fault code/origin, or missing fault is a failed control, never a success.
Only the inherited four-attempt fresh batch-LAYOUT setup may repeat. Semantic
assertions never retry. The two queued caller jobs must both finish their native
updates, share one finalizer, receive Err, and leave all committed SQL state equal.
The limit is restored and an actual public batched native append must subsequently
commit; memory/root restoration alone is insufficient writer recovery evidence.
One anchored nextest success-output override exposes this setup; it changes no
selection, retry, thread or ceiling. All original14 and managed8 plus new6 must have
source-bound hosted outcomes; no prior result transfers to this successor.

Held for production: aborting all unclassified SQLite errors (ERROR/SCHEMA/MISUSE,
for example) may turn an adapter SQL bug into an outage for every writer caller.
The private fail-closed choice requires its own production review, as do error-cell
allocation/message bounds, source completeness, trigger/index cost, binding/reopen,
storage-version/canonical_audit/digests, writer hold/latency and Doctor activation.
Library root/cleanup errors still do not supply an abort-only capability to arbitrary
callers which catch an Err. Shared policy consolidation is deferred unless another
operator needs it. Source review and rustfmt are not Rust/SQLite execution.


Private already-gapped successor (all six new controls UNRUN): one transaction-local
metadata SELECT avoids repeated diagnostic UPDATE statements when the persisted
source is unavailable, every same-source root is unready with the exact bounded
guard reason, and no same-source scan/catchup job exists. The predicate is the
complement of Installer::source_gap's three WHERE clauses. Existing source/job/root
indexes serve it; EXPLAIN QUERY PLAN and bytecode are printed by the work control.
NULL, non-text or oversized root errors are mismatches and call source_gap, never
refusals. The guard alone retains its 512-byte text limit. New logical refusal text
is truncated at a UTF-8 boundary to <=512 bytes before persistence. Installer's
first-1024-character transform is then identity. Missing source or invalid source
availability deliberately refuses this private transaction; that stricter choice,
binding corruption and arbitrary same-source metadata fanout remain held for
production. No new schema, library policy, reader call or production hook is added.

The helper is called only from prepare's pending/active branch, finish's preexisting
gap and logical refusal branches, and finalize's explicit constraint-reason branch.
Clean prepare and idle finish never call it and remain exactly 2 and 1 statements.
Stable pending/gap skips the diagnostic guard UPDATE too. Scope entry and clear
still write: an empty committed transaction changes exactly two rows; a private
unrelated meta UPDATE changes exactly three. Each uses connection total_changes
alongside fresh pinned persisted-state equality. total_changes includes rolled-back
attempts; it is neither commit durability nor a claim that zero-row DML was elided.
The SELECT decides whether the existing three UPDATE statements execute at all.
It neither repairs evidence nor certifies the current graph; every gap stays UNKNOWN.

Stable-gap statement arithmetic is prepare 3 + finish 2 + marker read 1 + marker
clear 1 = 7 phase statements, plus BEGIN/COMMIT = whole idle 9 (clean phases 5,
whole idle 7). Predeclared private stable limits are prepare <=3/240, finish <=2/160,
whole idle <=9/500, with zero fullscan, sort and autoindex at 0/1024/100000 unrelated
claims. Marker, clean and maintenance ceilings are unchanged. The bytecode control
also derives a tighter fixture-only helper VM bound: EXPLAIN program length +16
for the two IN probes and final cursor-next branches in the closed one-root,
one-published-job layout. Actual prepare/finish/whole VM must additionally stay
under old clean 57/23/127 plus one/one/two helper bounds. These are source-derived
control obligations, not measured savings; neither the derivation nor ceilings may
expand after a failure. Header-only Column flags must remain present. The work
interval includes BEGIN through COMMIT on the lent caller; acquisition/return,
setup and read proofs are excluded. The unrelated caller is a fixed meta UPDATE,
not a public append or a production write-cost claim.

The six managed_gap_* controls cover stable identity and exact changes, a legitimate
later capture reason/status transition, partial fences including all same-source
roots and jobs, caller savepoint/lent rollback/drop and diagnostic/finalizer/COMMIT
failure with writer recovery, an actual multibyte logical refusal, and clean-path
statement equality plus missing/invalid metadata refusal. A diagnostic trigger is
UPDATE OF available only, so capture's revision updates succeed before the intended
persistence fault. Only a fresh batch-layout setup may repeat under the existing
four-attempt bound; semantic assertions never retry. Original 14 native, 8 managed
and 6 operator-error names and adverse assertions remain unchanged. The required
future hosted roster is those original 28 plus these six =34, source-bound with
individual outcomes. #2301's 28 INITIAL PASS belong to its head and do not transfer.
The only nextest addition exposes this one work control's successful diagnostics;
selection, retries, threads and ceilings are unchanged. No Rust or SQL execution,
writer savings, elapsed time, production authority, coverage or feasibility is
claimed by this source preparation. #2077/#2091, link_minutes UNKNOWN/unwaived,
replica ownership and #1909/#1865 composed-control obligations remain separate.
