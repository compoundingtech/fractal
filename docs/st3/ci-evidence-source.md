# CI source receipts

Workspace CI retains a source receipt in the non-required `evidence-source` job.
It runs alongside existing checks, has only read permission, and holds no signing
key. Failure to produce or upload the receipt leaves the required checks running.
The actual failed step remains visible; no check result is inferred from a receipt.

For a pull request, GitHub's workflow and check-run `head_sha` identify the author
head. CI's checkout can instead be the synthetic merge `GITHUB_SHA`. The receipt
records `git rev-parse HEAD` and its tree and requires that HEAD equals the
effective `GITHUB_SHA`. Later evidence observers must use this source rather than
guess the tested tree from run metadata.

The artifact `ci-evidence-source-RUN_ATTEMPT` contains exactly:

- `receipt.json`: schema version, repository, event, run ID, attempt, complete
  source and tree IDs, object format, change placeholder, producer script SHA-256,
  and the pack's byte length and SHA-256.
- `source.pack`: the observed commit and its complete tree and blob objects.
  It carries no refs, repository configuration, executable observer or signing key.
  Commit history is excluded, so a depth-one checkout suffices. Gitlinks retain
  their commit IDs without copying submodule repositories.

The workflow embeds its producer in a temporary file outside the checkout. The
receipt measures those actual script bytes. The artifact is retained for 14 days,
including source objects whose synthetic merge ref has since moved or disappeared.

`change_id` is `github:OWNER/REPO#NUMBER` for a PR or merge queue entry, pending the
changes-in-git format. A queue entry can contain additional changes: its source
and tree still identify the whole tested checkout. A manual run uses
`github:OWNER/REPO@SOURCE`, without inventing a PR identity.

This receipt supplies source provenance for the record adapter; it is not a signed
evidence record and authorizes no gate. The protected observer must independently
authenticate artifact size/digest and repository/run/attempt identity, validate
the producer against its pinned recipe, and derive the tree from the imported Git
objects. It must observe completed check results separately and refuse mismatches.
Our cooperative v1 runner boundary does not prove process identity against a
hostile workflow author; native observer identity is a separate prerequisite.

Run `python3 scripts/ci-evidence-source-test` to exercise real Git merges, shallow
checkouts, object import, replacement-object resistance and provenance mismatches.
