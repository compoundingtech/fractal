# Portable gate evidence v1

`st evidence put`, `show`, and `verify` run offline. They require Git with SSH
signing support, OpenSSH, repository objects, and an operator-supplied allowed
signers file. They do not load daemon configuration or contact an st instance.

One immutable record represents one observation; its trust tier is explicit. The first kinds are `tests` and
`review`. `produced-by` is reserved as a name for a future trusted observer's
verified trace receipt; v1 rejects it until its predicate and verifier exist.

## Record and transport

The record is an in-toto v1 statement with one subject: `name` is the observed
full commit ID, and `digest.gitTree` is its exact Git tree ID. The predicate type
is `https://smalltalk.dev/evidence/gate/v1`. Its fields are:

| Field | Meaning |
| --- | --- |
| `change_id` | Nonempty stable identifier; currently supplied by the workflow pending the change format. |
| `signer` | Exact SSH principal in the caller's allowed-signers file, or null for degraded records. |
| `tier` | `full`, `degraded`, or `test`; a signature alone never promotes a tier. |
| `re_attests` | Optional degraded record ID when a fresh full-trust test run re-attests the same keyed check. |
| `producer_sha256` | Lowercase SHA-256 of the signing observer's running executable, measured inside the signing API. |
| `observed_at` | RFC 3339 time recorded by the trusted observer. |
| `observed` | A typed `tests` or `review` observation. |

A `tests` observation names its `runner` and maps check names to `command`
(argument vector), `exit_code`, and `output_sha256` (exact captured output
bytes). The observer must construct these from its run, including failed checks.

A `review` observation names its `reviewer`, `verdict` (`GO` or `STOP`), and
`base_commit`, an ancestor of the observed commit. Its `files` map covers the complete changed-path set from base to
observed commit, with each path's Git blob ID or null for deletion. Renames
appear as deletion plus addition; submodules use their commit ID. `tests` lists
immutable signed test record IDs the reviewer considered. The observer must
obtain the verdict from its review execution and derive file hashes itself.

The statement is the sole regular-file blob in a notes tree, named by its
subject's full tree ID. For full/test records, `git commit-tree -S` signs that notes tree using Git's
SSH signing. Its only parent is the observed source commit, so transporting the
note also retains the source tree and review base history. The signature covers
the exact JSON bytes and attachment, without inventing a JSON canonicalization
or a detached signature scheme. This is in-toto statement JSON inside a signed
Git commit, **not** a DSSE envelope or a GitHub artifact attestation.

The immutable record ID is this notes commit's full object ID. Its only ref is
`refs/notes/evidence/<record-id>`. There is no shared mutable notes tip: different
writers create different refs, so their updates cannot conflict. Duplicate
imports verify the signature and create the ref transactionally, or confirm it
already points to the same ID. A ref whose suffix does not equal its target is
invalid. The flat notes tree remains readable with Git's notes command:

```sh
git notes --ref=refs/notes/evidence/RECORD_ID show TREE_ID
git push origin 'refs/notes/evidence/*:refs/notes/evidence/*'
git fetch origin 'refs/notes/evidence/*:refs/notes/evidence/*'
```

Keep the namespace when synchronizing repositories; do not prune records as a
side effect of branch cleanup. The wildcard refspec is additive and does not
need force updates or a merge of notes histories. v1 uses one ref per record;
large-repository indexing and retention belong to later stages.

## Trust and custody

The CLI never takes a JSON payload or a signing key. `put` imports an
already-constructed observation: full/test must be signed; degraded must be
explicitly unsigned with a null signer. Fetch its objects before importing:

```sh
st evidence put --repo . --record RECORD_ID --trusted-signers /tmp/allowed-signers
st evidence show --repo . --record RECORD_ID
st evidence verify --repo . --record RECORD_ID --trusted-signers /tmp/allowed-signers
```

