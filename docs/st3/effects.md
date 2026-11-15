# Native mission effects

An effect identifies one immutable outward action across attempts, replacement runs and hosts. Its key is the pair `(mission subject, author key)`, not a run, attempt, incarnation or clock. The operation ID is `operation_id_for_key` of the namespaced encoded pair; durable receipts live on the dedicated `effect/<operation-id>` subject. The bound step-run is evidence, not identity.

## Native messages

`POST /v1/messages` opts in with an `owner` object containing `mission`, `key`, `step`, `attempt` and `incarnation`. The authenticated caller must be the worker holding that exact live step-run lease. Its incarnation is also verified through the existing helper-launcher process provenance check: a claimed incarnation alone is not authority. First origination additionally requires the verified caller to equal the message's `from`. Unattested callers and stale leases are refused before admission. Program-run authority is not admitted by this endpoint.

The daemon derives a stable message idempotency key from the owner identity; the caller's transport idempotency key does not change effect identity. The immutable digest covers the exact sender, recipient, content, title, reply reference, tags and attachments. Reusing an effect key with different input returns `idempotency-mismatch`, records `effect.unresolved`, sets the bound step's `effect-unresolved` condition, and sends nothing. Use a new author key for a different action.

An absent owner preserves ordinary message sends, including their existing idempotency and signed wire representation. The optional owner field is omitted, never serialized as null.

## Receipts and recovery

The system-only, durable append receipts are:

- `effect.intended`: immutable provider, target, payload digest, class and work binding admitted under the run fence.
- `effect.in-flight`: persisted before native message origination.
- `effect.confirmed`: the committed message reference and its claim evidence were observed.
- `effect.unresolved`: recovery cannot establish the result, or the operation conflicts. The step visibly carries `effect-unresolved`.

Native messages use native coordination. First origination of an absent effect requires both the step-run creation anchor and the current `work.claimed` receipt to originate on the local host. The anchor is the enclosing `mission-run.created` receipt: initial step rows are materialized from that receipt, not from a separate step creation claim. Missing or ambiguous anchors park the effect. A peer's locally claimed replacement attempt does not acquire origination authority, even if that peer has not received the original intent. Other hosts may only recover by key or park.

The anchor is retained for the run's lifetime: `mission-run.created` is durable (`crates/fractal-schema/retention.toml:137`), and claims without a checkpoint rule are retained (`retention.toml:297`; `crates/fractal-store/src/checkpoint_rules.rs:265–268`). Its system-only once semantics additionally prevent checkpoint dropping (`checkpoint_rules.rs:853–859`).

**Cross-run uniqueness is NOT guaranteed under partition.** The key remains mission-global, but the origination fence is limited to the same step's original run. Claim replication does not provide a contiguous per-origin visibility fence: an earlier held envelope can be rolled back while later envelopes are admitted (`crates/smallclaims/src/store.rs:8523–8612`). Consequently, seeing an earlier run's terminal claim does not prove that its effect receipts are locally visible. This slice does not claim partition-safe cross-run absence or add a broader coordination protocol.

A replacement attempt encountering intended or in-flight state reads the existing message by key. It confirms the observed committed message or remains blocked; it never sends again. In particular, interruption after intended but before a message commit may leave an unresolved effect. An empty local lookup, missing response cache, stopped worker or unavailable originating store is not proof of non-delivery. There is no timeout-based takeover or automatic retry of origination. Unresolved state requires explicit human or revision action, not elapsed time.

The durable unresolved receipt fences reconciliation and late work-state projections, not just the current visible status. Lease expiry, stopped incarnations and submitted-step gates cannot ready or complete the bound step while that fence exists. The enclosing run remains blocked before dependency-based mission completion, final-phase selection and final success or cleanup.

A different verified worker may recover an effect only while holding the current live attempt of the **same bound step-run**, under the same effect key. It replays the byte-identical original payload, including the original worker in `from`; changing that sender is a payload mismatch, not a replacement send. This authority permits only keyed readback and confirmation (including returning an already confirmed result), never a provider send under the original worker's identity. Without delivery evidence it records unresolved and parks. Confirmation and unresolved receipts separately record `recovering_actor`, retaining the original sender and lease binding as immutable intent evidence.

For a long message, recovery checks the supplied full text's hash and byte length before reusing the committed body attachment metadata, including its original host. Recovering on another host does not change immutable message identity.

Confirmation proves the native message committed, not that a recipient read it. Delivery and read observations remain the message lifecycle's evidence. Unobserved optional result metadata remains absent, not fabricated. Provider adapters, effect products/gates and program/CLI authoring helpers are separate from this native-message contract.
