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
