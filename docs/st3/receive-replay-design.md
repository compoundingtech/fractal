# Live receive: owned sets and the replay fence

Status: accepted design direction from FPL; not implemented. Behavioral changes still require
FPL GO per change. This note changes neither runtime behavior nor existing product requirements.

Review in order: this note, [whole-route continuation](receive-deadline-design.md),
[observation writer work](observation-writer-design.md), then [status reads](read-phase-design.md).
The documents describe separate acceptance gates, not an assertion that receive is bounded.

## Evidence and scope

The exact-source audit at `d2e1d3bb47299827136ce1a168dd81672a19e354` found that normal
owned-set publication emits `owned-set.revised`, replicated admission accepts it, and the
incremental structural allowlist excludes it. See `crates/fractal-store/src/store/owned_sets.rs`
(publication), `crates/fractal-store/src/store.rs` (admission and
`try_project_simple_replication_tx`), and `crates/smallclaims/src/store.rs`
(`project_replication_backlog_chunks`). That last function maps replay decisions and
non-contention incremental errors to full replay while retaining the writer.
The audited fallback also exists at `473a9502a8199a12c03df1fbb67b29816817050d`;
this is not evidence of an r2 regression. The replay fence does not gate r2 on that basis.

The graph-neutral operation-conflict fix removes a particular cause, not this fallback.
Zero captured full-replay logs cannot establish source unreachability. The explicit
operator replay direction in #6330 is design direction, not evidence it shipped.

## Proposed owned-set incremental treatment

Treat an admitted set revision as a structural dependency change, not as graph-neutral data
and not merely as an extra allowlist entry. Reuse the owned-set selection and effective-member
rules in [Owned sets](owned-sets.md#source-ordering-and-replication).

For the affected set, account for both old and new effective membership, including members
learned only from losing branches. Invalidate or rebuild the affected derived state and caches
when the winner, conflict state, or reference completeness changes. A late member reference or
previous revision must revisit the dependent set even if that incoming claim is not itself a
set revision. Keep operation conflict checks and receipt validation intact. Advance the
projection frontier only with the corresponding successful projection transaction.

Selection must retain highest-sequence semantics, equal-sequence conflict holds, missing-reference
holds, exact managed references, synthetic retirement of losing-only members, and pinned active
mission revisions. Incremental arrival order must agree with canonical replay.

Do not promise bounded writer work from set-local rebuilding alone: set history and membership
can themselves be large. The semantic treatment is approved; its bounded execution
belongs to the next note. The fence and incremental owned-set projection must ship together
in one change: the fence alone would stall normal set traffic as recovery-required. If dependency closure cannot be identified and processed safely within
the chosen limits, return explicit recovery-required rather than silently full-replaying.
A justified inability must identify the specific dependency or indivisible fold, not just retain
the old fallback under a new name.

## Proposed exhaustive live-receive fence

Make replay permission an explicit execution-context contract at the shared fallback boundary;
a diagnostic phase string is not permission. Live receive and its automatic deferred retries
cannot invoke full replay, directly or through repair, projection, or recovery helpers.
Full replay belongs only to the explicit local-operator recovery path below.
Moving an unbounded replay to an automatically spawned background task is not this fence.

| Incremental outcome | Live-receive handling |
| --- | --- |
| Projected | Commit the bounded chunk and its frontier. |
| Replay: missing-health, unhealthy-projection, frontier-ahead | Preserve evidence and request explicit recovery; do not infer a healthy frontier. |
| ReplayWithContext: non-incremental-kind, malformed-operation, operation-conflict | Preserve reason/context and pending work; never suppress the guard or run full replay. |
| Non-contention incremental error | Roll back the speculative chunk, retain the typed cause, and record recovery-required. |
| SQLite contention | Roll back the chunk and retain retryable pending work; replay cannot repair lock contention. |

All `Replay` and `ReplayWithContext` values, including future reasons, take the same denied-replay
branch. No wildcard error-to-replay path remains reachable from receive. A replay decision must
roll back speculative projection writes even when represented as a successful enum return.
Previously committed envelope/admission work remains durable. Persist the pending target and
reason with a truthful health/frontier state; never acknowledge projection completion for it.
Distinguish ordinary pending work from recovery-required, so a worker cannot retry a terminal
replay requirement in a tight loop. Restart must preserve that distinction.

## Recovery ownership and public outcome

`fractal replication replay` runs daemon-side for a local operator only. Without `--confirm`,
it prints only an estimate. With `--confirm`, it writes an audit record and rejects concurrent
calls. Automatic background full replay is forbidden. On managed dotfiles hosts, each activation
requires per-host FPL GO and Deployer execution; the person's personal workstation additionally
requires that person's GO. These are activation gates, not permission for an agent to run replay.

Delivery success continues to mean a durable receipt. Show projection pending or recovery-required
separately through existing health/doctor projection fields, with a degraded doctor row naming
the reason and frontier. Entering recovery-required raises a Fractal alert to FPL. Add a wire
field only if the existing distinction cannot express the outcome, after separate compatibility
review.

## Implementation order and proof gate

Receive instrumentation and bulk preparation capture precede implementation on main. The fence
and incremental owned sets require FPL GO for their single change; decide any r3 backport only
afterward. No behavioral implementation is authorized by this documentation change.

Before implementation acceptance, prove producer-to-admission-to-projection behavior for real
owned-set revisions in shuffled order, partial references, losing-only members, equal-sequence
conflicts and restart/checkpoint replay. Compare selected references, affected shared rows and
digests. A real owned-set revision through receive must project incrementally with zero full-replay
calls. Exercise every replay reason and a non-contention incremental failure through receive and
its deferred retry, asserting zero full-replay calls, unchanged failed-chunk frontier, preserved
committed delivery, and visible recovery state. Restart while recovery-required must preserve
that state without a hot retry loop. A new enum reason
must remain denied without adding a reason-specific fence. No proofs were run for this note.