`show` prints data and asserts no trust. `verify --record` checks the format,
attachment, source tree, signature, and exact signer principal for full/test.
For degraded it validates the format and confirms the absence of a signature.
CLI results report the tier and `signature_verified` explicitly. The trust file
uses OpenSSH allowed-signers syntax, for example a distinct public key for each
principal, restricted to `namespaces="git"`. Keep it outside the untrusted
checkout. A missing key, forged signature, mismatched signer, unsupported kind,
or malformed statement supplies no evidence. Repository Git signing programs,
allowed-signers paths, revocation files, and replacement objects cannot widen
this caller-supplied trust. Key removal from the trust list revokes its evidence.

`st3::evidence::sign_observation` is an API for trusted observers running under
a separate signing account. It validates the statement, measures its own
executable digest, constructs the note, and calls Git's SSH signer. The observer
must construct facts from its own execution; it must never expose this API as a
service that signs arbitrary agent-authored JSON. Test keys are generated in
temporary directories. No production key or key path is shipped here.

Production signer provisioning and narrow observation adapters are required
before intake or CI can write records. Preferred custody is a sekrets profile
whose only granted command is a trusted observer. Hosts without sekrets need an
explicitly documented cooperative interim. Custody and trace-receipt isolation
share an OS-account boundary; gate evidence does not claim native agent identity
or reimplement that separate receipt protocol.

## Gate policy and reuse

`.st/evidence-policy.json` in the exact target commit declares the initial
`daemon` risk class, required `linux-gate` check, allowed `full` tier, and CI/review principal roles.
These generic principals are provisioning targets; no key is trusted merely
because repository policy names it. The caller must separately supply its
operator-controlled trust list. Other risk classes require a later policy
version. The file affects this verifier only; this change does not activate
curator enforcement or alter the merge queue.

```sh
st evidence verify --repo . --revision HEAD --base BASE_COMMIT \
  --change-id CHANGE_ID --trusted-signers /tmp/allowed-signers
```

Verification selects the newest trusted review for this change ID whose entire
changed-path set and hashes match the current base/target diff. Observation
time orders reviews; a STOP wins equal-time ties, then record IDs order ties.
A newer applicable STOP or incomplete GO cannot be hidden by an older GO. The
selected GO must cite imported, trusted `tests` records for this exact target
tree and change ID, from permitted test signers. Every cited check must pass,
and their union must contain every required check. Its successful result reports `tier: full`. Otherwise the command exits
nonzero and blocks the stage. Test tier never satisfies production policy.
The initial daemon policy also rejects degraded records, even with passing
checks; there is no low-risk exception in this slice. Invalid or untrusted individual records cannot
satisfy a gate, and are ignored when selecting usable evidence; malformed or
rewritten namespace refs fail the transport integrity check.

Production is fail-soft: the later runner adapter must use `record_degraded`
when the protected signer is unavailable and continue CI. It preserves the
measured facts in a separate explicitly degraded record; production verification
remains fail-closed under the selected policy. Low-risk policy allowances and
re-attestation deadlines require the gate step and its policy decision. A fresh
full-trust rerun may cite the original with `re_attests`; the verifier checks its
subject, change ID, check names/commands, and time against that degraded record.
It never changes the original tier. Observing the same old bytes without running
the check again cannot qualify as a re-attestation.

The production adapters will measure full-trust record share, time from degraded
observation to fresh full re-attestation, and wall-clock signing latency added
to CI. Those numbers are not measured by this offline format slice. A future
short-lived throwaway-key test service always emits test tier, never full tier.
A second fixture signer proves opt-in trust, role restrictions, and revocation;
the gate step adds the actual spot-check mismatch drill.

A rebase that preserves the reviewed paths, their blobs, and the cited test
records keeps the GO. A new unreviewed path, changed blob, deletion, different
change ID, missing citation, failed check, wrong signer role, or changed tested
tree blocks reuse. v1 tests cover the entire exact tree: a rebase incorporating
unrelated source changes needs new tests and a GO citing those tests. Scoped
test input equivalence belongs to affected-set evidence and must not be inferred
from unchanged review blobs alone. Executable modes are covered by the exact
tested tree even though the review map records blob identities.

This format slice does not wire the later 14-day comment fallback, dual-writing
intake and CI adapters, exact-head review gates/typed waits, or the wait clock.
Those steps consume this format. The merge queue remains the landing path.
