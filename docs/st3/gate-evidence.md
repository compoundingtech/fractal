# Gate evidence

`st evidence put`, `show`, and `verify` run offline. They require Git with SSH
signing support, OpenSSH, repository objects, and an operator-supplied allowed
signers file. They do not load daemon configuration or contact an st instance.

One immutable record represents one observation; its trust tier is explicit. The first kinds are `tests` and
`review`. `produced-by` is reserved as a name for a future trusted observer's
verified trace receipt; v1 rejects it until its predicate and verifier exist.

## Record and transport

The record is an in-toto v1 statement with one subject: `name` is the observed
full commit ID, and `digest.gitTree` is its exact Git tree ID. Rust and Python
verifiers accept both predicate types:
`https://compounding.tech/fractal/evidence/gate/v1` and
`https://smalltalk.dev/evidence/gate/v1`. Acceptance of the old URI is the
permanent signed-history decoder exception named in
[#3032](https://github.com/compoundingtech/smalltalk/issues/3032); existing signed
attestations are never rewritten. All other predicate URIs are rejected.
The Rust observers and `scripts/evidence-sign` write
`https://compounding.tech/fractal/evidence/gate/v1`. The dual-URI verifiers
also accept records produced by older signers. Its fields are:

| Field | Meaning |
| --- | --- |
| `schema_version` | Predicate schema version, initially `1`; unknown versions fail closed. |
| `change_id` | Nonempty free label in the signed predicate; supplied by the workflow or the wrapper. Nothing matches or filters on it. |
| `signer` | Exact SSH principal in the caller's allowed-signers file, or null for degraded records. |
| `tier` | `full`, `degraded`, or `test`; a signature alone never promotes a tier. |
| `re_attests` | Optional degraded record ID when a fresh full-trust test run re-attests the same keyed check. |
| `producer_sha256` | Lowercase SHA-256 of the signing observer's running executable, measured inside the signing API. |
| `observed_at` | RFC 3339 time recorded by the trusted observer. |
| `observed` | A typed `tests` or `review` observation. |

A `tests` observation names its `runner` and maps check names to `command`
(argument vector), `exit_code`, and `output_sha256` (exact captured output
bytes). Its open `inputs` map carries opaque target/input keys and digest strings
for future affected-set observers, and defaults to empty. v1 preserves those keys
but still requires the whole exact tree; it does not infer scoped equivalence. The observer must construct these from its run, including failed checks.

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
`refs/notes/evidence/<tree>/<record-id>`, where `<tree>` is the git tree the note is about (the
name of the one entry in the note's flat tree). There is no shared mutable notes tip: different
writers create different refs, so their updates cannot conflict. Duplicate
imports verify the signature and create the ref transactionally, or confirm it
already points to the same ID. A ref whose last component does not equal its target, or that
sits under a tree the note is not about, is invalid and finds nothing. The flat notes tree
remains readable with Git's notes command:

```sh
git notes --ref=refs/notes/evidence/TREE_ID/RECORD_ID show TREE_ID
git push origin 'refs/notes/evidence/TREE_ID/RECORD_ID:refs/notes/evidence/TREE_ID/RECORD_ID'
git fetch origin 'refs/notes/evidence/TREE_ID/*:refs/notes/evidence/TREE_ID/*'
```

**One tree is one prefix.** The verifier, the gate, the signed-tree job and the publisher reach the
records of a tree through `refs/notes/evidence/<tree>/` (and the records of an input key through
`refs/notes/evidence-input/<check>/<key>/`), never by listing or fetching every note: a lookup costs
what that tree's own records cost, however many trees have records. A test pins this (the verifier's
`for-each-ref` listings during one verification return at most the tree's own records). The cost of the
whole namespace is paid only by `scripts/evidence-gc`, which also keeps its size bounded (see below).

**Tags, written and read first.** Every reader also reads `refs/tags/evidence/<tree>/<record>` and
`refs/tags/evidence/input/<check>/<key>/<record>` (lightweight tags naming the same signed commit), in the same
listing or fetch as the notes, one prefix per tree or key in each namespace; a record under both names counts
once. Records move there because a ruleset can protect tags and not notes (`doc/fleet/factory/skip-ci-plan`, the
hard precondition: the readers were on main before any producer wrote one). The producers
(`scripts/evidence-publish`, `scripts/ci-evidence-observe`, `scripts/evidence-wrap` and so `scripts/st-test --push`)
file every record under both names, the tag and the note, created together or not at all and pushed in one push:
a gate whose base predates the tag readers reads only notes, and a record only it cannot see would hide a newer
failure from it. Writing notes stops in a later change, once no gate that still runs reads notes only. A tree id
is hex, so `input/` never collides with a tree's prefix; both live under `evidence/`, which the tag release
workflow ignores, full-history checkouts leave out and no workflow's push trigger matches (a lint pins it).

**The first layout** was `refs/notes/evidence/<record-id>` (one object id under the prefix). The
verifier still reads it, with a pattern that matches only names exactly one object id long, so it never
lists a tree-prefixed ref, and `stale_tree` ("records exist, for other trees") is reported only for those
refs: a tree-prefixed lookup cannot know about other trees' records without a scan, so it says `no_record`.
Publishers write the tree-prefixed layouts only, the tag and the note (`scripts/evidence-publish`,
`scripts/ci-evidence-observe`, `scripts/evidence-wrap`); `st evidence put` (the Rust module, `crates/fractal-kernel/src/evidence`) still writes
the first layout for its own records and ignores tree-prefixed refs when it lists. The garbage collector
removes or migrates the first layout; the gate and the signed-tree job fetch only the tree-prefixed refs.

Do not prune records as a side effect of branch cleanup, and keep the namespace when synchronizing
repositories. The wildcard refspec is additive and does not need force updates or a merge of notes
histories. v1 uses one ref per record.

### Garbage collection and the size of the namespace

Every gate member of every pull request and merge-group run adds one ref, so the namespace grows with CI
(measured 2026-10-19: 9,430 records on 1,195 trees, about 8 records per tree, 258 input-index refs, 2.2k to
3.5k new refs a day; the whole remote lists 13,797 refs, 1.3 MB, in about 0.5 s). `scripts/evidence-gc`
keeps it bounded. It is the one tool that reads the whole namespace; nothing on a hot path does.

```sh
scripts/evidence-gc --fetch --remote origin                       # dry run: prints the plan's counts per reason
scripts/evidence-gc --fetch --remote origin --plan-out plan.json  # the same, and the plan with its inputs, to recompute it
scripts/evidence-gc --fetch --remote origin --apply               # deletes exactly the plan's refs, locally and on the remote
```

**A pure plan.** `gc_plan(records, index, live trees, live keys, retention, as_of)` is a function of git data alone,
so anyone with the same refs gets the same plan; the output names its `inputs_sha256` and `plan_sha256`, and
`--plan-out` writes the inputs beside the plan. Applying it deletes exactly the refs it names. Its inputs:

- **live trees**: the tree of every branch tip committed within `branch_days` of `as_of` (`--fetch` reads every
  `refs/heads/*` into `refs/evidence-gc/tips/*`; open pull request heads and merge-queue tips are branches, so no pull
  request API is asked), plus `--head`/`--heads-from`; and the trees of main's first-parent commits within `main_days`
  of `as_of`, main's tip always.
- **live keys**: the input key each live tree projects to, for every check whose policy declares inputs.
- **retention**: `.st/evidence-retention.json` at `--main` (`--retention FILE` to try another), reviewed like any
  change: `main_days` (3), `branch_days` (30), `grace_hours` (24), `soft_records` (3,000), `cap_records` (5,000).
- **as_of**: main's tip commit time unless `--as-of` is given, so a quiet main ages nothing.

**Whole trees.** A tree's records are deleted together or not at all, and a live tree's never; an input key's index
refs likewise. The verifier decides a tree from that tree's records and its input key's records, so no verdict of a
live tree can change. A dead tree that is pushed again is a new tree: its verdict is its old one, no record (the check
runs), or what its input key alone says, as for any new tree with the same inputs. Deleting records one at a time by
age, as the first version did, could remove a failure and keep the pass dated within the two-minute margin after it,
which the failure still shadowed: that tree then verified. `scripts/evidence-gc-test` checks these properties on 400
random worlds against a model of the verifier, and end to end with the verifier on real records.

**Bounds.** A dead tree or key whose newest record is younger than `grace_hours` is kept (a record written moments
ago for a head the listing raced with) unless more than `soft_records` records remain: then dead groups go oldest
first by their newest record. Records are counted, not refs, so a record filed as a tag and a note counts once. If the
live set alone is over `cap_records`, nothing live is deleted, the set is printed and the exit status is 3.

**Measured 2026-10-21** (a dry run against the remote): 5,021 refs, 4,562 records; 960 branch tips within 30 days
(their trees hold 434 refs) and 217 main trees within 3 days (2,593 refs); 155 dead trees and 22 dead keys are deleted
(629 refs), and 1,077 refs more over the soft limit, leaving 3,000 records, 2,970 of them live, and 3,315 refs. The
first run in an empty repository took 463 s (most of it fetching every record and 40 days of main's trees); a second
run in the same repository took 68 s, 56 s of it projecting 2,350 input keys.

**The lock window.** A tag ruleset on `refs/tags/evidence/**` (no deletion, no update) refuses GC's own deletions too,
so `--lock-config FILE` names three commands, run without a shell: `unlock`, `relock` and `check` (exit 0 only while the
protection is on). `scripts/evidence-gc-lock.example.json` has them for the ruleset through `gh api`; moving the
credential to a sekrets profile is a change of that file alone. The order: migrations and note deletions first
(nothing protects notes), then `check` (found off: relock, delete no tag, exit 4), a listing of the remote's evidence
tags, `unlock`, the tag deletions, `relock` in a `finally` that also runs on SIGTERM, SIGINT and SIGHUP (retried three
times), `check` again, and a second listing. A tag that disappeared or moved without being in the plan, or a protection
that is not on at the end, is an alarm (exit 4). One line on stderr records it:
`evidence-gc-lock v1 unlocked_at=... relocked_at=... window_s=... tags_deleted=N unexplained=N final=on`.
`--relock-only` runs `relock` and `check` alone.

**Where it runs.** `scripts/evidence-gc.timer` (daily, 03:40 UTC) starts `scripts/evidence-gc.service`, a systemd
user unit on a host we hold, never a GitHub workflow. Both are installed in `~/.config/systemd/user/`, and
`scripts/evidence-gc-timer` in `~/.local/bin/`. The wrapper keeps a bare repository under
`~/.local/state/evidence-gc/`, reads `scripts/evidence-gc` and `scripts/evidence-verify` from `origin/main` (a branch
cannot change what deletes), runs it with `--fetch --apply` and the lock config in `~/.config/evidence-gc/lock.json`
when present, keeps 30 days of summaries and plans, and stores each summary as a new version of the st document
`doc/fleet/factory/evidence-gc/runs`. The unit's `ExecStopPost` runs `--relock-only` after every stop, so even a run
killed inside its window leaves the protection on. On a host that also runs the evidence log (`~/.local/bin/evidence-log-timer`
and `~/.config/evidence-log/signer.json` present), the wrapper passes `--before-delete`, so `evidence-log gc --publish`
writes GC's lines and publishes the log before anything is deleted; if that fails, nothing is deleted.

**Before deleting.** `--before-delete JSON_ARGV` (with `--plan-out`) runs a command, without a shell and with the plan
file's path as its last argument, after the plan is written and before any deletion or migration. If it exits non-zero
or cannot run, `--apply` changes nothing (`refused`, exit 1) and the lock window is never opened.

**Safety.** It fails closed: if a commit named by `--head` cannot be turned into a tree, no branch tip is known at
all, or the `--before-delete` command fails, `--apply` deletes nothing. A failed remote push leaves the local refs alone. A second run finds nothing to do.
Refs outside the evidence namespaces and refs that are not well-formed records are never touched. The first layout
(`refs/notes/evidence/<record>`) is read from the note: in a live tree it is migrated to
`refs/notes/evidence/<tree>/<record>` (the new name is created before the old one is removed), in a dead tree it goes
with the tree. Take a bundle first when running it by hand:
`git bundle create evidence-notes.bundle 'refs/tags/evidence/*' 'refs/notes/evidence/*' 'refs/notes/evidence-input/*'`.

### Records as tags (in progress)

No ruleset can protect `refs/notes/*` (GitHub refuses the target pattern), so anyone with write access can delete a
newer failing record and let an older pass decide again, and git keeps no trace of the deletion. The records are
therefore moving to `refs/tags/evidence/<tree>/<record>` under a tag ruleset (no deletion, no update) that GC turns off
only around its own deletions; the ruleset is an extra, and the protection that does not depend on GitHub is a signed log
of every record kept on a host we hold (`doc/fleet/factory/skip-ci-plan`, "Portable evidence"). Until then the exposure is bounded: nothing rejects, a pull request skip is
undone by the merge queue's full run, and one run in five is sampled. The order, each step its own pull request:

1. The tag release workflow ignores `evidence/**` tags (done).
2. No workflow checkout fetches them (done). `fetch-depth: 0` fetches every tag, and 5,000 records as tags added 81.5 MiB and
   16 s to a full fetch (measured 2026-10-19), mostly source objects of tested commits that no branch holds. Every
   full-history checkout is now `fullHistoryCheckoutSteps` in `.github/workflows/workspace-ci.ts`: a one-commit checkout,
   then one fetch of every branch and tag with the negative refspec `^refs/tags/evidence/*`. Deepening the shallow clone
   took 21 s against 11 s for a full fetch, so the step drops the shallow marker, fetches as a fresh clone would
   (`--refetch`, git 2.36 or later) and checks connectivity: 15 s in all, against 11 s for `fetch-depth: 0` today
   (2026-10-20, from hetz2). `scripts/ci-evidence-signing-test` allows only literal positive depths and runs that fetch
   against a real repository.
   A plain `git fetch` does not follow these tags either: a record's tag points at a commit no fetched branch holds.
   A fresh `git clone` does fetch them.
3. The verifier and the gate read tags first and notes as the legacy layout (done); then the producers write each
   record as a tag and a note (done, see "Tags, written and read first" above). Writing notes stops once no gate that
   still runs reads notes only.
4. GC as a pure plan of whole trees (done), GC's lock window and its timer (done), then the tag ruleset, and the
   signed log.

### The evidence log (in progress)

A ruleset is GitHub's, and GitHub must not be the only thing that protects evidence (`doc/fleet/factory/skip-ci-plan`,
"Portable evidence"). `scripts/evidence-log` keeps a copy of every trusted record on a host we hold and a signed,
append-only list of them; GitHub is only its transport. No inbound port and no new account: a systemd user timer
(`scripts/evidence-log.timer`, every 2 minutes) runs `scripts/evidence-log-timer`, which reads the script from
`origin/main` and pulls over outbound HTTPS.

One pull lists the remote's evidence refs and fetches, by name, only those whose object is new, with full history: a
shallow copy sends every shallow root on every fetch (13 s to fetch 20 records into a copy of 5,000) and cannot push a
record back. It accepts a record that is well formed and
signed by a key main's `.st/evidence-keys` trusts for the signer the record names, keeps it under `refs/evidence-copy/`
(never pruned), and appends one line per record and check to the log. An input-index ref is logged under its key only
when the record carries that key. It then commits the log, every time, even with nothing new: the commit is signed by
the log key and its message carries its time, so a verifier can refuse a head that is not fresh. `--publish` pushes the
head to `refs/evidence-log/main`; a logged record the remote no longer has is reported (`missing`, exit 4), and
`--restore` pushes it back. `evidence-log gc --plan FILE` appends a GC line for each tree and input key of an
evidence-gc plan and drops their copies; it runs before the plan's deletions (`evidence-gc --before-delete`), so a pull
does not restore them. It also keeps the plan's ref names as pending deletions in the copy
(`evidence-log.gc-pending.json`): a pull that runs between the GC lines and the deletions does not log those records
again after the GC line, which would turn their deletion into `missing` and let `--restore` push them back. A name stops
being pending once the remote no longer has it, or after 24 hours if GC never deleted it, and is then logged again like
any record. `gc --publish` pushes the new head at once, so a verifier does not count the deletions as missing until the
next pull.

The log's tree is one file per tree (`trees/<tree[:2]>/<tree>`) and per input key (`keys/<check>/<key[:2]>/<key>`):
`record <record> <check> <exit_code> <signer> <observed_at>` lines and `gc <time> <plan sha256>` lines. A verifier reads
one file at the head. The signer is a command that signs standard input (`ssh-keygen -Y sign -n git -f KEY`, or the same
through `st sekrets` so no seat reads the key), configured on the host in `~/.config/evidence-log/signer.json`; the
log key list is main's `.st/evidence-log-key` (`~/.config/evidence-log/log-keys` on a host whose main has none), and a
pull never extends a head that list does not sign. The signer starts without the caller's loader variables.

**The log key's custody (interim).** The pinned key (principal `log/evidence`) is a plain file on the log host, readable by
any process running as the account that runs the timer, until a `st sekrets` profile holds it. The log is shadow only
until then: nothing skips or refuses on it. Moving the key behind sekrets changes `signer.json` alone, not this file;
replacing the key is a change of `.st/evidence-log-key`, which the gate protects (a change to it never skips).

Measured 2026-10-21 against this repository: the first pull (an empty copy) accepted 4,928 records in 548 s, most of it
checking their signatures, and the copy is 230 MB; the 245 first-layout notes are not logged (their name carries no tree;
GC migrates or removes them). A pull with 46 new records took 7.7 s, one with none 0.8 s. The log's files held 426 KB.

**The verifier's view of the log (shadow).** With `--log-keys FILE` or `--log-keys-from COMMIT` (which reads
`.st/evidence-log-key`; none there means no view), the verifier adds `log`, `log_head`, `log_age_s` and `log_missing`
to its answer, and changes nothing else. `log` is the first of:
- `absent`: no `refs/evidence-log/main` here;
- `unsigned`: the head is not signed by that key (principal `log/...`);
- `stale`: the head is more than 600 s old (five pulls), names no time, or is dated in the future;
- `missing`: a record the log lists for this tree, or for its input key and check, after the last `gc` line is not
  here, because it was deleted or not fetched;
- `unlogged_pass`: the chosen record is a pass the log does not list;
- `ok`.

A failure counts whether or not the log lists it, as the verdict already does. Once the base pins the key, the gate
fetches the one log ref (one commit deep) and prints the view on its decision line. Enforcing it, so that skipping
requires `ok`, is a later change, after the shadow shows what it would refuse. Every read of the log is made with
`GIT_NO_LAZY_FETCH`.

The copy never becomes a partial clone, and no git call may fetch lazily (`GIT_NO_LAZY_FETCH`): in a partial clone a
presence check of a missing record fetches it with no negotiation, that is with its whole history (3.4 GB before the
first record was logged, in the first measurement).

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

`fractal_daemon::evidence::sign_observation` is an API for trusted observers running under
a separate signing account. It validates the statement, measures its own
executable digest, constructs the note, and calls Git's SSH signer. The observer
must construct facts from its own execution; it must never expose this API as a
service that signs arbitrary agent-authored JSON. Test keys are generated in
temporary directories. No production key or key path is shipped here.

The protected CI observer described below writes records from an operator-owned
sekrets profile. An intake review observation adapter is not implemented. Custody
uses a profile whose only granted command is the installed, pinned observer. Custody and trace-receipt isolation
share an OS-account boundary; gate evidence does not claim native agent identity
or reimplement that separate receipt protocol.

## Gate policy and reuse

`.st/evidence-policy.json` in the accepted base commit declares the initial
`daemon` risk class, required `linux-gate` check, allowed `full` tier, and CI/review principal roles.
The workflow must supply an accepted, immutable base: the proposed target cannot
weaken its own required checks or signer roles. A policy change is judged under
the old accepted policy. Missing base policy fails closed.
These generic principals are provisioning targets; no key is trusted merely
because repository policy names it. The caller must separately supply its
operator-controlled trust list. Other risk classes require a later policy
version. The file affects this verifier only; it does not activate
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
checks; there is no low-risk exception. Invalid or untrusted individual records cannot
satisfy a gate, and are ignored when selecting usable evidence; malformed or
rewritten namespace refs fail the transport integrity check.

The CI observer uses `record_degraded` when the protected signer is unavailable.
CI runs independently of the post-completion observer. It preserves the
measured facts in a separate explicitly degraded record; production verification
remains fail-closed under the selected policy. Low-risk policy allowances and
re-attestation deadlines require the gate step and its policy decision. A fresh
full-trust rerun may cite the original with `re_attests`; the verifier checks its
subject, change ID, check names/commands, opaque input keys, and time against that degraded record.
It never changes the original tier. Observing the same old bytes without running
the check again cannot qualify as a re-attestation.

The CI observer reports full-trust record share, signing latency and linked fresh
re-attestation timing as described below. Its throwaway-key test mode emits only
test tier and expires within 15 minutes. Test tier never satisfies production policy.

A rebase that preserves the reviewed paths, their blobs, and the cited test
records keeps the GO. A new unreviewed path, changed blob, deletion, different
change ID, missing citation, failed check, wrong signer role, or changed tested
tree blocks reuse. v1 tests cover the entire exact tree: a rebase incorporating
unrelated source changes needs new tests and a GO citing those tests. Scoped
test input equivalence belongs to affected-set evidence and must not be inferred
from unchanged review blobs alone. Executable modes are covered by the exact
tested tree even though the review map records blob identities.

Observation scheduling, evidence publication, intake signing and typed review-gate
waits require separate integration. The merge queue remains the landing path.

## CI source receipt

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

`change_id` is a free label in the signed predicate. Today it is `github:OWNER/REPO#NUMBER`
for a PR or merge queue entry in the source receipt, `github:OWNER/REPO@SHA` in the aggregate
note, and `local:<commit>` from the local wrapper. Nothing reads a commit message, and no
trailer or ref is needed. A queue entry can contain additional changes: its source
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

## Protected CI observer

The internal `st3 evidence observe-ci --run RUN --attempt ATTEMPT --artifact ID`
command accepts numeric selectors only. It reads its operator-owned configuration
from the profile's private HOME at `.config/st/evidence-ci.json`. A sekrets grant
must allow only that installed command prefix, after review and binary pinning.
The caller cannot supply a statement, repository, check result, signer or key.

The private configuration names the repository, absolute private bare
`scratch_repo`, operator-selected immutable `accepted_base`, pinned
`recipe_sha256` for `scripts/ci-evidence-source`, pinned `workflow_sha256` for
`.github/workflows/fleet.yml`, and `required_checks`. It sets `tier` to `full`, `degraded` or `test`. Full mode also
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
must match. The observer fetches the accepted baseline and authenticated platform
head (plus fixed main history for PR merge parents) from its configured public
repository, with fsck enabled, then imports the bounded source pack with
`index-pack --strict`. Git independently derives the source tree and producer
script digest; neither the artifact nor its self-reported producer hash proves
what executed.

Full trust is limited to `merge_group` and `workflow_dispatch`: their retained
source must equal the platform's authenticated `run.head_sha`. Both the workflow
and producer recipe must exactly match the operator-selected accepted base and
its pins. A workflow change is refused for full trust until separately reviewed
and accepted by the operator. `pull_request` cannot prove the effective tested
merge: its author controls the artifact while run metadata names the author head.
Production PR observations are always degraded, even with a pinned recipe; test
services may emit test tier. A PR observation never satisfies production policy.
The response explicitly reports full eligibility and any trust limitation. The
core producer API refuses a request to sign ineligible source at full tier.

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
hostile workflow author. These limits are why full tier requires a platform-bound
queue/manual source and an accepted workflow pin. Native receipts remain separate
work; this does not certify the behavior of all target code or runtime identity.

## Evidence observe workflow

`.github/workflows/evidence-observe.yml` starts from a finished merge-group run of
`Workspace CI` (or a manual replay from `main`) on runners labelled `evidence-ci`. It never runs
for a pull request or a fork. `scripts/ci-evidence-observe` asks the protected observer
(`$EVIDENCE_OBSERVER`, the operator's JSON argument vector; default
`st sekrets -- st3 evidence observe-ci`) to record the run, imports the returned pack into a
persistent scratch repository, verifies every full-trust record with `scripts/evidence-verify`
against the key list in `main`, creates `refs/notes/evidence/<tree>/<record>` and pushes it. It runs no
check, never replaces an existing ref, and publishes degraded records labelled as unsigned and
unverified. A full record that does not verify fails the job and creates no ref. A cancelled
run has no `linux-gate` job to record (the observer exits 2): the workflow skips cancelled
runs, and a replay of one reports "nothing to attest" instead of failing. The observer's answer
is about 20 MB (the base64 object pack), so the script streams it to a file. The same code
serves a degraded profile (no key) and a full one; the operator's profile decides.

The same script runs outside GitHub Actions, for a timer on an operator host:

```
python3 scripts/ci-evidence-observe --poll --publish --checkout CHECKOUT_OF_MAIN \
  --keys-from origin/main --state-dir /var/lib/evidence-observe
python3 scripts/ci-evidence-observe --run RUN_ID --publish --checkout CHECKOUT_OF_MAIN   # one run
```

`--poll` lists the last `--limit` (20) completed merge-group `Workspace CI` runs, skips cancelled
or skipped ones and runs already done, observes the rest and records each in
`STATE_DIR/observed.jsonl`; a run that fails three times is marked `gave_up` and never retried
(loop protection). `--run` takes the run id alone (the attempt is read from GitHub). Environment:
`EVIDENCE_OBSERVER` (observer argument vector as JSON), `GH_TOKEN` or the operator's own `gh` login
for GitHub reads and the push, and the observer's own `HOME` for its private configuration. State
(`scratch.git`, `observed.jsonl`) lives in `--state-dir` (default `~/.local/state/evidence-observe`).

The controller never records a check that the evidence gate skipped. A gate job that decides to skip ends
with a step named `evidence skip taken` and still concludes success; if the controller recorded the check
anyway, one real run would keep refreshing its own skips. It therefore reads the run's jobs, and any record
whose check has an `evidence-gate-<check>` job whose `evidence skip taken` step ran, or whose own job ran the
step `evidence covered by a trusted note` (a second, independent signal), is neither imported nor
published (the summary names it). If the jobs cannot be read nothing is recorded. It also refuses any run
that is not a run of `Workspace CI`, so it can never act on its own workflow's runs.

The server rejects a shallow push, and the pack carries the source commit without its history, so
the script first fetches the source's missing parents from the repository. Run
`python3 scripts/ci-evidence-observe-test` for the cases (signature verified, verifier refusal,
wrong run, malformed pack, push with a missing parent, replay).

## Recording a local run: evidence-wrap and evidence-sign

`scripts/evidence-wrap --check NAME -- COMMAND...` runs one check once, from a clean checkout, on
the developer's machine, and asks the signer to record it. It measures the commit, the command,
its exit code, the sha256 of its exact output, and an environment fingerprint: the sha256 of
`flake.lock`, of the dev shell derivation path, of the wrapper's own bytes, the system and the
attempt. A failing command is recorded too. It refuses a dirty checkout and a run that changes the
tree. Take the script from `main`, not from the change under test.

`scripts/evidence-sign` runs where the key is (root-installed, hash-pinned, behind a sekrets grant
that allows only that command; configuration in `$HOME/.config/st/evidence-sign.json`). It reads
the commit, tree and `flake.lock` from the caller's checkout, never from the request, uses its own
clock and its own digest as producer, accepts exactly the documented request keys, and signs a
`tests` observation with `runner: "wrapper"`. It cannot check that the command really ran; that is
the cooperative boundary (agents with the key's account can lie) and belongs in the known defects.

`scripts/testdata/evidence/wrapper-note/` is a golden note made with a throwaway key
(`python3 scripts/evidence-wrap-test --write-fixture DIR` regenerates one). The verifier's tests
read it too. A note applies only to the tree it names, so it helps CI only when the pushed commit
has the tree CI will test.

## A CI job that signs its own result

The machine that ran a check can sign the result itself, with no call to GitHub. The key is the
`EVIDENCE_SIGNING_KEY` secret of the `evidence-signing` environment (forks never receive it).
`scripts/ci-evidence-signing` holds the steps that handle it, and all of them are tested without
GitHub: `guard` allows only merge-queue refs, `main`, manual runs on `main` and same-repository
pull requests whose opener, run trigger and commit authors are on the list `.st/evidence-authors.json`
at the base commit (GitHub logins are authenticated; commit author emails are free text and only
catch a stray commit by someone else; a fork never signs); `configure` writes the signer configuration and key as mode 0600 in a 0700
directory and masks every key line; `once CHECK` refuses a second signature for a check in one job;
`cleanup` overwrites and removes the key, and runs even when earlier steps failed. The wrapper and
signer scripts must come from the base commit of the change, never from the tree under test.
Input keys: when the skip policy at the base commit declares `inputs` for a check, the signing job asks the base commit's
`scripts/evidence-verify --project` for the git tree of the tested tree reduced to those paths and signs it as
`observed.inputs.source_inputs`; a base without `--project`, a check without inputs, or any error means an exact-tree
record as before. `scripts/evidence-publish` then also writes `refs/notes/evidence-input/<check>/<source_inputs>/<record>`
for every accepted record (pass or fail, one ref per record, never replaced) so a gate can find the records of
another tree with the same inputs by one prefix fetch. The ref is only an index; a record is still accepted only
by its signature, exact command, required inputs and tier.
Signer launch controls (`scripts/evidence-sign`): each resolved git and ssh-keygen file must be a regular executable file owned
by root and closed to group and others, in addition to the directory rules, and the signer runs that resolved file, not the
link it found. git and ssh-keygen are the only programs the script starts itself; the helper programs git starts come from git's
own installation, which the launcher's pinned git provides and this script does not enumerate, and the interpreter is the
launcher's concern too. A file in the Nix store is judged like the store directory: it is content-addressed, so the rule is no write bit and not who
owns it (inside a Nix build sandbox root-owned store files show as owned by nobody, and the first version of this rule failed the
sandboxed genie-freshness check for exactly that reason). Outside the store the owner must be root. git and ssh-keygen receive only PATH (the
signer's own), LC_ALL, HOME, TMPDIR and git settings the signer sets itself (no system or global git configuration; HOME
and TMPDIR are passed as given, so for a privileged integration only the fixed trusted launcher or profile may supply them,
and passing them proves nothing about an arbitrary inherited path; the "no global git configuration" setting needs Git 2.32 or
newer, and the script does not check the version: the fixed launcher or install manifest that supplies the pinned git must);
and the signer refuses to start when LD_PRELOAD, LD_AUDIT, DYLD_INSERT_LIBRARIES, GCONV_PATH, NLSPATH, PYTHONPATH,
PYTHONSTARTUP or PYTHONHOME is set. The refusal is a tripwire, not a repair: the interpreter has started before the script
runs. Scrubbing the environment, running a root-owned interpreter in isolated mode and fixing the ownership of the signer
file and its directories belong to the launcher, and neither this change nor the CI helper does them. The same-user
and passwordless-sudo gaps below are unchanged.

Accepted gap: code under test in a job that holds the key can read it. That now includes every pull
request from this repository: its author controls the workflow file the run executes, so a person or agent with
write access can read the key and sign any note; the author list narrows who gets a note, it does not
stop that person's code from reading the key. Forks never get it. Narrowing the `evidence-signing`
environment back to the merge queue and main undoes this.

An aggregate job records what it knows: `scripts/ci-evidence-signing aggregate FILE` builds, from
the `needs` context, the needed jobs, the result each reported and the gate job's own result, and
`scripts/evidence-wrap --aggregate FILE` signs that. It does not claim the stages' commands,
outputs, test inventories, timings or toolchains, because it did not run them; those belong in
notes signed by the stage jobs. The signer accepts extra named string inputs next to the required
environment fingerprint, so a check's note can carry more over time; the repository policy decides
which inputs a check must carry.

The `linux-gate` aggregate records only the members of `linux-gate`: it leaves out the evidence gate jobs, the
`fractal-web` lane and the two separate checks `genie-freshness` and `typescript-client`, which have their own notes
(the note job still waits for them, to publish those notes). It refuses, making no note, when the gate of a member
said skip (`evidence-gate-linux-gate`, `evidence-gate-mail-redelivery-canaries`, `evidence-gate-isolation-vm`; a gate
added later counts too unless it belongs to a separate check) or when a member reports `skipped` (only the merge-queue
shards may, outside a merge group), so a member that did not run never reads as success. A skip taken by the
`genie-freshness` or `typescript-client` gate does not stop it.

### Stage notes and publishing

Each `linux-gate` stage job (the test shards, clippy, fleet-compat and the mail canaries) signs its
own result in fail-soft steps after its logs are retained, so signing cannot change the stage's
outcome. `ci-evidence-signing stage-facts` reads what `scripts/ci-linux` wrote: the exit status and
seconds (`STAGE.time`) and the digest of the exact log (`STAGE.log`); a missing or unreadable timing
file is recorded as a failure, never as success. `evidence-wrap --aggregate FILE --out DIR` signs
those facts (and records `workspace_clean` rather than refusing a workspace the tests dirtied) and
writes the note as a pack for upload. A stage job's checkout is shallow and the server refuses a push
from one, so the aggregate note job collects the packs and `scripts/evidence-publish` publishes them.
It does so whenever the job has its key configured, whether or not the aggregate itself signed: when the aggregate
refused (another lane skipped), the stage notes that ran are still published, just without a `linux-gate` note.
Each note stands on its own: its producer made none when its own gate skipped, and the publisher
accepts a note only if its signature verifies against `.st/evidence-keys` from the base commit for
the principal it claims, it is a full-tier single-note commit about this commit and tree, its check
is one of the allowed stage names, and the commit is its only parent. It never replaces a ref and
accepts at most 40 notes in one run; failures and refusals appear in the job summary by record id.

A step-based job may record extra named inputs next to the fixed ones: `STEP_INPUTS` is a JSON object of short
lower-case names with nonempty string values (a shard number, a test set). It can add a name but never replace a fixed
one, and anything malformed makes no note. The aggregate note ignores the jobs of the separate `fractal-web` lane
(`fractal-web` and `fractal-web-*`) and its gate's decision: that lane is skipped whenever no web path changed, which must not
stop the `linux-gate` verdict, and it has its own whole-check note (below).
It ignores `genie-freshness` and `typescript-client` the same way, and their gates' decisions.

The `fractal-web` lane runs only when web paths changed, in one shard on pull requests and two on the merge queue (when kit
inputs changed). Each `fractal-web-execution` shard signs its own result (check `fractal-web`, command `["bash",
"scripts/ci-fractal-web"]`, artifact `evidence-note-fractal-web-shard-N`) and records `kit_shard`, `kit_shard_count`,
`kit_play_set` and `app_test_set`. A shard's note is evidence of that shard only, and the skip policy does not accept it.

**One whole-check note stands for the check.** The aggregate note job also signs a note for `fractal-web` from the lane's
jobs (`ci-evidence-signing aggregate` with `GATE_JOB=fractal-web`, the step "Sign the fractal-web note"), with the same key,
environment and permissions as the `linux-gate` note it signs beside it; the always-run `fractal-web` job holds no key. Its
command is `["fractal-web", "aggregate", "fractal-web", "fractal-web-changes", "fractal-web-execution"]`; its exit status is
the `fractal-web` verdict's. It records every job's result, the shard count (`kit_shard_count`) and each shard's result
(`shard_results`), whether kit plays ran (`kit_inputs`) and the test sets (`kit_play_set`, `app_test_set`); the count and sets
come from the same workflow expressions as the matrix and the run step. GitHub gives a dependent job one result for a matrix
job, `success` only when every shard succeeded, so the helper refuses (and no note is made) when any shard was skipped, failed
or was cancelled, when the lane did not run (detection failed or said no web path changed, so the execution was skipped),
when the verdict did not finish, and when the lane's evidence gate took a skip. A skipped or failed shard therefore never
reads as success. A pull request's note says `kit_shard_count` 1, `kit_play_set` `commit`, `app_test_set` `related`: it is
honestly a one-shard note, and it ran everything a pull request runs, so it may cover a pull request run. It must never
cover a merge-queue run, which runs both shards of everything: that is the policy's `events` (an enforce flip is
`"enforce": true, "events": ["pull_request"]`, as for `typescript-client`), not a new verifier rule. The note is published
by the same step as the stage notes, which runs whenever the key is configured, so it is published even when the `linux-gate`
aggregate refuses.

A run that did not execute the lane makes no note and the shadow report reads `n/a`. The skip policy's `fractal-web` entry
is shadow (`enforce` false, `fleet` only; see "The skip policy"), and `evidence-gate-fractal-web` reports its decisions like
the other gates. The execution shards read it the way `typescript-client` does: each shard keeps running and skips its steps
(never the checkout) on a skip, and its own note then refuses, so turning `enforce` on is the only change a skip needs. With
`enforce` false the gate never says skip. The verdict job still runs, and the merge queue's kit coverage step never reads the
gate (a skip there would leave no receipts and fail the check). The local `scripts/web-skip-proof` reports a matching record
under a shadow entry as `decision=run reason=shadow`, never `skip`.

### Shadow verification

After publishing, the aggregate note job runs `scripts/ci-evidence-shadow` (from the base commit, fail-soft,
skipped while the base has none). For every lane it published or should have published it fetches only that
lane's record from the remote at depth 1 into an empty scratch repository and runs the standalone verifier
there with the base commit's key list. That is what a later gate that fetched the refs would see, not this job's
own local refs. A lane prints `ok`, or `would-reject` with the verifier's reason (`no_note` for a lane this run did
not sign, `not_on_remote`, `check_failed`, `untrusted_level`, `stale_tree` and the rest of the verifier's reasons). The job summary carries the table and a
total (`lanes`, `ok`, `would_reject`, `refs_added`, seconds); each line is also in the log as
`evidence-shadow v1 ...`. A lane listed in `--optional` (one that runs only when its paths changed, such as
`fractal-web`) reads `n/a not_run` when this run made no record for it and is counted apart. It gates nothing and never
changes the job's result.

### What reaches the signing environment

Only the merge queue and `main` reach the `evidence-signing` environment: every signing job's
`environment:` is the expression `merge_group` or push to `main` ? `evidence-signing` : empty, and the
aggregate note job does not run for pull requests at all. A pull request, including one from this
repository, runs the same jobs without the environment, gets no key, and the guard step refuses it again
in code. **The environment must not gain required reviewers or a wait timer:** eleven required check jobs
carry it in the merge queue, so a protection rule that waits would block every required check. A
deployment-branch policy comes only after this conditional is on main (a policy that excludes the queue
refs would otherwise fail those jobs at start). Signing jobs also refuse to sign when tracked files
differ from the commit (`git status --porcelain --untracked-files=no`): the code under test ran in that
checkout, and the note names `HEAD`'s tree. Untracked build output is recorded (`untracked_files`) and
does not stop the note. `git status` can be told to ignore a tracked file (`--assume-unchanged`,
`--skip-worktree`), so the index flags are read too (`git ls-files -v`: every tracked file must be tagged
`H`). Untracked and ignored files are not refused, by design, because build output lives there; the note
therefore says "tested" about a checkout whose tracked files equal the tree it names, plus whatever
untracked or ignored files the job had. The signer finds `git` and `ssh-keygen` only in its own path: the
standard system directories, or the `path` in its configuration (`EVIDENCE_TOOL_PATH` when
`ci-evidence-signing configure` writes it), which is how the Nix sandbox, where they live in
`/nix/store`, supplies them without trusting the caller's PATH.

## A check derivation as evidence: the `nix` record

A required check that runs in the Nix sandbox can be a check derivation (`checks.<system>.<check>` in `flake.nix`). It only
produces output when its tests pass, and the output path hashes every input (source, `flake.lock`, toolchain, build script),
so a valid path is both the proof that these inputs passed and a cache: an unchanged tree needs no rebuild. Today
The first one built this way is `checks.genie-freshness` (about 163 s cold, 0.4 s warm on hetz2; a change to any file in the flake
source still rebuilds it, because its source is the whole flake).

`scripts/nix-check-facts --check NAME --out FILE` builds the derivation and writes the facts for
`scripts/evidence-wrap --aggregate FILE`: `runner` `nix`, the command `nix build .#checks.<system>.NAME`, the exit code and
`output_sha256` (the SHA-256 of the NAR hash string), and the inputs `drv_path`, `out_path`, `nar_hash`, `nar_size`,
`references` (sorted store paths joined by commas, `-` for none), `cache_hit`, `nix_signatures` (the daemon's `name:base64`
signatures, `none` when unsigned), `nix_key_id` and `nix_unsigned`. A path that is already valid is not built again; the record
says `cache_hit=true` and names the same derivation, which is the proof that an unchanged tree skips the rebuild. A failing build
is recorded too (exit code 1, the derivation, no output path) and never satisfies a check.

`scripts/evidence-sign` accepts `runner: nix` only for that command shape and holds the inputs to their formats (store paths, the
output named like its derivation, a `sha256-` NAR hash, sorted references, signatures as the daemon prints them). It cannot see
the store, so it does not judge the Nix signature. The verifier does, in a later slice: it recomputes the fingerprint
`1;out_path;sha256:<nar_hash in Nix base 32>;nar_size;references` itself, and trusts a Nix signature only when its key name and
public key are in `.st/evidence-nix-keys` (Nix `trusted-public-keys` lines) read from the accepted base. A record cannot name its
own key. The record needs both signatures: the SSH wrapper key, and at least one valid Nix signature from a different key.

**Residual trust.** The Nix key proves the content of the path is genuine. Only the wrapper key says "this tree built to this
path": a cache hit shares one derivation across trees, so the path cannot name a tree. A stolen wrapper key could attach a genuine
old path to a new tree; the `flake_lock_sha256` bind and `max_age_s` limit that, they do not close it.

**Key custody.** The Nix key is the machine's own (`nix.conf` `secret-key-files`, readable by the Nix daemon only). ci1 and ci2
have none today (checked 2026-10-20), so no record can be signed by a production Nix key yet; tests and the sample in
`doc/fleet/factory/nix-evidence/nix-sample-20261020` use a throwaway key.

## Fresh CI re-attestation

The protected CI adapter accepts repeatable `--re-attest RECORD_ID` selectors
alongside its run, attempt and artifact selectors. These are immutable degraded
check records, never caller statements. Configuration, repository, roles, keys,
workflow and recipe pins still come from the private operator profile.

The observer fetches predecessor refs from the fixed repository and verifies
that each contains one explicitly unsigned degraded CI check. It derives the
old repository/run/attempt/job identity from that record, then reads the original
attempt's run metadata and job objects from authenticated GitHub. The run must
belong to the configured repository and completed Workspace CI workflow path;
an unrelated workflow with the same check name cannot supply a predecessor.
It requires an exact job ID,
check name, conclusion, serialized job digest and completion timestamp match.
One run/job read pair per distinct old attempt supplies all its selected jobs.

The current run must independently pass the existing source and workflow trust
boundary. A link needs identical change identity, whole-tree digest, check name,
command and opaque input map. It also needs a different run or attempt and job,
with the fresh job starting after the old check completed. Reusing the original
job, changing an input, or inventing an old result cannot produce a link. There
is at most one predecessor per current required check. All links are validated
before any new observation is written.

Only a newly produced Full record gets `re_attests`. Signing failure preserves
the fresh facts at degraded tier with no completed re-attestation link. Replay
retains the original result and never promotes it. An already recorded Full job
cannot acquire a different link; request the link on its first observation or
run a fresh check. Test-tier services never produce Full re-attestations.

The data-only export includes linked predecessor notes and their source commits
so offline verification can validate the chain. The predecessor stays degraded;
its record is never modified. This is an explicit selector API, not automatic
rerun scheduling or shared fallback coordination. Production controllers still
need to publish refs and provide the correct predecessor selectors.

The response adds `re_attestations` samples and `re_attested_records`. Each sample
measures from the authenticated degraded check's completion to the signed Full
note's issuance time. This includes scheduling delay after the rerun completed;
using rerun completion alone would understate time to trusted evidence. Git note
issuance has one-second precision; elapsed values are expressed in milliseconds.
The batch's `re_attestation_elapsed_ms` is the maximum new sample, or null when
none exists. Replays contribute no new samples. Full-trust share and signing
latency keep their existing fresh-record denominators. Production aggregation,
risk-class deadlines and their enforcing policy remain separate work.

A predecessor's conclusion may be failed. A passing rerun still links to it: the
link says the rerun re-checks the same tree, not that the verdict is the same.
Consumers must read the new record's own result, never infer it from the link.
If the observer clock reads earlier than the degraded completion, the sample has
`elapsed_ms` null and `skewed` true; the signed record stays valid and the sample
is left out of `re_attestation_elapsed_ms`. That value is a measurement only, not
a policy input, until deadlines and the production controller are decided.

## Standalone verifier

`scripts/evidence-verify` answers one question without a built st: does a trusted,
signed note exist for this exact tree and this check? It needs only `git`,
`ssh-keygen` and Python 3, reads the same `refs/notes/evidence/<tree>/<record-id>` notes
(one SSH-signed commit per record), and finishes in well under a second on a
repository with hundreds of records. `st evidence verify` stays the reference
implementation; the script's tests use real Git repositories and real SSH
signatures.

```sh
scripts/evidence-verify --repo . --revision HEAD --check genie-freshness \
  --keys-from BASE_COMMIT [--command '["nix","build",".#checks.x86_64-linux.genie-freshness"]']
```

Exit 0 is satisfied, exit 1 is not satisfied, exit 2 is a usage error. Both print one JSON
document: `{"satisfied":true,"tree","check","record","kind","signer","trust_level","tier","observed_at","command"}`
or `{"satisfied":false,"reason","check","detail"}`.

Trust comes only from the key list: `--keys FILE`, or `--keys-from COMMIT`, which reads
`.st/evidence-keys` (OpenSSH allowed-signers syntax, `namespaces="git"`) from the
accepted base, never from the change under test. A signer's trust level is the part of
its principal before the first slash (`machine/ci1` is level `machine`); it is never
read from the record. Removing a line revokes every note that key signed.

`kind` comes from the record's `runner`: `nix`, `wrapper` (a command run once and signed by the wrapper) and `sekrets` name themselves, and a runner that starts with `github:` is kind
`observed`, the central observer's record of a finished GitHub job (`st evidence observe-ci`).
Anything else is `unsupported_kind`. An unsigned record at tier `degraded` is never evidence
and is reported as `degraded`, not as a signature fault, because it is what the observer writes
when its signer is unreachable. The latest
trusted record for the tree and check decides, so a newer failure supersedes an older
pass; `observed_at` is the signer's own clock, so a pass replaces a failure only when it is dated MORE than two minutes after it (the same margin as
`future_dated`): equal, inverted or within-the-margin dates leave the failure standing, whatever order the notes were created in. Reasons: `no_record`, `stale_tree` (records exist only for other trees),
`wrong_check`, `bad_signature` (forged or unsigned), `untrusted_signer` (a valid
signature from a key not on the list, or a principal that differs from the record's
signer), `untrusted_tier`, `untrusted_level`, `wrong_runner`, `too_old`, `future_dated`, `degraded`, `unsupported_kind`, `check_failed`, `wrong_command`, `wrong_input` and
`error` (unreadable input: no decision was made, count it separately). Notes are found
by the tree they name with one `git cat-file --batch-check`, so the cost grows with the
number of evidence refs; pack refs, or add a per-tree index, before they reach the
thousands.

