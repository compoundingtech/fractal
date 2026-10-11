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
marker clear, whole operation and finalize count. Targets, still UNRUN: clean prepare
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

Additional fixture limitations from Intake's source review: while pending remains set,
prepare repeats its gap UPDATE and Installer::source_gap on every managed transaction.
A future adapter must short-circuit an already recorded gap, so unrelated writes do
not pay repeated gap writes. A saved-binding mismatch currently makes prepare error
and fails every write on this private Store; this abort behavior must not carry into a
production coverage design. Neither limitation is production-approved by this slice.
