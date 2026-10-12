# Protected CI observer

The internal `st3 evidence observe-ci --run RUN --attempt ATTEMPT --artifact ID`
command accepts numeric selectors only. It reads its operator-owned configuration
from the profile's private HOME at `.config/st/evidence-ci.json`. A sekrets grant
must allow only that installed command prefix, after review and binary pinning.
The caller cannot supply a statement, repository, check result, signer or key.

The private configuration names the repository, absolute private bare
`scratch_repo`, pinned `recipe_sha256` for `scripts/ci-evidence-source`, and
`required_checks`. It sets `tier` to `full`, `degraded` or `test`. Full mode also
names `signer` and the absolute private `ssh_key`; degraded mode has neither.
Test mode requires a throwaway key and `expires_at`, no more than 15 minutes
ahead. Use a separate scratch repository for the test service. Production key
files and configuration are never stored in a change checkout or this repository.

The protected PATH supplies operator-installed st3, Git, OpenSSH, gh, Python and
GNU timeout. gh receives only the profile's read-only GitHub credential. The
observer fetches attempt-specific run metadata and every page of jobs; it checks
the configured repository, Workspace CI workflow, completed state and required
check names. Missing, duplicate or incomplete jobs are refused. It checks the
source producer's actual steps, including upload, even when the optional job's
overall result is softened by `continue-on-error`.

Artifact identity, byte length and SHA-256 come from authenticated GitHub
metadata. The complete ZIP must match before extraction. Exactly two regular
files are allowed, with bounded sizes: receipt JSON and the Git object pack.
The receipt must bind the run/attempt and pinned recipe, and its pack digest
must match. After import into the private bare repo, Git independently derives
the source tree and the SHA-256 of the retained producer script. A PR's author
head from run metadata is never substituted for the tested merge tree.

Each required check gets one record, including failures. The command descriptor
identifies the GitHub workflow/check, while `runner` retains run, attempt and job
provenance. `output_sha256` hashes the serialized observed job JSON, rather than
claiming the observer ran cargo or captured its stdout. Skipped, neutral, cancelled
and failed jobs all have a failing exit code.

When signing fails, the same measured facts become explicitly unsigned degraded
records. A local per-job reference retains that result: retrying the same job
after the key returns does not promote it. A fresh keyed rerun is required.
The full record and observation reference are created in one Git transaction.
Test services emit test tier; production policy rejects those records even when
their signatures verify. An expired test service refuses to produce records.

The response contains record IDs, explicit tiers, and an object-only base64 pack
with source and evidence objects. The caller imports it with `git index-pack --stdin`, then uses `st evidence put` with its external allowed-signers file.
No key, configuration, ref or executable is exported. The caller retains/fetches
source parent history before pushing evidence refs to peers.

Batch metrics count newly recorded jobs only: full records / fresh records,
full-trust share (null with a zero denominator), and signing wall time. Original
CI receives zero signing wait because this adapter runs after check completion.
Re-attestation elapsed time remains null until a separately observed rerun is
explicitly linked; it is not reported as zero. The controller that schedules
observations and outage fallback is a separate integration.

The cooperative v1 receipt boundary trusts our runner agents. Pinning recipes
and isolating the key does not prove native process-to-agent identity against a
hostile workflow author. Native receipts remain a separate prerequisite.