The notes may be fetched shallow, as a CI checkout does: the note commit's parent is read
from the raw commit, and when the subject commit is not in the local repository the signed
statement's tree stands. When the subject commit is present its tree must equal the
statement's, so a record cannot be rebound to a different tree. The tree claim is the
signer's: the signature covers the statement, so only a key on the list can make it.

`--kinds nix,wrapper` limits the record kinds a check accepts. `--max-age SECONDS` rejects a record observed longer ago than that (`too_old`) or claiming a time
more than two minutes ahead (`future_dated`). `--runner-prefix github:ORG/REPO/` requires the record
to come from this repository's runs (`wrong_runner`; keep the trailing slash, it is what stops `repo`
matching `repo2`). `--trust-levels fleet,machine` limits which signer trust levels a check accepts
(`untrusted_level`); a newer failure from a level the check does not accept still stops a skip.
A record claiming a time more than two minutes ahead is always `future_dated`, with or without
`--max-age`. Every one of these options defaults to permissive, so **a check that skips on an
observed record must pass `--kinds`, `--command`, `--runner-prefix`, `--trust-levels` and
`--max-age` explicitly** (or, better, `--policy-from`). Any unexpected exception is a JSON
refusal with reason `error`, never a traceback and never a pass.

`--bind-input KEY=PATH`
(repeatable) requires the record's input `KEY` to equal the sha256 of `PATH` as it is in
the tree being verified, for example `flake_lock_sha256=flake.lock`, so an environment
fingerprint is tied to the tree and not to a claim; any mismatch, missing key or missing
file is `wrong_input`. Record inputs are opaque nonempty strings; the producers use
`flake_lock_sha256`, `toolchain_sha256` and `wrapper_sha256` (digests) and `system` and
`attempt` (plain strings), and only keys named by `--bind-input` are interpreted. Success
and most failures report `attempts`, the number of trusted records for the tree and
check, so a producer can refuse to run a third time.

