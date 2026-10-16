# Resource observation preparation: unresolved currentness fence

This private source unit is pinned to installed main
`4b04f5bdbefb9d7ecac427711a4449866a64f03e`. It takes the explicit blocker branch
of Root's `message/84b33f26f5e6111b`. It does **not** move production work, add a
prepared-write API, or qualify a performance fix. All four authored controls are
**UNRUN**, including their first compile. Only rustfmt parsing and `git diff --check`
have run. No SQLite, Cargo, local compiler, daemon read, profile, CI action or
publication was performed.

The starting evidence is
`doc/fleet/smalltalk/speed/2026-10-16/writer-resource-source-trace@479ded00abd546c1cf6dfa1632ccc781ceffa509b15dfe3760ce41c459e27a39`.
The retained 03:20:22.190Z event reports a 9,951.192 ms batch, a 9,869.132 ms
longest job, and seven jobs. The installed call site is
`Store::record_resource_observation`. This identifies a job, not its observer or
the expensive inner operation. Only 82.060 ms remain for the other six jobs and
the surrounding transaction machinery together. The observation function is
byte-identical in the old ae128a22 and installed 4b04 cuts.

## Exact obstruction

There is no established, bounded, keyed currentness token in this cut for the
canonical actual-state inputs used by this method. The cheap rows already
available are not such a token:

* `MAX(claims.store_index)` and `desired.claim_id` do not change when a retained
  actual-state source becomes repaired. The actual fold excludes
  `projection_digest_repaired_claims`. Observer/subscription state is not covered
  by `local_resource_projection_pending`, whose claim filters name
  `resource.observed` only. The existing `latest_actual_value` append-index cache
  therefore cannot be substituted for the uncached fold used by this method as
  a new authority fence.
* `resource_observations` is a latest-value projection. Its clean marker is not a
  certificate of all source coverage. Its pending triggers cover claim insert/
  delete and record insert/update for resource claims. They do not establish
  keyed coverage of every input to canonical ordering, both sides of record
  identity changes, record deletion, or observer/subscription actuals. The
  canonical comparator includes claim time, batch origin and sequence, and
  record position (with a legacy-position fallback). A prepared dependency set
  must cover these inputs or prove their immutability on every admitted path.
* `projection_digest_generation` is a global diagnostic generation, not a keyed
  authority token. It detects the repair case in the first control, but its
  trigger registry does not cover all physical ordering inputs. The second
  control changes batch ordering without changing that generation. This is an
  injected physical source case, **not** a claim that an authenticated producer
  is permitted to rewrite a signed batch or that this happened in the incident.
  A narrower candidate still needs an explicit exclusion/coverage proof for
  those paths.
* A committed-write observer publishes after the transaction resolves. It cannot
  fence a dependency changed earlier in the same outer transaction. The third
  control retains the observer handle, mutates the repair source inside a
  managed job, and compares that transaction's actual fold with the unchanged
  commit counter. It does not hold a reader across a writer loan.
* Installing a whole-store writer epoch would be a different native source
  adapter with coverage and liveness obligations. Invalidating a multi-second
  preparation on every unrelated write gives no progress argument on this busy
  daemon. The existing global counters alone neither establish complete
  coverage nor solve that problem. No such epoch or retry loop was added.

The missing prerequisite is a qualified keyed invalidation/readiness adapter for
the entire dependency closure, or an equivalent existing adapter whose source
coverage and invariants can actually be established. It must observe repair,
import and removal, canonical ordering, opener inputs, dirty projection state,
observer revision/cancellation and subscription/recipient changes **before a
later job in the same batch validates**. It must also bind to this store and
epoch and refuse schema/coverage gaps. The writer must compare that token with
bounded point work and return a stale-plan result, never fold history there.
No existing adapter has been proved to provide that closure in this unit.
Creating persisted generation tables/triggers would exceed the explicit
no-new-storage condition; an in-memory native adapter would require its actual
source-coverage/control design rather than an assumed commit hint. Neither was
silently introduced.

## Authored source controls

The complete bodies are in
`crates/st3/src/store/resource_preparation_fence_tests.rs`, registered only under
`cfg(test)`. The helper is named `IncompleteFence`; it must never be used to
authorize an effect. All four controls are UNRUN.

1. `repairing_observer_health_is_invisible_to_append_and_resource_projection_fences`
   holds retained claims and the candidate token fixed, excludes the healthy
   state through the actual repair-cache seam, and requires the uncached actual
   fold to become degraded. It separately requires the global digest generation
   to change, so it does not falsely say that generation misses this repair.
2. `canonical_batch_order_is_not_covered_by_the_global_projection_generation`
   constructs an invented equal-time ordering case, changes the ordering source,
   and requires both candidate tokens to stay equal while the actual fold
   changes. It rolls the injected metadata back and checks the old actual state
   is restored. This tests physical coverage, not repair or signing authority.
3. `a_commit_observer_does_not_fence_an_uncommitted_actual_state_change`
   compares a retained live commit observer with the writer's actual state after
   the repair seam changes but before commit; then requires publication after
   commit. It does not assert that two separately scheduled jobs were batched.
4. `unchanged_values_skip_history_but_a_named_attempt_keeps_its_idempotent_receipt`
   exercises the real public observation method on an invented declared
   resource/observer. An unchanged poll without an attempt must append nothing;
   a named attempt must add exactly one matching observer receipt without a new
   resource value; retrying the identical operation must add nothing further.
   It uses actual claim counts and independently inspects the attempt field.

Raw repair-cache and ordering writes here are isolated fixture mutations. They
are not a new public repair operation, a simulated authenticated import, or a
replacement for signed mixed-build/repair tests. Recipient/observer revision,
opener/import and dirty-state race controls for a future real adapter remain
owed. No passing result or complete race coverage is inferred from these bodies.

## Writer work accounting and disposition

Production code is unchanged except for a test-only module declaration. The
current writer still performs canonical normalization and history folds for the
resource, observer, supplied collection items and active subscriptions. The
no-change decision still follows those reads. A changed/attempt observation
still appends receipts/deliveries and runs the existing inline custom/resource
maintenance. No statement count, writer-hold reduction or CPU saving is claimed.

A future prepared implementation must prepare those canonical inputs in one
off-writer read cut, release that cut before queueing, and compare a complete
token under the writer. Claim/evidence identity, attempt receipts, idempotency,
notifications, recipients and authority must remain intact. The token must
make validation proportional to the actual dependency keys rather than their
retained histories. This unit does not have the missing adapter and therefore
does not wire an unsafe fast path or move validation behind an incomplete check.

Root and existing Intake/curator receive this private diff and all control
bodies before any compiler/publication decision. The existing #2930 telemetry
stack and #3082 productive cohort are untouched. The fleet latency incident,
Window 4 hold, Silber failed gate and Bluey disk hold remain unresolved. This
packet closes only the finite feasibility/discriminator unit, not the incident.
