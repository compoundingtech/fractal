# Native mission effects

An effect identifies one immutable outward action across attempts, replacement runs and hosts. Its key is the pair `(mission subject, author key)`, not a run, attempt, incarnation or clock. The operation ID is `operation_id_for_key` of the namespaced encoded pair; durable receipts live on the dedicated `effect/<operation-id>` subject. The bound step-run is evidence, not identity.

## Native messages

`POST /v1/messages` opts in with an `owner` object containing `mission`, `key`, `step`, `attempt` and `incarnation`. The authenticated sender must be the worker holding that exact live step-run lease. Its incarnation is also verified through the existing helper-launcher process provenance check: a claimed incarnation alone is not authority. Unattested callers and stale leases are refused before admission. Program-run authority is not admitted by this endpoint.

The daemon derives a stable message idempotency key from the owner identity; the caller's transport idempotency key does not change effect identity. The immutable digest covers the exact sender, recipient, content, title, reply reference, tags and attachments. Reusing an effect key with different input returns `idempotency-mismatch`, records `effect.unresolved`, sets the bound step's `effect-unresolved` condition, and sends nothing. Use a new author key for a different action.

An absent owner preserves ordinary message sends, including their existing idempotency and signed wire representation. The optional owner field is omitted, never serialized as null.

## Receipts and recovery

The system-only, durable append receipts are:

- `effect.intended`: immutable provider, target, payload digest, class and work binding admitted under the run fence.
- `effect.in-flight`: persisted before native message origination.
- `effect.confirmed`: the committed message reference and its claim evidence were observed.
- `effect.unresolved`: recovery cannot establish the result, or the operation conflicts. The step visibly carries `effect-unresolved`.

Native messages use native coordination. Only the host that originated the current `work.claimed` claim may originate a new effect. Other hosts may recover the message by its stable key or park; they cannot take over origination. This is deliberately stricter than ordinary store-local message idempotency and prevents disconnected first-send races from becoming two sends.

A replacement attempt encountering intended or in-flight state reads the existing message by key. It confirms the observed committed message or remains blocked; it never sends again. In particular, interruption after intended but before a message commit may leave an unresolved effect. An empty local lookup, missing response cache, stopped worker or unavailable originating store is not proof of non-delivery. There is no timeout-based takeover or automatic retry of origination. Unresolved state requires explicit human or revision action, not elapsed time.

Confirmation proves the native message committed, not that a recipient read it. Delivery and read observations remain the message lifecycle's evidence. Unobserved optional result metadata remains absent, not fabricated. Provider adapters, effect products/gates and program/CLI authoring helpers are separate from this native-message contract.