A check that calls the script must pass `--command` with the argument vector it
expects; without it, any command a trusted signer recorded under that check name
satisfies the check, and the recorded output hash is not compared. A record whose
`observed_at` has no time zone is not evidence, because naive and aware times cannot
be ordered against each other.

This slice verifies SSH-signed `tests` records. The raw Nix output-path signature
check for the typed `nix` observation is a later slice, agreed with the Nix producer.

## Evidence gate and the skip policy

Six non-required jobs, `evidence-gate-<check>` for each required check (`linux-gate`,
`typescript-client`, `isolation-vm`, `genie-freshness`, `mail-redelivery-canaries`) and for the `fractal-web` lane, decide whether
a trusted note already covers the run's tree. They hold `contents: read` only, write nothing and dispatch
nothing, and every problem becomes `decision=run`. **Every required check now consumes its gate's decision**: `genie-freshness` and `typescript-client`
(by skipping their steps), and the Linux group (below); the `fractal-web` execution shards consume theirs the same way
(its entry is shadow, so it never says skip).

### What a skip does

When `evidence-gate-<check>` decides `skip`, the job of that check still runs and still ends in success, so it
keeps its name, its place in the required set and every `needs` on it. It runs only the checkout and its own
cache-coverage report, skips every other step, and adds the step `evidence covered by a trusted note`, which
writes the record id to the job summary. The wiring is deliberately conservative:

