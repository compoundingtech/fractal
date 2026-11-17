# Native mission effects

An effect identifies one immutable outward action across attempts, replacement runs and hosts. Its key is the pair `(mission subject, author key)`, not a run, attempt, incarnation or clock. The operation ID is `operation_id_for_key` of the namespaced encoded pair; durable receipts live on the dedicated `effect/<operation-id>` subject. The bound step-run is evidence, not identity.

## Provider capabilities

The daemon owns the capability table; SDK declarations cannot widen it. Unknown
providers are refused with `effect-provider-unknown` before admission. A declared
provider's class and target must match its built-in capability.

| Provider | Class | Target |
| --- | --- | --- |
| `native-message` | `native-coordinated` | Agent, person or channel subject |
| `person-ask` | `native-coordinated` | Person subject |
| `github.comment` | `native-coordinated` | JSON object with `repo` and positive `number` |
| `git.branch-create`, `git.pr-create` | `provider-idempotent` | JSON object with nonempty `repo` and `head` |

A capability declaration is not an adapter implementation. Only
`provider-idempotent` permits a provider adapter to re-issue an operation.
Native messages remain **native-coordinated**: keyed readback never grants a
second send. This is intentionally stricter than treating message idempotency as
permission to re-originate. A visible operation conflict parks the effect in the
admission transaction, before any outward action.


## Native messages

Every authenticated program-run send through `POST /v1/messages` is automatically fenced as a native effect. The caller uses the existing CLI `--idempotency-key` as the author key; the pair `(mission subject, supplied idempotency key)` identifies the effect across attempts and runs. The daemon derives the effect owner from the authenticated run binding, never from caller-supplied ownership. Program callers must not supply `owner`. The CLI requires `--from`, which must explicitly name their own `mission-run/RUN`; an agent, person or foreign run sender is refused. The revision must declare `write "message/*"`, subject to the run's effective authority.

Program sends require an explicit stable `--idempotency-key` for the same logical action. The ordinary CLI's hourly-derived default key is not a program recovery contract: under a run credential, omitting the explicit key is refused.

Admission requires the program credential's own current live step-run attempt. A stale credential or a credential for another run grants no authority. Program sends do not borrow a worker lease: their authenticated binding supplies the mission, step, attempt and incarnation evidence.

Worker callers may still opt in with an `owner` object containing `mission`, `key`, `step`, `attempt` and `incarnation`. The authenticated caller must be the worker holding that exact live step-run lease. Its incarnation is also verified through the existing helper-launcher process provenance check: a claimed incarnation alone is not authority. First origination additionally requires the verified caller to equal the message's `from`. Unattested callers and stale leases are refused before admission.

The daemon derives a stable provider message idempotency key from the effect identity. For worker-owned effects, the caller's transport idempotency key does not change that identity; for program sends, the supplied key is the author key itself. The immutable digest covers the exact sender, recipient, content, title, reply reference, tags and attachments. Reusing an effect key with different input returns `idempotency-mismatch`, records `effect.unresolved`, sets the bound step's `effect-unresolved` condition, and sends nothing. Use a new author key for a different action.

For callers without an authenticated program-run binding, an absent owner preserves ordinary message sends, including their existing idempotency and signed wire representation. The optional owner field is omitted, never serialized as null. Omitting owner never opts a program-run send out of fencing.

## Receipts and recovery

The system-only, durable append receipts are:

- `effect.intended`: immutable provider, target, payload digest, class and work binding admitted under the run fence.
- `effect.in-flight`: persisted before native message origination.
- `effect.confirmed`: the committed message reference and its claim evidence were observed.
- `effect.unresolved`: recovery cannot establish the result, or the operation conflicts. The step visibly carries `effect-unresolved`.

