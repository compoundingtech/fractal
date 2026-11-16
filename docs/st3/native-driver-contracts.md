# Retained native driver contracts

These requirements survive retirement of the catalog CLI and its VRS. They apply
to the shared driver and current runtime boundaries named below. The
[native invariants](invariants.md) identify executable proofs and their limits;
this document does not make a passing source-reference check an execution result.

## Process ownership and launch bytes

The control plane may be replaced without killing or replacing its durable work.
Stopping that work requires an explicit lifecycle action. Parent-bound test
helpers and durable work have separate lifetimes, and cleanup must target the
owned generation without touching neighboring work. The real-systemd controls in
[the work-scope invariant](invariants.md) exercise parent-service death, literal
environment values and escaped-descendant retirement.

Every added launch wrapper preserves the program and ordered arguments as opaque
OS strings. Systemd v254 and newer require `--expand-environment=no` before the
`--` separator. Versions 236–253 directly execute scope arguments and omit that
unsupported option. Version and user-manager probes must establish scope support;
the documented degraded mode grants no scope-isolation proof. A fixed description
keeps task argument bytes out of the D-Bus description. Neither shell quoting nor
dollar/percent rewriting may change caller-owned bytes. The implementations are
[`fractal-runtime::isolate`](../../crates/fractal-runtime/src/isolate.rs) and the
[retained driver wrapper](../../crates/fractal-drivers/src/isolate.rs).

Launch environment values travel in the inherited environment, with only variable
names in generated PTY environment arguments. Canonical runtime placement owns
`PTY_ROOT`; the deprecated `PTY_SESSION_DIR` is removed after the overlay. Launcher
failure claims carry bounded diagnostic kinds and exit status rather than raw
captured output. This does not remove values from explicitly authored arguments
or from authored replicated declarations.

Bounded helper teardown includes the helper's process group and descendants. The
direct child is reaped or its wait ownership is transferred to the shared reaper.
[`fractal-drivers::process`](../../crates/fractal-drivers/src/process.rs) retains
at most the last 256 KiB of each diagnostic stream and reports truncation.
Structured payload callers opt into an explicitly named full-stdout helper;
stderr remains bounded. The cap, teardown and reaping requirements formerly
identified as R32, R34 and decision 0007 remain in force for these shared helpers.

## Harness evidence

The native portion of OHS-R01–R16 and decision 0006 belongs to the
[observed-state, stop-ordering, diagnostics and delivery-input invariants](invariants.md)
and [seat state authority](seat-state-truth.md). Only the owning driver, channel or
hook publishes state. Missing evidence never proves idle. `unknown` is derived,
freshness uses embedded time rather than mtime, and evidence loss stops
heartbeats. A positively proved dead session can narrow a live observation; an
indeterminate liveness probe cannot prove death. A terminal observation survives
its writer, and stop escalation must preserve terminal evidence first.

Diagnostic stages, reasons and sources have closed vocabularies. Missing,
malformed, foreign and indeterminate records are distinct from healthy evidence.
Recovery clears only its own stage. Admission failures outrank credential
rejection, which outranks delivery/read-back symptoms. Credential rejection needs
typed producer evidence; provider prose supplies no authority. The current
[admission and repair policy](seat-deploys.md) owns current behavior, including
explicit person-authored admission exceptions. Retired catalog roster commands
and historical permission-downgrade proposals supply no current API contract.
Bounded diagnostic telemetry belongs to [observability](observability.md).

HC-R01–R18 and decision 0014's surviving numeric-context requirements belong to
the [context, producer-arithmetic and status-line invariants](invariants.md).
Each maintained harness supplies its own numerator, window and compaction facts,
qualified by a version-pinned fixture. Unknown values stay absent, above-window
values stay unclamped, and account totals do not become occupancy. Readings remain
advisory and independent of categorical activity; stale values retain their age.
Origin time owns freshness. Bucket changes, compaction and the named heartbeat
bound writes; record updates must not wake their own delivery pump. Incarnation
is provenance for numeric readings; the relaunch claim removes the predecessor's
context. The status-line tee records and forwards exact input to the operator's
renderer. Without a usable renderer, stdout stays empty rather than exposing the
machine payload. These requirements do not retain the retired catalog's transport
include list, roster wire format or Doctor interface.

## Native channel and admission decisions

Installed hook sets remain immutable and verified. Selecting a new set does not
invalidate a set used by a running binary; the current upgrade and selection
rules live in [seat deploys](seat-deploys.md). Restored session context uses the
provider's model-visible contract rather than terminal inference. A residency
resume must preserve the exact native session and refuse foreign or malformed
checkpoints before binding or delivery, as described in [omp](omp.md). Durable
mail waits for readiness rather than authorizing a replacement conversation.

Decision 0005's pi extension and decision 0007's separate omp extension remain
native transports. Rust owns delivery policy; extensions carry correlated frames.
No screen classifier supplies native delivery readiness. A lost channel supplies
no positive delivery or idle evidence. The current channel binding, checkpoint,
observed-state and restart rules are in [seat deploys](seat-deploys.md),
[harness access](harness-access.md), and [omp](omp.md).

OMP-R01–R06 require its own shipped extension, positive post-`agent_end` idle
evidence, correlated human-approval state, exact-build measured admission and
typed credential classification. The complete maintained launch, admission,
exception, native error and modal-hold contracts are in [omp](omp.md). Historical
captures qualify only their named builds; they do not admit a new executable or
extension. Codex's schema admission (R38) and the other native gates are described
in [seat deploys](seat-deploys.md); schema compatibility alone proves no turn or
receipt behavior.

The rolling-release obligation R49 lives in
[compatibility](compatibility.md#required-rolling-window), with its implementation
gap explicitly open. DELTA-006 remains at its
[migration deletion trigger](../vrs/.delta/DELTA-006-delivery-state-v1-arm.md), with
both seven-day zero observations and a separate ratified rollback decision still
required.

## Historical provenance

The retired VRS and captures remain readable at
[main commit bb51bdcb0cd549b06c298d81c1907a7fd35f0c83](https://github.com/compoundingtech/fractal/tree/bb51bdcb0cd549b06c298d81c1907a7fd35f0c83/docs/vrs).
Source comments that cite measured harness captures use that same immutable
commit. Embedded omp hook files keep their original bytes, including relative
historical citations; resolve those paths against this commit. This preserves
their content-addressed hook identity while retaining the measurements' origin.