- Each wired job also waits for its gate job (a hosted runner start, typically 10 to 30 seconds), so while no
  note exists these two checks start slightly later than before. That is the price of a single, auditable
  decision.
- A wired job's own `if` names a status function (`!cancelled()`), so a gate job that failed, was skipped or
  never started cannot skip the required check: its output is empty, which is not `'true'`, and the steps run.
- The checkout is never skipped, and a step that already had an `if` keeps it, combined with the skip
  condition. A step that does not know about the skip runs.
- The skip decision the check jobs read is the output of the `evidence skip taken` step itself, written as its last
  command, not of the earlier decision step. So `skip=true` cannot exist unless that marker step already ran: a gate
  job that dies between deciding and marking leaves no output and the check runs. The observer refuses to record a
  check whose gate job ran the marker (and should also refuse one whose own job ran `evidence covered by a trusted
  note`, the step a wired job runs only when it skipped its work), and the in-job producers refuse to sign for a
  stage that did not run, so a skip never produces a fresh note for itself.
- Every signing step is told when its gate skipped: it gets `GATE_SKIP` (true when any gate that can cover that job
  said skip), and the signing helper then makes no note whatever the job's log or step outcomes say. The aggregate
  note job `needs` every `evidence-gate-*` job, so it sees them all: the `linux-gate` note refuses when any gate but
  `fractal-web`'s says skip, and the `fractal-web` note when that one does. Each
  gate a job reads is one of that job's own `needs`, which a test checks.
