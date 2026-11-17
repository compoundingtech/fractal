# Portable offline rename proofs

A rename changes a public agent or mission name, not its immutable storage identity.
A portable proof carries the signed evidence needed to check that relationship on
another machine without a daemon, a database, fleet discovery, or network access.
The verifier trusts **only the fleet anchor supplied by its caller**. Obtain that
public key through an authenticated channel; a key accompanying an untrusted
bundle is not a trust anchor.

Proof bundles are **fleet-internal** artifacts for graph tooling. They contain
complete co-batched claims and blobs of the required sealed envelopes.
**Do not publish bundles or attach them to public issues.** Export requires
explicit `--include-co-batched-claims` consent; HTTP callers must set
`include_co_batched_claims: true`. Missing consent is refused with an explanation.

## Plan and inspect a local rename

Agents and missions share the same operator contract:

```sh
fractal agents rename-id agent/example-old agent/example-new --as person/operator --reason "Clarify responsibility" --idempotency-key example-agent-rename --plan --json
fractal missions rename-id mission/example-old mission/example-new --as person/operator --reason "Clarify responsibility" --idempotency-key example-mission-rename --plan --json
fractal agents rename-proof agent/example-old agent/example-new --request-digest '<planned-request-digest>' --json
fractal missions rename-proof mission/example-old mission/example-new --request-digest '<planned-request-digest>' --json
```

`POST /v1/{agents,missions}/rename-id/plan` accepts the same `RenameRequest`
body as `POST /v1/{agents,missions}/rename-id`, including actor, reason,
idempotency key, and optional `expected_naming_revision` / `expected_declaration`
fences (`--if-naming-revision` / `--if-declaration`). It evaluates the request
without writing a claim, changing a name, or signaling reconciliation. The plan
echoes the exact request and its digest; `naming_revision` is the head **before**
the proposed rename. Keep the echoed request, especially its idempotency key, and
submit it unchanged to apply. Without an explicit key the CLI generates one and
includes it in the echoed request. A plan does not reserve a name or guarantee a
later apply against changed state.

`GET /v1/{agents,missions}/rename-proof?from=...&to=...&request_digest=...`
reads local evidence; `from` and `to` are required and `request_digest` is optional.
Query values must be URL-encoded. The CLI encodes them automatically. Both routes
use the native `st3.v1` value envelope, and `--json` prints its value.
Proof availability distinguishes selected or superseded evidence from unavailable
evidence; nullable metadata must not be interpreted as a successful proof.
The proof's previous revision and declaration describe the rename claim, while
`naming_revision` and `current_canonical_id` describe the current naming head.
The optional digest restricts evidence to the exact planned request.

### Factory source transition carrier

Factory authors the full typed transition
`{kind, request: RenameRequest, request_digest}` in source. `request` contains
`from`, `to`, `actor`, `reason`, `idempotency_key`, `expected_naming_revision`,
and `expected_declaration`; it is not stored separately from the carrier.
The first PLAN supplies the pre-rename pins. If those pins are added to the
request, run PLAN again: the digest covers the entire echoed request.
Preview POSTs exactly the carrier's `request`, requires `admissible: true`,
and compares the echoed request, pins, and digest. The execution slot submits
exactly that same request. No claim ID belongs in the source carrier.

#### Rename claim body and final admission

