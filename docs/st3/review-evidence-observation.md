# Structured review observations

Intake retains its immutable review document and curator routing. It records
judgment as a `resource.observed` claim of kind `human.review`, bound to the exact
PR resource snapshot it read. A GitHub comment is not a prerequisite for this
observation.

The existing `decision`, `reviewer`, `submitted_at`, `reason`, `document` and
`target` fields remain. Optional `evidence_tests` is an array of immutable Git
notes record IDs that intake explicitly considered. The protected signing
adapter validates each full object ID; a prose `cited:` line is not authority.
Older observations without this field remain valid resource facts, and do not
gain signed GO authority by omission.

Intake maps an unconditional GO to `approve`, HOLD/STOP to `request-changes`,
and informational or unresolved conditional decisions to `comment`. It uses
its actual seat subject as `reviewer`, records the submission time and reason,
and cites only records it actually reviewed. Until evidence exists, it keeps
the current document/message workflow. A pending or absent typed citation is
never silently filled from an unrelated successful CI link.

The GitHub repository observer now reads `baseRefOid` beside `headRefOid` in
the same query and retains it as `base_sha` in the PR resource snapshot. Its
existing `base_branch` remains. Future signing derives the review's ancestor
base and complete changed-path/blob map from those immutable Git identities,
instead of resolving today's moving main branch.

This change supplies inputs for the protected review adapter. It does not
activate signing, change curator behavior or enforce a gate. Deploy the schema
to participating daemons before intake publishes the new optional citation
field. Historical snapshots lacking `base_sha` must be observed again; the
signer must refuse to guess their base.