- A skip of `genie-freshness` skips the tests of this machinery too (`ci-evidence-gate-test`,
  `evidence-verify-test`, `genie --check`) for that tree. That is accepted because the note is for the same tree and
  a change to those files is a protected path that always runs in full; a reader must not assume they ran.
- What is protected and what is bound otherwise. The protected paths are the gate, the verifier, both of their
  tests, the policy, the evidence keys, the workflow source (`fleet.yml.genie.ts`), the helper module it imports
  (`workspace-ci.ts`) and the generated `fleet.yml`. The signing and publishing wrapper scripts are not on that list:
  they are bound by the note itself, which carries their `wrapper_sha256`, so a changed wrapper cannot produce a note
  the policy accepts.
- A required check that is a job, `isolation-vm` and `mail-redelivery-canaries`, never reports `skipped`; under an
  enforced skip it reports success with its steps skipped, so once a check is enforced the name means "covered by
  a trusted note", not "ran". Whoever reads a result (including the observer) must treat such a run as covered, not
  freshly run; the job's summary says so (`evidence covered by a trusted note`).
- The aggregate `linux-gate` accepts `skipped` for a stage only in the stage's position and only when that stage's
  own skip rule evaluates true (the workflow passes one flag per stage), so a stage skipped for any other reason
  fails the gate even when the aggregate's gate also decided to skip.
- Step conditions are combined as text only when none contains a marker inside it; the helper that combines them
  refuses otherwise and a test checks every condition of the generated workflow. Keep actionlint in the required path.
- The skip rests on a note of kind `observed`, which the central observer records only for a `merge_group`
  run, so a pull request can skip only a tree an earlier queue run already passed. Do not expect a saving at
  pull-request time from these two checks.
- Until a producer writes a note for a check, that check never skips. At the time of writing no note exists for
  these two, so both run in full; the first skip needs a producer.

### The Linux group

`linux-gate` is an aggregate of jobs that are not themselves required checks, so those jobs skip at the **job**
level, which allocates no runner at all: `linux-tests` and its three shards, `linux-clippy` and
`linux-fleet-compat` skip when `evidence-gate-linux-gate` decides skip. `linux-test-build` feeds the test shards,
`mail-redelivery-canaries` and `isolation-vm`, so it skips only when **every one of them** is covered; that
condition is derived from theirs in the generator, never written twice. The two jobs that are required checks in
their own right keep running and end in success with their steps skipped, so a required check never reports
`skipped`: `mail-redelivery-canaries` is covered by its own note or by `linux-gate`'s (the aggregate note proves
its canaries passed), `isolation-vm` by its own note; both gate their first step, the pre-check that the shared
build succeeded.

`linux-gate` itself still runs always and still fails on anything but success, with one addition: it reads its gate's
decision, and a member that reports `skipped` is accepted only when that decision was exactly `true`, only
in the **positions** of the stages that are skipped at job level (the build, the test shards, clippy and fleet
compatibility, derived in the generator from the very list that skips them), never for a failure or a
cancellation, and the stage counts are still enforced. `upgrade-impact` and `mail-redelivery-canaries` are never
accepted as skipped, so a condition that skipped them for any other reason cannot be waved through just because a
note exists. A skipped job reports `skipped` to
the aggregate evidence-note producers, which refuse to sign for a skipped member, so a skipped run writes no
fresh `linux-gate` note.

### The skip policy

`.st/evidence-skip-policy.json` is read from the accepted base (the pull request base or the merge
group base), never from the change under test, and says per check what a note must be:

```json
{"version": 1, "checks": {"linux-gate": {
  "trust_levels": ["fleet"],
  "enforce": false,
  "max_age_s": null,
  "kinds": {
    "observed": {"command": ["github-actions", "{repository}", ".github/workflows/fleet.yml", "{check}"],
                 "runner_prefix": "github:{repository}/"},
    "wrapper": {"command": ["..."], "required_inputs": ["flake_lock_sha256", "toolchain_sha256"],
                "bind_inputs": {"flake_lock_sha256": "flake.lock"}}}}}}
```

`enforce` is the per-check switch for skipping and is **off unless it says `true`** (absent means
false; any other value is an error). With it off the gate still looks the note up and verifies it, and
prints `outcome=satisfied` with `decision=run` and a reason starting `shadow:`, so the proof that a
note would have covered the check accumulates while the check keeps running as today. Turning a check
on is the one-line change `"enforce": false` to `"enforce": true` in this file, reviewed and queued
like any change (the file is read from the accepted base, so the change takes effect for the runs
after it merges).

`inputs` (optional, per check) lists the repository paths the check's result depends on. With it, a record
of ANOTHER tree counts when it carries the same input key: `source_inputs`, the git tree id of the tested
tree reduced to exactly those paths (`git write-tree` of only their files; `.` means everything, whose key is
the tree itself). `scripts/evidence-verify --project --check C --paths-from BASE ...` prints it, so a producer,
the observer and the gate compute the same key. The producer signs it as `observed.inputs.source_inputs` and
publishes the record under `refs/notes/evidence-input/<check>/<source_inputs>/<record id>`; the gate fetches only
that prefix, and the ref is just an index: whatever it finds must still verify and carry the same signed key.
Every other rule applies unchanged, and the latest record for the key decides, so a newer failure with the same
key stops the skip by itself. Paths are literal and relative; an under-declared list is the dangerous direction
(a stale pass), so each list is reviewed against a trace of what the check opens. `sample_one_in` (default 1)
keeps running the real job for one run in N even when a note matches, chosen by a hash of the check, the key and
the head, so a retry behaves the same; the gate prints `decision=run` with a reason starting `sampled`.

`linux-gate` declares its inputs too (shadow only: `enforce` stays false, `sample_one_in` 5). The list is the Rust test stage's
(`doc/fleet/factory/nix-evidence/rust-test-declared-inputs`, a textual scan of what the Rust sources name) plus what the other
members of the aggregate read: the whole of `.github` (workflows, the clippy and fleet-compat baselines) and `scripts` (the stage
scripts and their tests), `fixtures` and `templates`. A pull request that changes none of those paths (documentation outside
`docs/st2`, `docs/st3` and `docs/vrs`, `apps`, `packages`, `release-notes`, `AGENTS.md`, `README*`) has the same key as a tree
on which the whole gate passed. `slo` (`include_str!` of `slo/targets.toml` into the st3 crate), `evals` (the example and graph tests read `evals/st3`),
`claude-channel` and `plugins` (the plugin files `include_bytes!`d into fractal-drivers) and `docs/st2` (`tests/invariants.rs` includes
`docs/st2/invariants.md`) are declared because Rust sources open them; a committed-policy test fails when any literal `include_str!`,
`include_bytes!` or `include!` path in the Rust sources resolves outside the declared inputs. `release-notes` is deliberately not declared: the Rust sources only name `release-notes` as an
artifact name, and `upgrade-impact`, which reads the fragments, always runs. The one file under `apps` the Rust tests read at run time is declared by name
(`apps/fractal-web/src/terminal/assets/JetBrainsMonoNerdFontMono-Regular.ttf`, opened by a test in `crates/fractal-daemon/src/attach_image.rs`). The scan misses paths a test builds at run time, so
the first runs are shadow: a miss shows up as a satisfied note beside a failing real job, and `scripts` and `.github` are declared
whole until a per-file list is reviewed against a trace of what each stage opens.

`fractal-web` declares its inputs too (shadow only, `sample_one_in` 5). The list is every path the lane's detector
(`scripts/ci-fractal-web-changes`, embedded in `fractal-web-changes`) matches, written as the directories and files that exist. That
includes four paths the lane reads although no web source names them, which the detector now matches too: `apps/ios/package.json`
(a workspace member, so the frozen install reads it), `docs/st3/client-v0/fixtures` (an SDK unit test opens `timeline-conformance.json`),
`pnpm-source-inputs.json` and its `.genie.ts` (the web shell hook publishes the source inputs it lists) and `scripts/check-public-repo`
(the scenario privacy lane runs it); before, a change to only them did not run the lane at all. They are lane inputs, not kit inputs:
a change to only them runs the lane without the kit plays. The privacy lane also fails when that lint fails outside its own
scope; `genie-freshness`, whose key is the whole tree, runs the same lint for every tree. A policy test fails when a tracked path
the detector matches lies outside the declared list.

**Whole-tree lints run in genie-freshness, not in the Linux gate.** Two lints read every tracked file: the public-repository check
(`scripts/check-public-repo`, formerly called from `tests/public_repo.rs`) and the retired-name check (`scripts/check-native-names`, formerly
`tests/native_only.rs::tracked_product_surface_contains_only_native_names`). Run inside the Linux gate they would see a page under `apps`, `packages` or
`release-notes` that the declared inputs leave out, so a skip by input key could hide their failure. `scripts/ci-genie-freshness` runs both; that check's
key is the whole tree, so it runs for every tree the Linux gate skips. `tests/vrs_ledger.rs` reads only `docs/vrs`, `src`, `tests` and `crates`, all
declared. A policy test (`test_no_rust_source_lints_the_whole_tree`) fails when a Rust source lists the whole tree with `git ls-files` or calls one of
those scripts. The aggregate `linux-gate` note signs its input key like a stage note (its job has no `PROJECT_BASE_SHA`, so
`ci-evidence-signing` takes the same base from the event payload), which is what publishes `refs/notes/evidence-input/linux-gate/...`. The skip decision is taken once per stage group
before the partitions (`needs.evidence-gate-linux-gate.outputs.skip` on the shards, clippy and fleet-compat), so the four shards
share one key.

`isolation-vm` and `mail-redelivery-canaries` declare the same list (shadow only too: `enforce` stays false, `sample_one_in` 5),
so the three lanes share one input key. Both are members of the `linux-gate` aggregate, whose list already covers what its members
read, and neither reads anything outside it. The canaries are `scripts/ci-linux mail-redelivery-canaries`, which runs
`scripts/ci-mail-redelivery-canaries`: a fixed set of the Rust tests from `linux-test-build`'s archive, with the `ci` profile of
`.config/nextest.toml`. `isolation-vm` builds the two VM drivers from `flake.nix`, `flake.lock`, `nix/transport-isolation-vm.nix` and
`nix/sekrets-vm.nix` (the `pty` package is a flake input), the provider components from the workspace's Rust sources, and runs the
same archive and the `sekrets-st3` binary of the same build, with only `Cargo.toml` copied into the VM; its other steps are
`scripts` and the baselines under `.github`. Each job signs its key itself (`STEP_CHECK` and `STAGE_JOB` are the policy's check
names, and both steps set `PROJECT_BASE_SHA`), and the aggregate job's publish step indexes every accepted record with a key, so
nothing in the workflow changes.

The projection reads names as bytes and passes them NUL-terminated both ways (`ls-tree -z`, `update-index -z`),
so a newline in a name cannot split one file into two entries; symlinks and gitlinks keep their modes. When none
of the declared paths exists the projection would be the empty tree that every such tree shares, so there is no
key and nothing matches by it. Two limits are deliberate. The verifier does not recompute the projection of the
record's own tree: it trusts the key the producer signed, so the producer is trusted exactly as far as for the
tree rule. And sampling is a comparison sample, not a security control: the head is in the seed, so on a pull
request an author can re-push to change whether a run is sampled (about five tries at one in five), while in a
merge group the head is fixed. A result that does not carry the sampling rule at all is treated as sampled.

Each kind has its own rule: the exact `command` the record must have run, an optional
`runner_prefix`, optional per-kind `trust_levels`, the `required_inputs` it must carry (every one a nonempty string) and
`bind_inputs`, which must equal the sha256 of a file as it is in the tree being verified. A kind's own
`trust_levels` replaces the check's for that kind only, so a weaker signer (say `local`, the CI's
own runner signer) can be admitted for `wrapper` notes without ever satisfying `observed`; a note
never changes kind, since the kind comes from the record's runner and each kind keeps its own
command and inputs.
**Adding an input to `required_inputs` tightens future notes without breaking anything**: a
note that lacks it simply stops satisfying the check (`missing_input`). `max_age_s` is `null`
by default (no age limit; a signature is meant to grow to cover more over time); a note claiming
a future time is always refused. The file is validated strictly: an unknown key is an error, so
a typo cannot silently weaken a rule, and a check with no entry never skips. The tree, the check
name, the command, the exit code and the output hash are part of every record and are always
checked. `observed` (the central observer's record of a finished job) and `wrapper` (a note the in-job producer signs
as `fleet/ci`) are listed for **every** required check, each with the exact command that producer records:
- `linux-gate`: `["linux-gate", "aggregate", <the aggregate's needed jobs, sorted>]`, without the gate jobs, `fractal-web-*`,
  `genie-freshness` and `typescript-client`;