The native writer constructs these claim fields in
[`identities.rs:482–491`](../../crates/fractal-store/src/store/identities.rs#L482-L491):

```text
body.fields = {
  from: string, to: string, reason: string, owner_host: string,
  declaration: string, previous_revision?: string,
  request: {
    from: string, to: string, actor: string, reason: string,
    idempotency_key: string,
    expected_naming_revision: string | null,
    expected_declaration: string | null
  },
  preserved_incarnation?: string
}
body.evidence = []
```

`previous_revision` is omitted before the first rename, not serialized as null.
`preserved_incarnation` is omitted when unavailable (including mission renames).
Both request pins remain present as null when unpinned. The `from` field is the
pre-rename canonical name; `request.from` preserves the caller's spelling.
[`store.rs:24887–24897`](../../crates/fractal-store/src/store.rs#L24887-L24897)
wraps the fields and evidence and adds
`body._operation = {id: string, request_digest: string}`. `ClaimRecord` also
exposes that operation ID and digest as top-level metadata.

Final Factory admission must require:

- PROOF availability `selected` or `superseded`, and proof kind/from/to matching
  the carried kind/request transition.
- Both proof and embedded claim request digests equal the carried digest, and
  `claim.body.fields.request` deeply equal the full carried request.
- Claim kind `agent.renamed` or `mission.renamed` matching the carried kind;
  claim ID/subject matching proof claim ID/identity; and claim fields `from`,
  `to`, and `reason` matching the intended source transition/request.
- Claim declaration and previous revision matching the carried pins (an absent
  previous revision represents a null pre-first-rename pin), and matching the
  proof's declaration/previous revision. A final Factory request pins declaration.
- Claim `owner_host` matching claim origin and proof owner host; operation ID
  and digest matching the claim's `_operation` metadata; and evidence `[]`.

Compare the request's actor within the exact request, not by blindly equating
it with `claim.actor`: the native writer resolves an actor alias to immutable
identity. `preserved_incarnation` is descriptive, not a source rename pin.
For `superseded`, do not require current canonical ID to equal the edge target
or current naming revision to equal this historical claim ID.

`selected` means the requested edge is the current naming head; `superseded`
means it remains on the selected chain but a later rename advanced the head.
Both names must resolve to the same immutable identity in the same store snapshot.
`conflicted`, `pending-replication`, `not-found`, and `unavailable` are not proofs.
A digest mismatch returns `not-found` without a claim. `snapshot_index` is the
snapshot's highest local claim store index, not a fleet-wide freshness guarantee.
The PROOF declaration and previous revision come from the selected edge's claim;
the request digest and operation ID come from its `ClaimRecord`. An idempotent
replay preserves that claim and digest and returns `already-applied`.

PLAN metadata is nullable when its source cannot be resolved. Refusal is
`{code, detail}` and is omitted on success. PROOF metadata and `claim` are
nullable when no admissible edge evidence is available. Store failures remain
errors, not successful proof evidence. PLAN and PROOF are one-subject operator
reads; neither performs namespace renames.

Complete-membership owned-set publications compare immutable full subject keys:
after A→B, the membership key remains A. Both preview and apply normalize authored
B through the identity resolver before the omission diff, so listing B without A
does not retire or stop that identity. A mission's existing runs remain bound to
its immutable mission identity and revision.

This local read is distinct from the portable signed bundle commands below:
it neither exports a bundle nor independently verifies a fleet trust chain.
The existing `rename-id proof export` and `rename-id proof verify` commands
remain unchanged.

## Export the public fleet anchor

On a trusted fleet member with its daemon running, print the pinned public key:

```sh
fractal fleet anchor show
```

The default output is the public key alone, suitable for the verifier's
`--fleet-anchor` argument. `fractal --json fleet anchor show` returns JSON with
the single field `fleet_anchor`. This command exports no private key or invite code.
The anchor is the founding member's public trust key; other member keys are
authorized by the signed membership history rooted in it.

Distribute this key through an authenticated channel and pin it in the consumer's
source or trusted configuration, independently of any proof bundle. For example,
after authenticating the key, pass that pinned value to the offline verifier:

```sh
fractal agents rename-id proof verify agent-proof.json --fleet-anchor '<pinned-public-key>'
```

Do not populate the trusted argument from an untrusted bundle's `fleet_anchor`
field: that field is informational and cannot authenticate itself.

## Commands

```sh
fractal agents rename-id proof export agent/example/worker --include-co-batched-claims --out agent-proof.json
fractal missions rename-id proof export mission/example --include-co-batched-claims --out mission-proof.json
fractal agents rename-id proof verify agent-proof.json --fleet-anchor '<trusted-public-key>'
fractal missions rename-id proof verify mission-proof.json --fleet-anchor '<trusted-public-key>'
```

Export accepts a current public name, a historical alias, or the immutable storage
ID. Without `--out FILE`, the JSON bundle is written to standard output. Export
uses the source store and seals outstanding local replication batches before
constructing the proof. Verification reads only the file and explicit command
arguments; it does not open or initialize a local store.

### Revisions are signed claim-ID tokens

`naming_revision`, `--revision`, and `--after-revision` use the existing signed
rename claim ID, **not an integer, timestamp, or lexical ordering**. The `N` in
command summaries denotes a revision token.

```sh
fractal agents rename-id proof export agent/example/worker --include-co-batched-claims --revision '<rename-claim-id>' --out historical.json
fractal agents rename-id proof verify agent-proof.json --fleet-anchor '<trusted-public-key>' --after-revision '<previous-rename-claim-id>'
```

`--revision` selects an exact revision in the resolved identity's selected naming
chain. The historical bundle describes that revision's canonical name, even if
the source store knows later renames. A revision outside that chain is refused.

`--after-revision` succeeds only if that exact token occurs in the proven chain
**strictly before its head**. A token equal to the head is stale, and an absent
token is also stale. A two-rename proof can therefore demonstrate progress beyond
its first rename; the first-rename proof cannot demonstrate progress beyond itself.

## Bundle and verdict

The bundle schema is `fractal.rename-proof.v1`. Its descriptive fields are
`fleet_id`, `fleet_anchor`, `identity_kind`, `storage_id`, `canonical_id`, `naming_revision`,
`declaration`, and `chain`. The embedded `fleet_anchor` is informational and must
match the caller's independently trusted anchor; it cannot establish trust.
Both the bundle header and successful verdict carry `disclosure: "fleet-internal"`.
The verifier refuses missing or altered classification.
The bundle's `chain` lists signed rename claim IDs oldest first.
`declaration` identifies the declaration used by the selected head; each earlier
rename's own declaration is also checked. Descriptive fields are compared against
the authenticated claim content, not taken on trust.

`envelopes` preserves exact sealed CBOR payload bytes (base64 in JSON), writer
sequences and hashes, and the real member signatures. The payloads contain the
real claim signatures, membership admissions and windows, key grants and
revocations, declarations, and naming history. The exporter includes only required
evidence writers and their dependency-complete authenticated incarnation prefixes.
These full prefixes are necessary to detect interior omission of membership,
revocation, conflict, or name-reservation evidence, so bundle size grows with the
required writers' retained histories rather than only the number of renames.
Quarantined records are not proof evidence. Unrelated purely data-only writers
are excluded, but unrelated co-batched contents in required envelopes remain.

Verification checks envelope hashes and signatures, contiguous writer prefixes,
anchor-rooted membership and sequence windows, claim signatures and delegations,
declaration ownership, and the linked rename chain. Neither a self-asserted member
key nor a cryptographically valid signature outside its membership window grants
trust. A renamed actor's public alias is resolved through authenticated naming
evidence to the immutable actor named by the signed claim.

Success emits JSON with schema `fractal.rename-proof-verdict.v1` and `ok: true`.
The verdict reports `kind`, `storage_identity`, head `from`/`to`, signed `revision`,
`actor`, `reason`, `owner_host`, `signer_member`, `signer_key`, and `fleet_id`.
Its ordered `chain` contains steps with `revision`, `from`, `to`, and `claim_id`.
It also reports the declaration and authenticated head `request_actor`.
`actor` preserves the actor spelling in the signed request, including a proven
historical alias; it need not equal the immutable claim actor's storage spelling.
Success exits zero. Failures have stable codes and nonzero process statuses:

| Code | Exit | Meaning |
| --- | ---: | --- |
| `invalid-signature` | 2 | An envelope or required claim signature does not verify. |
| `broken-chain` | 3 | Writer or rename linkage is missing, inconsistent, or truncated internally. |
| `untrusted-membership` | 4 | Evidence is not authorized by the caller's anchor and membership windows. |
| `stale-revision` | 5 | The requested progress token is absent or is not strictly before the head. |
| `malformed-bundle` | 6 | The input JSON, schema, or required structure is invalid. |

Export fails closed when the retained source history cannot produce a valid
signed proof. Unsigned legacy history, a missing writer prefix (including a
nonzero writer floor), or checkpoint-pruned proof material is not replaced with
fabricated signatures or a local-cache assertion.

## Historical evidence, not a freshness oracle

A proof authenticates the history it contains; it cannot prove that no later
rename, removal, or revocation exists elsewhere. Removing an interior envelope
breaks the writer chain, but removing a complete signed suffix can yield an older
historical cut. Keep a previously verified revision and require
`--after-revision` when progress matters. Even that check is relative to the
caller's pinned token, not a claim of global latestness.