Native messages use native coordination. First origination of an absent effect requires the step-run's original creation anchor to originate on the local host. The anchor is the enclosing `mission-run.created` receipt: initial step rows are materialized from that receipt, not from a separate step creation claim. Program callers must hold their own current live attempt; worker callers additionally require the current `work.claimed` receipt to originate locally. Missing or ambiguous anchors park the effect. A peer's locally claimed replacement attempt does not acquire origination authority, even if that peer has not received the original intent. Other hosts may only recover by key or park.

The anchor is retained for the run's lifetime: `mission-run.created` is durable (`crates/fractal-schema/retention.toml:137`), and claims without a checkpoint rule are retained (`retention.toml:297`; `crates/fractal-store/src/checkpoint_rules.rs:265–268`). Its system-only once semantics additionally prevent checkpoint dropping (`checkpoint_rules.rs:853–859`).

**Cross-run uniqueness is NOT guaranteed under partition.** The key remains mission-global, but the origination fence is limited to the same step's original run. Claim replication does not provide a contiguous per-origin visibility fence: an earlier held envelope can be rolled back while later envelopes are admitted (`crates/smallclaims/src/store.rs:8523–8612`). Consequently, seeing an earlier run's terminal claim does not prove that its effect receipts are locally visible. This contract does not guarantee partition-safe cross-run absence or add a broader coordination protocol.

The daemon persists `effect.intended` before invoking the provider, then persists `effect.in-flight` before native message origination. Any repeat encountering intended or in-flight state is readback-only: it reads the existing message by key and confirms the observed committed message or parks; it never invokes the provider again. Interruption after intent but before the provider therefore parks when no committed message can be established, even if the provider was never called. An uncertain outcome returns the typed `effect-unresolved` error and never resends. An empty local lookup, missing response cache, stopped worker or program, or unavailable originating store is not proof of non-delivery. There is no timeout-based takeover or automatic retry of origination. Unresolved state requires explicit human or revision action, not elapsed time.

The durable unresolved receipt fences reconciliation and late work-state projections, not just the current visible status. Lease expiry, stopped incarnations and submitted-step gates cannot ready or complete the bound step while that fence exists. The enclosing run remains blocked before dependency-based mission completion, final-phase selection and final success or cleanup.

A different verified worker may recover an effect only while holding the current live attempt of the **same bound step-run**, under the same effect key. It replays the byte-identical original payload, including the original worker in `from`; changing that sender is a payload mismatch, not a replacement send. This authority permits only keyed readback and confirmation (including returning an already confirmed result), never a provider send under the original worker's identity. Without delivery evidence it records unresolved and parks. Confirmation and unresolved receipts separately record `recovering_actor`, retaining the original sender and lease binding as immutable intent evidence.

A program caller may likewise recover only under its own current live attempt of the same bound step-run, preserving the original `mission-run/RUN` sender and immutable payload. Program recovery is keyed readback and confirmation only; neither a replacement attempt nor a new host grants permission to resend. Another run cannot recover under the original run's identity.

For a long message, recovery checks the supplied full text's hash and byte length before reusing the committed body attachment metadata, including its original host. Recovering on another host does not change immutable message identity.

Confirmation proves the native message committed, not that a recipient read it. Delivery and read observations remain the message lifecycle's evidence. Unobserved optional result metadata remains absent, not fabricated.

### Deterministic ambiguous-window proof (isolated test daemons only)

The `test-support` feature includes a one-shot checkpoint; production builds do
not contain it. Never arm it on a shared daemon. Use a separately admitted
`fractal-unstamped` test artifact (`cargo build -p fractal --features test-support
--bin fractal-unstamped`) in a fresh isolated state directory with no fleet peers.
Do not replace or restart the production daemon for this proof.

Before starting the approved isolated program run, choose its exact mission ID
(without `mission/`) and immutable notification author key. Include the immutable
run and notification step in the key, never the attempt. Compute the effect hash
and arm the boundary in that daemon's state directory:

```sh
: "${ISOLATED_STATE:?fresh isolated daemon state directory}"
: "${MISSION:?exact mission ID without mission/}"
: "${AUTHOR_KEY:?immutable run and notification step key}"
EFFECT_HASH=$(python3 -c 'import hashlib,json,sys; k="native-effect/"+json.dumps([sys.argv[1],sys.argv[2]],separators=(",",":"),ensure_ascii=False); print(hashlib.sha256(b"st3.idempotency.v1\0"+k.encode()).hexdigest())' "$MISSION" "$AUTHOR_KEY")
mkdir -p "$ISOLATED_STATE/test-native-message"
touch "$ISOLATED_STATE/test-native-message/$EFFECT_HASH.after-accept.arm"
```

Use only the isolated daemon's socket/config for publication, start and reads.
At provider acceptance, before `effect.confirmed` or the command's program journal
acknowledgment, the checkpoint renames `.after-accept.arm` to
`.after-accept.reached` and terminates only that store's registered program host.
The daemon itself stays alive; normal host supervision starts a replacement and
replays the same attempt. No timeout or retry grants send authority.

Observe the `.reached` file, unchanged daemon PID, a replaced program-host PID,
the same run/step/attempt and replayed earlier `program.step` values. Inspect
`fractal effects status --key "native-effect/[...]"` using the exact compact JSON
pair above: the original intended/in-flight receipts must be followed by
confirmation of the already committed message. The resumed notification key must
have a completed `program.step` receipt, and the provider's message count for the
derived key must remain **one**. Its provider key is
`native-effect-message/op/$EFFECT_HASH`, not the CLI author key.

For the pre-provider cut, arm `$EFFECT_HASH.after-intent.arm` instead. It is
consumed to `.after-intent.reached` after durable intent and before any provider
call. Replay must park with `effect-unresolved` and a zero message count; it does
not originate. Remove unused arm files when the isolated proof ends. These
checkpoints are test fault injection, never a production recovery option.


## Unresolved effects and explicit resolution

An unresolved effect waits like a broken gate: reconciliation does not acquire
origination authority through lease expiry, restart or elapsed time. The owning
agent receives one `effect-unresolved:<effect-subject>` fault episode and the
person sees one `effect-unresolved` attention item. Population attention counts
publish that item only on the enclosing run's canonical creation-origin host,
not on every replica that reads it. The run view's `effects` block identifies each
effect, its state and any observed `provider_ref`.

Read an effect by its full stable key:

```sh
fractal effects status --key 'native-effect/["example/report","report-ready"]'
```

The same read is available at `GET /v1/effects/by-key?key=...`. A missing
effect is not permission to originate on another host.

Only an authenticated person may explicitly resolve an uncertain outcome.
An actor string alone, an agent identity or a non-person principal is not
person authority. Inspect the provider and evidence before choosing:

```sh
fractal effects resolve --subject effect/OPERATION_ID --as person/example \
  --confirmed --provider-ref message/EXAMPLE --evidence CLAIM_ID \
  --idempotency-key example-confirmation
fractal effects resolve --subject effect/OPERATION_ID --as person/example \
  --abandoned --evidence CLAIM_ID --idempotency-key example-abandonment
```

`POST /v1/effects/{subject}/resolve` accepts the same actor, outcome
(`confirmed` or `abandoned`), evidence and idempotency key; confirmation also
requires a provider reference. Resolution appends the existing
`effect.confirmed` receipt with `resolution: "human"` and `recovering_actor`.
Abandonment additionally records `outcome: "abandoned"`; it does not invent a
provider reference or delivery evidence. Ordinary receipts omit these optional
fields. Neither resolution option re-sends the effect.

The explicit resolution clears the covered unresolved fence and permits the
bound step and run to continue. Abandonment explicitly waives an effect's product
hold without turning it into delivery: `delivered` remains false and no
`provider_ref` is invented. An explicit delivery predicate still evaluates false.
Confirmed effects expose `delivered` and the observed `provider_ref` through the
existing product/field-gate machinery, rather than a second hold mechanism.
See [gate evidence](gate-evidence.md) and the
[program engine](program-engine.md) for the surrounding completion contract.