- `mail-redelivery-canaries`: the stage command `["bash", "scripts/ci-linux", "mail-redelivery-canaries"]`;
- `genie-freshness`: `["bash", "scripts/ci-genie-freshness"]`; `typescript-client`: `["bash", "scripts/ci-typescript-client"]`;
  `isolation-vm`: `["isolation-vm", "nixos-test-driver", "transport-isolation-vm", "sekrets-vm"]`;
- `fractal-web` (shadow, `wrapper` only): `["fractal-web", "aggregate", "fractal-web", "fractal-web-changes", "fractal-web-execution"]`,
  the whole-check note the aggregate note job signs from the lane's jobs, which must also carry `kit_shard_count`, `shard_results`,
  `kit_inputs`, `kit_play_set` and `app_test_set`. A shard's own note (`["bash", "scripts/ci-fractal-web"]`) does not match it.
  It lists no `observed` kind: the observer's record is of the always-emitted `fractal-web` job, which passes
  without running the lane when no web path changed against that run's base.
Every wrapper rule requires `flake_lock_sha256`, `toolchain_sha256`, `wrapper_sha256`, `system` and `attempt`
and binds `flake_lock_sha256` to `flake.lock` in the verified tree; the stage rule also requires `stage` and
`job`, the step-outcome rules `job`, `steps` and `step_outcomes`, and `fractal-web` its shard and test-set inputs. No rule names a weaker signer level, so only
`fleet/ci` counts. A producer signs a step-outcome note only when every listed step actually ran, and refuses
when a gate in its run said skip, so a skipped run writes no note for itself. If a producer's note changes (a job
added to the aggregate, a step renamed), the policy must change with it; until it does, the notes simply stop
matching and the check runs.
Write `scripts/evidence-verify --policy-from BASE
--repository OWNER/NAME` to apply it by hand.

A kind's rule may also name `by_level`: for one signer level (the first part of the principal), its own `command`, `required_inputs`,
`bind_inputs` and `events` (`pull_request` and/or `merge_group`; `events` is required, so an override can never silently cover every event) replace the kind's. The level must be one the kind accepts. It is how a
host-signed local note (its command is the `nix develop ...` line `scripts/st-test` runs) can satisfy a check whose CI step wrapper records a
different command and extra inputs, while each signer is still held to exactly its own rule. The verifier reports the `events` that apply
and the gate skips only on those events: a note whose signer level lists `pull_request` only never covers a `merge_group` run. Today the
`host` level is admitted for the `wrapper` kind of `genie-freshness` and `typescript-client`, for pull requests only.
A level's rule may also say `"lane": true` (default false): that level's notes are then held to the lane of the check in
`.st/required-lanes.json` at the same accepted base as the policy, exactly as `--lanes-from` holds any note: the tree must not change
the lane's definition paths (`definition_changed`) and the note must carry that base's `lane_definition_sha256` (`wrong_definition`); a base
without a lane for the check refuses such a note (`no_lane`). Other levels are unaffected. The `agent` level (below) uses it.

A check may also name `events` itself (optional; the same two values). It limits the runs ANY note for that check may skip, whoever signed it,
and a signer level's own `events` can only narrow it further (the verifier reports the intersection, which may be empty: then nothing skips).
Absent means every event the gate allows. This is how a check is enforced for pull requests while the merge queue keeps testing the combined
tree: `"enforce": true, "events": ["pull_request"]`.

### Which checks skip today

