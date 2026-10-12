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

The protected review observer reads the PR head and base together when recording
evidence. It must require that head to equal the immutable reviewed snapshot,
derive the ancestor base and complete changed-path/blob map from Git, and retain
the base in the evidence record. General PR snapshots do not record the moving
base SHA, so an unrelated main advance adds no snapshot claims for dormant PRs.

This change supplies inputs for the protected review adapter. It does not
activate signing, change curator behavior or enforce a gate. Deploy the schema
to participating daemons before intake publishes the new optional citation
field. If the live PR head differs from the reviewed snapshot, the signer refuses
to guess applicability and a new review observation is required.
