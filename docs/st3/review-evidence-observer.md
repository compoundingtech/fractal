# Protected review observer

The installed `st evidence observe-review --claim CLAIM_ID` accepts one full
64-character daemon claim ID. A protected profile fetches that claim itself,
its immutable PR resource snapshot and review document, and one authenticated
GitHub PR response containing both head and base. It derives the complete
changed-path/blob map from Git in its private bare repository. It never checks
out or executes the reviewed source. No CLI argument supplies a statement,
verdict, repository, endpoint, signer, key or configuration path.

The operator supplies `.config/st/evidence-review.json` in the profile's private
HOME. It names `repository`, `scratch_repo`, `daemon_socket`, `trusted_signers`,
`resource_prefixes`, `review_origins`, `snapshot_origins`, `reviewers`,
`test_signers`, `required_checks` and `tier`. Paths must be absolute. Full mode
also requires `signer` and `ssh_key`; test mode requires those and `expires_at`
with at most 15 minutes of remaining lifetime. Degraded mode forbids signer,
key and expiration. The installed PATH supplies Git, gh, ssh-keygen, curl and
GNU timeout. The profile requires read-only GitHub metadata/source access and
read access to the operator-chosen daemon Unix socket. Grant only this pinned
selector command, never a general shell, Git or SSH signing command.

The daemon's claim actor must be an allowed reviewer, its origin must be
allowed, and the structured reviewer field must equal that actor. The PR
snapshot must have the exact cited claim ID, resource subject and configured
origin. Resource prefixes can retain a repository's older name; the current
GitHub repository, PR number and node ID independently bind its identity.
The live head must equal the reviewed snapshot. A moved head needs a new
review observation. Multiple merge bases are refused. The base is captured
at evidence time and adds no field or base-only observations to general PR
snapshots. Ordering uses the daemon's claim acceptance time, not prose or a
reviewer's supplied timestamp.

`approve` means GO and requires the explicitly considered `evidence_tests`
record IDs. Every citation must have a trusted full signature in the configured
test role, name the same change, pass its checks, and include the reviewed head
in its Git ancestry with unchanged reviewed file blobs. This supports the
platform's tested synthetic merge. The operator's required checks must all be
cited. Stage verification still requires tests on its exact target tree.
`comment` is refused as a verdict. `request-changes` means STOP and can have
no test citations, including when CI has not run. A newer applicable full STOP
blocks an older GO. Upgrade all verifiers before enforcing: older v1 binaries
reject an empty-citation STOP. No enforcement is activated here.

The signature also covers the immutable claim, snapshot and document references
in the Git commit message. A document's bytes must match its SHA-256 reference;
its prose does not authorize a verdict. The producer digest measures the actual
installed binary. Native process-to-agent attestation remains separate work;
v1 trusts the configured daemon and cooperating agents.

Signing failure records the same observed facts as unsigned `degraded` evidence.
A claim's first result is retained under a private per-claim ref; replay returns
that result and never promotes degraded evidence. A new independently observed
review claim is required for a new review signature. Malformed, unavailable or
inapplicable observations are refused, so they cannot invent a verdict. Existing
intake documents and routing continue independently. This command is the adapter;
its automatic invocation, publication and production provisioning are separate.

The JSON response contains the result, explicit tier, immutable references and a
public object pack. A caller imports it, verifies with an operator allowed-signers
file, and publishes the additive evidence ref through its existing Git transport.
The export contains enumerated source/review objects, no key or private config.
Cited tests retain their own public refs. Fresh-record count, full-trust share and
signing latency exclude replays; a replay reports a null share with no denominator.
Test-tier keys never satisfy production policy. Tests use only throwaway keys.