| Check | `enforce` | `events` | `sample_one_in` | Since |
| --- | --- | --- | --- | --- |
| `typescript-client` | true | `pull_request` | 5 | this change |
| `genie-freshness`, `linux-gate`, `isolation-vm`, `mail-redelivery-canaries` | false | - | 5 or 1 | - |
| `fractal-web` (whole-check note; one shard's note never counts) | false | - | 5 | - |

`typescript-client` was turned on after 48 hours of shadow decisions (2026-10-17 to 2026-10-19, 585 pull-request runs): 378 satisfied by a
trusted note, 254 of which would have skipped (the rest were sampled or touched a protected file), and not one satisfied note sat beside a failing
real job; the published notes' shadow verification (`evidence-shadow v1`) was `ok` for every lane in every merge-group run since it landed. The
sampled one in five keeps comparing the note with a real run. Each further check is its own reviewed change to this table, the policy and the
`enforced` lists in `ci-evidence-gate-test` and `evidence-verify-test`.

### When a check is ready to flip (`scripts/evidence-flip-ready`)

The trigger for turning a further check on is read from the gate's own decision lines, once the base pins the evidence log key
(the log view, "The evidence log"). `scripts/evidence-flip-ready` reads the `evidence-decision v1` line of every `evidence-gate-<check>`
job of the `Workspace CI` pull request runs created in the last 24 hours (job logs through `gh api`, cached by job id) and, per check in
the policy at origin/main, says `ready` when all of these hold, and otherwise lists `why_not`:

- 24 hours of fresh published log: no line in the window says `log=stale`, `absent`, `unsigned` or `-` (the base did not pin the
  key yet), and the first line with a log value after the last `-` (`log_window_start`) is at least 24 hours old;
- at least 30 lines with `outcome=satisfied` and `log=ok`;
- no `log=missing`;
- no unexplained `log=unlogged_pass`: one is explained when the record was observed at most 240 s (two log pulls) before the run was
  created, or after it;
- no hidden red: no satisfied line in a run (and attempt) whose real job, the job named exactly the check, concluded `failure`. A real
  job the gate skipped is evidence neither way. For the aggregate `linux-gate` the red is hidden only when a member that a skip skips
  (the jobs whose `if` reads `needs.evidence-gate-linux-gate.outputs.skip`: `linux-test-build`, `linux-tests` and its shards,
  `linux-clippy`, `linux-fleet-compat`) failed or was cancelled; a red from a member that always runs (`upgrade-impact`) would show
  under enforce too, and is reported as `visible_red`, which does not block.

A check can also carry preconditions on its own policy entry at origin/main, whatever its decision lines say. `linux-gate` has one:
its clippy stage ratchets against a base, so it is not ready until the policy requires `clippy_base_sha` of both its `host` and `agent`
notes (each note then names the base it was measured against, not whatever a local `origin/main` happened to be).

It prints JSON (`checks.<check>`: `enforce`, `lines`, `satisfied_log_ok`, `log_missing`, `unlogged_pass_unexplained`, `hidden_red`
and `visible_red` run ids, `log_window_start`, `ready`, `why_not`) and exits 0, or 2 when GitHub could not be read. Enforced checks are reported too.
`--propose` opens the flip for each ready check that is not enforced and has no open pull request from `evidence-flip/<check>`: in a
temporary worktree of origin/main it sets `"enforce": true` and adds `"events": ["pull_request"]` and `"sample_one_in": 5` where absent,
moves the check to its own row of the table above, adds it to the two `enforced` test lists, adds a release note, pushes the branch and
opens a pull request whose body carries the trigger. An edit it cannot make mechanically stops that proposal before anything is pushed;
`--dry-run` prints the diff instead. The pull request is reviewed and queued like any other.

### Requiring a signed note (`require`, shadow only so far)

`require` is a second per-check switch in `.st/evidence-skip-policy.json`, separate from `enforce`: `off` (the default, also when absent),
`shadow` or `reject` (anything else is an error). It is about REJECTING a change that has no trusted note, where `enforce` is about SKIPPING
work that has one. `scripts/evidence-verify --require-all --tree T --keys-from BASE --policy-from BASE --repository OWNER/NAME` judges
every check whose `require` is not `off`, each by exactly the rules of a single check, and prints one entry per check with its own reason
(`no_record`, `stale_tree`, `bad_signature`, `untrusted_signer`, `untrusted_level`, `wrong_command`, `check_failed`, ...), so a missing lane is named.
The overall `decision` is `reject` when a `reject` check is not satisfied, `would-reject` when only `shadow` checks are not, and `ok` otherwise
(also when nothing requires anything). The exit code is 1 only for `reject`; `shadow` never fails; a verifier that could not decide (invalid or missing base policy, unreadable keys, any unexpected error) exits 2 with `decision: error`, so a consumer can tell an error from a reject. Every kind rule must declare its command (the policy refuses one that does not), so a lane is always held to the argv its kind records. With no check requiring anything the answer is `ok` with an empty `lanes`; a queue mode built on this must treat an empty required set as an error, and this switch never enables skipping (`enforce` does). The policy and the keys are read from the
accepted base like everything else here. Nothing calls it yet and no check sets `require`: this adds the switch and the report, and a test
asserts that every committed check is `off`. Making any check `reject` is a separate reviewed change, and making it a required status
check is a repository ruleset change that is not made here. A pass for a tree is never evidence for the combined tree the merge queue tests.

### Lane definitions (`.st/required-lanes.json`, nothing reads it yet)

A note says a check passed; what the check RAN is decided by scripts that live in the tree under test, so an agent could change them and still hold a
valid note. `.st/required-lanes.json` (read from the accepted base, strictly validated) names, per lane, the exact `argv`, the `definition_paths`
(files or directories whose bytes decide what that argv does) and, for information, the `tree_inputs` the lane is meant to test. With
`--lanes-from BASE` the verifier (single check or `--require-all`) holds a check to its lane: every blob at or below a definition path must be
IDENTICAL in the verified tree and at BASE, otherwise `definition_changed` (the change edits its own checker, so the signed path is refused and CI
runs); and the note must carry `lane_definition_sha256`, the digest of those blobs at BASE plus the argv, otherwise `wrong_definition`. A check with
no lane entry is `no_lane`; a broken lane file or a definition path missing at the base is an error (exit 2 under `--require-all`), never a reject.
The committed file lists the two lanes `scripts/st-test` can run today, with the same argv (a test pins that) and deliberately wide definition paths
(all of `scripts/`, `.st/`, `flake.nix`, `flake.lock` for genie-freshness): a change to any script then always runs in CI. They are to be tightened
only with a read trace of the lane (a later change). No producer writes the digest yet, nothing passes `--lanes-from`, and no gate reads the file, so
this changes no behaviour; when a gate does read it, `.st/required-lanes.json` must join that gate's protected paths.

Producer half: `scripts/evidence-wrap --lanes-from BASE` (and `scripts/st-test`, which passes its base for every check the base's lane file defines)
refuses any command that is not the lane's argv, refuses a tree whose lane definition differs from BASE (nothing is signed; CI runs the check, and the
message names the paths), and records `lane_definition_sha256`, the digest the verifier recomputes from the same base. Without a lane file at the base
nothing changes. A lane's `definition_paths` are a hand-declared list: a helper script it reads but the list omits is not covered, which is what the closure
control below checks. So this alone must never let a signed note skip a queue lane; shadow reporting and the pull-request signing path are the only
uses until every lane a gate relies on has a clean trace (typescript-client has; genie-freshness has not been traced).

#### Lane closure: what the lane really touches (`scripts/evidence-lane-trace`)

A lane entry may also list `absent_paths`: paths the lane LOOKS FOR and does not find (a `.pnpmfile.cjs`, a `package.yaml`, a `fixtures/package.json`
that would change module resolution). A tracked file appearing at or below one of them changes what the lane does, so none may exist in the signed
tree (`definition_changed` names it), none may exist at the base (a lane-file error), and the list is part of the digest. The hand-written lists are
checked against a real run: `scripts/evidence-lane-trace --lane NAME --lanes-file FILE --root DIR -- COMMAND...` runs the command under `strace -f -y`
(or `--analyze RAW` reads a log) and reports `uncovered_reads` (repository files opened or stat'ed that no list covers) and
`uncovered_absence_probes` (paths looked for and not found that no list covers), exit 1 if either is non-empty. Relative paths are resolved from the
log's directory annotations and chdir/clone tracking; `unresolved_calls` counts the ones that could not be placed (a lane is not closed by a trace
that has any). Anything inside `node_modules` or `.git` is not tree content and is skipped, but a MISSING `node_modules` directory is reported.

typescript-client was traced once on 2026-10-17 at main 47fb60d96cb (`nix develop .#web -c bash -c 'pnpm install --frozen-lockfile && bash scripts/ci-typescript-client'`,
3,607 distinct paths opened, 1,922 looked for and not found). The hand-written lists had missed one file the tests import
(`apps/fractal-web/src/folders/core.mts`, now a `tree_input` and in the skip policy's declared `inputs`, which had the same gap) and 24 absence probes (now
`absent_paths`); the result after the fix is no uncovered path and no unresolved call. Limits, stated plainly: the trace covers only the paths that ONE
run took (a branch the run did not execute is not seen); it must be repeated when the lane's command, the dev shell or the test suites change; the
genie-freshness lane was traced separately (below; its input is the whole tree); and a lane whose `unresolved_calls` is not 0 is not closed. Until
every lane a gate relies on has a clean trace, a signed note still must not skip a queue lane on this alone.

genie-freshness was traced on 2026-10-17 at main 7c25ef7e87e (`nix develop .#genie -c bash scripts/ci-genie-freshness`, with `LD_PRELOAD` unset: the signer
refuses to start with it set; 8 min 55 s under strace, 1.8 GB of log; the lane's own real-strace test skips itself when already traced). Its input is the
whole tree (`genie --check` regenerates and checks every file, the licence check reads every lockfile, the tests build fixture repositories), so its lane
entry declares `tree_inputs: ["."]` and the trace tool reports nothing under the root as uncovered; the trace shows 2,353 repository paths opened, 2,513 looked
for and not found, and no call it could not place. What the whole-tree declaration does NOT cover is the environment outside the repository, which the
trace did show and no fingerprint records: the user's global git configuration (`~/.gitconfig`, `~/.config/git`, read by every git the lane starts outside
its fixtures), the bun install cache under `~/.bun`, the user's `PATH` tools (`gh`, `flock`) and `/etc/localtime`. They are not tree content and are not
bound by `flake.lock`; a host whose global git configuration differs can in principle produce a different result. That is a documented limit of the
cooperative host signature, not something this control removes.

### The `signed-tree` shadow job

`.github/workflows/signed-tree.yml` (source `signed-tree.yml.genie.ts`) runs `scripts/ci-signed-tree` on `pull_request_target` for same-repository,
non-draft pull requests and prints one line, also written to the step summary:

```
signed-tree v1 decision=ok|would-reject|reject|skipped|error mode=shadow pr=N head=SHA tree=SHA base=SHA reason=... lanes=name:state,... seconds=S
```

It asks `evidence-verify --require-all --lanes-from BASE` whether the pull request's HEAD tree has a trusted signed note for every check whose policy
says `require` (today `genie-freshness` and `typescript-client`, both `shadow`), with the reason per check (`no_record`, `stale_tree`, `wrong_definition`,
`definition_changed`, ...). It is SHADOW ONLY: the job always passes (`continue-on-error`, the script exits 0 on every path), it is not a required
check and no ruleset names it; `decision=reject` can only appear once a check's policy says `reject`, and even then it is reported, not enforced.

How a pull request cannot edit or delete the job: `pull_request_target` always runs the workflow file of the BASE branch, so the pull request's copy
of the workflow is never used, and the checkout is the base branch tip (no `ref`). The pull request's code is never checked out or executed:
`ci-signed-tree` fetches `refs/pull/N/head` as git objects (and the evidence notes) and reads them with `git rev-parse` / `git ls-tree`; the verifier,
key list, skip policy and lane file are the base checkout's. Permissions are `contents: read`, no secret is referenced, the checkout keeps no
credentials, and the event fields reach the script only as environment variables (never interpolated into a command). A test runs a pull request
that rewrites the script, the verifier, the policy and the lane file and shows that none of it runs and the base's rules decide. Forks and drafts are
`skipped`. With no check requiring a note the job is a no-op that fetches nothing. A new `pull_request_target` workflow does not run for the pull
request that adds it (the base does not have it yet), so its first runs are the next pull requests after it lands.

### SLO counters (`scripts/ci-evidence-slo`)

`scripts/ci-evidence-slo --repository OWNER/NAME [--hours 24 | --since T --until T] [--max-logs N] [--json]` reads the finished `Workspace CI` runs of an
interval (GitHub's read API only; nothing is written or started) and, for every `evidence-gate-<check>` job, the one fixed step the gate ended with. Per check
and event (`pull_request`, `merge_group`, `push`) it reports:

- `decisions`: gate jobs that completed with an outcome, the DENOMINATOR of every rate (a gate job that failed or was cancelled before printing an outcome is `gate_incomplete`, not a decision);
- `satisfied`: an existing trusted note covered the run, split into `skipped` (the gate took the skip: `enforce` true) and `shadow` (it would have), and `satisfied_by_producer`, the same count by `kind/trust_level` of the note
  read from the gate's decision line (`unread` for jobs beyond `--max-logs`, the log budget, which only bounds the producer and reason split, never the outcome counts);
- `produced_not_verified`: a note exists and is NOT usable, `rejected` (refused: bad signature, untrusted signer, failed check ...) plus `degraded` (only an unsigned degraded record), with the reasons. TARGET ZERO;
- `no_note` and `unavailable`: nothing usable was found, or the verifier or notes could not be obtained (the check ran as before).

Runs still in progress are not counted (their gate jobs may not have finished) and an API that cannot be read exits 1 instead of printing zeros. The counters measure the gate in
shadow today (`skipped` is 0 while every `enforce` is false); they are the numbers the flip decisions in the proof table read.

### Which tree a pull request run asks about

A `pull_request` run checks out GitHub's merge of the head into the CURRENT base, a new tree whenever main moved, so a note for the tree an agent
built and signed would nearly always miss. The gate therefore asks the verifier about the tree of the **pull request head commit**
(`github.event.pull_request.head.sha`; GitHub attaches the required checks to that sha) when every one of these holds, and about the merge tree
otherwise: the head repository is this repository and the pull request is not a draft; the payload head and base are full object ids and the opener,
actor and triggering actor are known; the head commit is present or fetchable; the checked-out commit is the merge whose second parent is that head
(or the head itself); the opener, the actor and the triggering actor are on `.st/evidence-authors.json` at the base and every non-merge commit of
the branch has a listed author and committer (the signing guard's own `check_authors`, loaded from the base's `ci-evidence-signing`); and no
protected file differs between the branch point and the head, the base, or the current tip of the merge's first parent. The gate, verifier, policy,
keys and author rule are all taken from the base, never from the checkout. The decision line says which: `tree_of=head|merge`, `pr_head=`,
`merge_tree=` and `head_tree=used` or `head_tree=fallback:<why>`. A `merge_group` run never uses a head tree: only a note for its own tree can skip
it, and a head-tree decision on any other event is refused (`pull_request_target` is not `pull_request`: it never uses a head tree and never skips). The line also
says `notes_fetch=ok|none|failed` (`none`: the remote has no record under the tree's own prefix, which git reports by
exiting 1 without a word; `failed`: the fetch could not reach or read the remote) and the first 16 hex digits of the base-tip policy and key-registry hashes it used (`policy_sha256`, `keys_sha256`), then the evidence
log's shadow view (`log`, `log_head`, `log_age_s`, `log_missing`, `log_fetch`; `-` until the base pins `.st/evidence-log-key`, see "The evidence log"), and input-keyed lookups use
the key of the tree the run asks about (the head's, in head mode). The signer is not checked against the author list: signers are the keys of
`.st/evidence-keys` (the trust root, read from the base), authors are GitHub accounts; the two answer different questions and a host key has no login. **Time
dependence:** a skip now depends on state outside the pull request. The same head can run when the gate looks (no note yet) and skip on a re-run once the
agent has signed and pushed the note, or the reverse if a note is later superseded by a failing record; a skip is the decision at that run, not a property of the sha. A skip on a pull request therefore means "the head tree passed on a trusted host", not
"the merge with today's main passes"; a break caused only by main moving is caught in the merge queue, which still tests the integrated tree.

**How stale a head may be (the staleness policy).** A head-tree note says the head passed; it says nothing about the main the branch has
not seen. The gate therefore uses the head tree only while the branch point is at most `HEAD_TREE_MAX_BEHIND` = 24 first-parent commits of
main behind the tip GitHub merged into (the merge's first parent). Further behind, the run asks about the merge tree, as before head trees
existed (`head_tree=fallback:the_branch_is_N_commits_behind_main...`), which in practice means the checks run; rebasing or merging main
brings the branch back inside the bound. The decision line prints `behind=N` for every head-tree candidate. 24 is about 8 hours of main at
today's 3 merges an hour; measured over 707 pull-request runs (2026-10-17 to 2026-10-19), the median branch was 2 commits behind, the 90th
percentile 11 and the 99th 22, so the bound refuses only outliers. The merge queue still tests the combined tree for every change, so the bound
limits how much a green pull request can differ from what lands, not what lands. The number lives in the gate (read from the accepted base,
a protected path), so changing it is a reviewed change like any other.

### Host keys

Our own machines are trusted to sign what they run. `scripts/evidence-keygen` makes a host's key, a scratch
repository and the signer configuration under `~/.config/st`, all owned by the running user, and prints one
line, `host/ALIAS namespaces="git" ssh-ed25519 ...`. The alias is required and is not the machine's name: the line goes into a
public file that must not carry real host names, so keep the alias-to-machine mapping somewhere private (revoking a key needs it). Adding that line to `.st/evidence-keys` is a pull request
like any other: the file is read from the accepted base (main), so a change cannot vouch for its own key, and
removing a line stops every record that key signed from counting. The principal's first part, `host`, is the
trust level; a check's policy admits it for the `wrapper` kind only (the per-kind `trust_levels` override above),
never for `observed`. Notes signed under the `host` level do not satisfy the merge queue until the key is shown to be isolated
from the code under test; they can cover pull requests only. No root, no `sekrets` grant and no pinned program is needed. The root-owned hardening of `scripts/evidence-sign` (tool
directories closed to the account) does not apply to a user-owned key, because the account is the one that runs code under test.
The script itself refuses symlinks, other owners and loose modes, creates files under `umask 077`, derives the printed line
from the private key, and refuses to run as root or with a `HOME` that is not the account's.

What this does not give, stated so nobody assumes it: anything that runs as the same user can read the key and
sign a result it never ran, and a signer that is not isolated from the code under test is only as honest as that
code. This is cooperative trust in the host. It is bounded: a record is keyed to a tree, a failing run is a
record too, the latest record decides, the merge queue still tests the integrated tree (a host-signed note never
covers it), and sampled real runs compare a note with the job. The root-installed, hash-pinned signer
(`scripts/evidence-sign` behind a `sekrets` grant) stays supported for a host that wants it.

### Agent keys

Agents push signatures (Nathan, 2026-10-19). An agent signing account has its own key and its own principal, `agent/ALIAS`, made by
`scripts/evidence-keygen --level agent --name ALIAS` under that account (the alias is invented, never a seat or host name; keep the
alias-to-account mapping private). Its line goes into `.st/evidence-keys` with a pull request, like a host key, and is read from the
accepted base, so a change can never vouch for its own key. The skip policy admits `agent` exactly where it admits `host` (the `wrapper`
kind of `genie-freshness` and `typescript-client`, the `scripts/st-test` command, `pull_request` only, never `observed` and never the merge
queue) with one addition, `"lane": true`: an agent note must be signed against the lane definition of the accepted base, so a tree that
edits the lane's own scripts never skips on an agent's note. The trust is still cooperative (the key's account runs the code it signs
for); what the separate level adds is the lane binding and revocation per account: deleting one `agent/...` line from `.st/evidence-keys`
on main stops every note that key signed from counting, for the next run after that change merges, without touching any other agent or
host. Revocation needs no ref deletion: the notes stay, they simply no longer verify.

Who can push the notes: no repository ruleset covers `refs/notes/*` (the only ruleset protects the default branch), so any identity with
write access, including an agent seat using the operator's credential, can create, move or delete `refs/notes/evidence/*`. Creating a
ref cannot forge evidence (a record counts only with a trusted signature and the right tree, check, command and inputs). Deleting one
can: if a newer failing record of a tree were deleted, an older passing record would decide again. That is the same write-access boundary
as editing a workflow, and it is why sampling stays on, the merge queue still tests the combined tree, and the gate's decision line names
the record it used.

### Running the checks locally (`scripts/st-test`)

`scripts/st-test` runs each required check that has a local command (today `genie-freshness`, `typescript-client` and
`mail-redelivery-canaries`; `--list` prints them) once, through `evidence-wrap`, on the commit you are pushing (HEAD), and the host's signer
records the exit code and output digest. The CI gate looks up the note for the pull request head's own tree (see "Which tree a pull
request run asks about"), so the tree you built and signed is the tree that counts even when main has moved since; the script tells you how
many commits behind the base the branch is. It no longer signs the merge tree (the merge of the branch into the base): the merge queue still tests
the integrated tree, and a pass of the merge tree is not made here. The command always runs in a fresh
throwaway worktree of that commit, so untracked or ignored files in your checkout cannot change the result. When that tree has a
`megarepo.lock`, its members are materialized under `repos/` first with the command CI's `megarepoApplyStep` runs (`mr apply`
from the flake's effect-utils pin; a test pins the two together): `genie-freshness` imports `repos/effect-utils`, and the dev
shells only warn when it is missing. If that fails, no check runs and nothing is signed. A failing run is
recorded too. `evidence-wrap`, `evidence-sign` and `evidence-verify` come from the accepted base. It prints one
line per check: result, seconds, and whether the verifier, from the base, now counts the tree as covered
(`untrusted_signer` until the host's key line is on main). `--push REMOTE` pushes the notes after they verify, with the identity git uses for REMOTE (or
`EVIDENCE_PUSH_TOKEN`); whether that identity may create `refs/notes/evidence/*` is a repository rule not yet confirmed for agent seats.
On a host that runs the evidence log (`evidence-log.service` installed as a systemd user unit, see "Portable evidence" in the plan), a
run with `--push` then starts that unit without waiting (`systemctl --user start --no-block evidence-log.service`, or the JSON argv in
`ST_EVIDENCE_LOG_NUDGE`): a pass counts only once the log lists it, so the agent's record is logged in seconds instead of at the next
2-minute pull. The pull itself is the unit's, with the log key's grant; st-test never signs the log. Asking is best effort and never
changes the result.
`scripts/st-test --status` runs nothing and says whether the tree is already covered (exit 0 if all, 3 if not).

A pre-push hook does this without anyone remembering. `scripts/install-git-hooks` (run by the default `nix develop` shell outside CI; it prints
one line the first time and does nothing when `GITHUB_ACTIONS` or `CI` is set) installs a stub in the repository's hooks directory, never replacing
a hook it did not write and leaving a configured `core.hooksPath` alone. **The hook and the runner are not the pushed branch's:** the stub takes
`scripts/evidence-prepush` and `scripts/st-test` from `<remote>/main` into a temporary directory and runs those copies (when the base has none it
does nothing). The checks they run are the tree's code (`nix develop`, `pnpm install`, the tests), on this machine as you with the host's signing key
within reach: that is cooperative trust (known defect 22), not something the base-pinned hook protects against. When you push the branch
you have checked out, the hook asks `st-test --status`; if every check is covered the push goes, silently. Otherwise it runs `st-test --push REMOTE`
(an interactive push waits for it up to `ST_EVIDENCE_TIME_LIMIT`, default 1200 seconds, then stops it with TERM and KILL of its whole process
group) and the push goes. A push from an agent (`ST_AGENT` set, or no terminal on stderr) runs inside a tool call with a short timeout, so there the hook
waits at most `ST_EVIDENCE_AGENT_LIMIT` seconds (default 120), prints that signing continues in the background with its log path, and lets the push go;
the notes are pushed when the run finishes, and agents should run `scripts/st-test --push origin` before pushing so the first CI run already sees them.
**The hook's environment is scrubbed (st2#3014).** git exports `GIT_DIR` (and `GIT_PREFIX`, `GIT_EXEC_PATH`, ...) to a hook, and `GIT_DIR` is set whenever the push is made from a linked worktree, which every agent seat uses. A script run with it that makes its own repository or says `git -C elsewhere ...` still acts on the CALLER's repository, because `GIT_DIR` wins over `-C` and over the working directory. The repository's own fixture tests do exactly that; run from the hook they committed on the caller's branches, rewrote the shared repository's identity and pushed. The stub, `evidence-prepush`, `st-test` and `ci-genie-freshness` (through `scripts/git-env-clean`) now drop the variables that select a repository or an identity (`GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE`, `GIT_OBJECT_DIRECTORY`, `GIT_ALTERNATE_OBJECT_DIRECTORIES`, `GIT_COMMON_DIR`, `GIT_NAMESPACE`, `GIT_PREFIX`, `GIT_QUARANTINE_PATH`, `GIT_AUTHOR_*`, `GIT_COMMITTER_*`, `GIT_PUSH_OPTION*`, ...) before they start anything; transport and credential variables (`GIT_SSH_COMMAND`, `GIT_ASKPASS`, `GIT_TERMINAL_PROMPT`, `GIT_HTTP_*`, `GIT_CONFIG_*`) stay, because the notes push needs them. The shell scrub uses only `export -p` (the non-interactive bash of a nix shell has no `compgen`, which made its first version a silent no-op there) and checks its own result: a scrub that did not work stops the caller, and makes the stub skip the hook with a message. The stub is a copy in each repository's hooks directory but runs `evidence-prepush` and `st-test` as they are on `origin/main`, so a stub installed before this change is covered as soon as it is on main; `install-git-hooks` replaces an older stub carrying its marker on the next shell entry. Tests push from a linked worktree through the real hook, also with `compgen` disabled, and require the caller's config, HEAD, status, branches and tags to be byte-identical.
It never blocks a push: a failing check is recorded and printed, a timeout is printed, and so is every other failure (an exception, a missing `python3`,
a slow fetch, a signer that `.st/evidence-keys` on main does not list, a missing signing key). `ST_SKIP_EVIDENCE=1` skips it and says so. Tags, deletes,
other branches and the base branch are pushed untouched, and the notes push inside the run does not run the hook again. A new tree always takes the slow
path (the checks run once); so does every fixup that changes the tree; a tree that already has notes is instant.

The checks run without the agent seat's identity and session variables (`ST_*`, `ST2_*`, `ST3_*`, `CLAUDE*`, `ANTHROPIC_*`; `ST_AGENT`, `ST_SKIP_EVIDENCE` and the `ST_EVIDENCE*` switches stay, by exact name: that list in `scripts/st-test` is the only exception mechanism, and `ST_AGENT` stays visible to the checks because the `nix` wrapper that needs it starts them), as on a CI runner: tests that read them fail or behave differently under a seat. Tests that bind a request to the seat by process ancestry still do, so a trustworthy full run of the Linux gate must be started detached from any seat. It runs `nix develop` and `pnpm install` from the tree under test, on the signing host as its user: code in that tree runs
there with the key within reach (cooperative trust, see Host keys). It changes no policy: a check skips only where `enforce` is true.

**The signer starts with a scrubbed loader environment.** `evidence-sign` refuses to start when the loader or the interpreter was
told to load code (`LD_PRELOAD` and the rest of its `REFUSED_ENVIRONMENT`): a preloaded library inside the signer is an injection
point, and the refusal stays. Some hosts set `LD_PRELOAD` for every process (hetz2 preloads jemalloc), which made every signed push
there report "a check failed". So the launcher scrubs instead: `evidence-wrap` resolves the signer program with the caller's PATH,
then starts it by absolute path without any `LD_*`, `DYLD_*` or `PYTHON*` variable, `GLIBC_TUNABLES`, `GCONV_PATH` or `NLSPATH`,
and with `PATH=/run/current-system/sw/bin:/usr/bin:/bin`; `st-test` starts the signer's interpreter with `-I`. The checks themselves
keep the caller's environment. Tests set these variables in the caller and require a signed note, and require the signer to still
refuse when it is started with `LD_PRELOAD`.


**Heavy checks and Linux gate stages (`scripts/ci-local`).** A check that builds and tests the Rust workspace
(`mail-redelivery-canaries` today) is heavy: tens of minutes of a shared build host. st-test runs it only when it is named
(`--check NAME`) or with `--all`, never by default, so the pre-push hook keeps signing the light checks within its wait. Its local
command is `nix develop -c bash scripts/ci-local STAGE`: the `scripts/ci-linux` stage as its CI job runs it, after building and
installing the rendered hooks as the job's test-archive setup does, in a home of its own. HOME and the XDG state, config, data
and cache directories are a scratch directory that is removed afterwards, and no st variable (`ST_*`, `ST2_*`, `ST3_*` and,
since the rename, `FRACTAL_*`: a seat's live endpoint and identity), GitHub, SSH agent, evidence or sekrets variable reaches the stage, so the daemons and harnesses its tests start cannot read or write the account's st state, harness
configuration or credentials, and no PATH entry inside the account's st state stays. Kept from the account: the cargo and
rustup homes (the dependency cache) and `XDG_RUNTIME_DIR` with the session bus, where the build admission shim queues: the
stage takes an ordinary heavy admission slot, with no priority of its own. Test binaries run outside that gate, so on a
shared host the stages' tests run 6 at a time (`CI_TEST_THREADS`; CI's runners use 8 on a machine of their own) under
`ionice -c 3` and `nice -n 10`: an unbounded run (one test per core) held the dev host's IO pressure above its alarm line. The shim is handed the dev shell's own cargo as
`CARGO_REAL` (`CI_LOCAL_CARGO`, or the first cargo on PATH outside the account's home that is not the shim): a build host's
default real cargo may source a saved dev shell environment from another day, and on 2026-10-22 one such file put an older
pty first on PATH, which made every boot canary fail on that host (the seat launch passes environment names only). The lane in `.st/required-lanes.json`
holds both the host and the agent level to its definition (`scripts`, `.st`, `.config/nextest.toml`, the flake), so a tree that
edits the scripts that decide what the stage runs never skips on a local note; the host and agent levels cover pull requests
only, and the merge queue runs the stage.

### When the gate says skip

`scripts/ci-evidence-gate` takes the verifier from the accepted base and asks it, with the policy,
about this run's tree. It prints `decision=skip` only if **all** of these hold, and otherwise
`decision=run`:

- the head repository is positively this repository (a fork or an unknown head never skips);
- the event is `pull_request` or `merge_group` (a manual run or a push never skips);
- the verifier exits 0 reporting `satisfied`, that it applied the policy, tier `full`, and a
  kind, trust level, signer and record id of the expected shape;
- the verifier's tree equals this run's own tree and its check equals this job's check;
- the record's time is not in the future;
- **no file that decides skipping differs between the accepted base and this run's tree**:
  `scripts/ci-evidence-gate`, `scripts/evidence-verify`, `.st/evidence-skip-policy.json`,
  `.st/evidence-keys`, `.github/workflows/fleet.yml` and its `.genie.ts` source. Such a change always
  runs everything, and a comparison that cannot be made counts as a change.

A verifier that is missing, slow (30 seconds), crashes, prints invalid UTF-8 or contradicts the run
leaves `decision=run`.

**The gate script comes from the accepted base, like the verifier.** The step runs
`git show "$BASE_SHA:scripts/ci-evidence-gate"`, never the checked-out copy, and if the base has no
gate or it fails in any way the step reports `unavailable` and the check runs. A change that rewrites
the gate therefore cannot make itself skip, and the protected-path rule above makes any such change
run in full. What remains is the workflow file itself, which a same-repo change can edit; that is
visible in review, is itself a protected path (so that very change runs in full), and is the documented
same-repo limit. Fork pull requests never skip.

Limits, stated plainly:
- The shipped `observed` rule has no `required_inputs` or `bind_inputs`. That is deliberate: a note is
  keyed to the exact tree, and the tree contains the workflow, so tree equality plus the pinned command
  and runner prefix is the binding. The central observer additionally records full tier only for a
  workflow and recipe matching its operator pin.
- For a pull request the tree is the merge-ref tree, so a note matches only a branch that is current
  with main.
- With no age limit (`max_age_s` null), an old success on the identical tree can skip checks that depend
  on the hosted image or on timing (`typescript-client`, `isolation-vm`). That is a choice, recorded in
  the known defects; `age_s` in the decision line is there to audit it.

### Skips are not recorded as fresh runs

A job that skipped because a note covered it still ends in success. Each gate job therefore runs one
more fixed step, `evidence skip taken`, when it decides to skip. **The observer must not record a check
whose gate job ran that step as a finished run**: otherwise one real run would keep refreshing its own
skips and the age and attempt data would lie. This is a contract with the observer (it lives in
`.github/workflows` of the Evidence observe workflow, not in this change) and nothing may consume the
skip decision until the observer honors it.

### The decision line

Every run prints exactly one line, to the job log and `$GITHUB_STEP_SUMMARY`, with no spaces inside
values and `-` for none:

```
evidence-decision v1 check=… decision=skip|run outcome=satisfied|degraded|no_note|rejected|unavailable
reason=… record=… signer=… trust_level=… kind=… tier=… observed_at=… age_s=… max_age_s=… attempts=… enforce=true|false match=tree|inputs source_inputs=…
tree=… tree_of=head|merge pr_head=… merge_tree=… head_tree=used|fallback:…|not-a-pull-request behind=… notes_fetch=ok|none|failed
policy_sha256=… keys_sha256=… head=… base=… event=… pr=… repo=… run=… attempt=… fork=true|false verifier_sha256=… gate_sha256=…
```

(one line; wrapped here). Each job also ends with exactly one of these steps running, which is how a
collector reads the outcome from the jobs API: `evidence satisfied by existing note`,
`evidence existing note is degraded`, `evidence no note`, `evidence note rejected`,
`evidence verifier unavailable`, plus `evidence skip taken` when the decision is skip. A note made for a pull request is reused by the merge queue's run
only when the tree is identical, that is, when main has not moved in between.

### Loops

No workflow has an unfiltered `push` trigger (the `push` triggers are limited to `branches: [main]` or
tags), so pushing `refs/notes/evidence/*` starts nothing; a test fails if one is added. The gate holds
`contents: read`, so it cannot push, dispatch or comment and cannot cause a rerun. The record's
`attempts` count (trusted records for the tree and check) is what a producer reads to refuse a
second production.
