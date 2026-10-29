# Gate evidence

`st evidence put`, `show`, and `verify` run offline. They require Git with SSH
signing support, OpenSSH, repository objects, and an operator-supplied allowed
signers file. They do not load daemon configuration or contact a Fractal instance.

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
[#3032](https://github.com/compoundingtech/fractal/issues/3032); existing signed
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
(`scripts/evidence-publish`, `scripts/ci-evidence-observe`, `scripts/evidence-wrap` and so `scripts/fractal-test --push`)
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
  `refs/heads/*` into `refs/evidence-gc/tips/*`; merge-queue tips are branches), plus `--head`/`--heads-from`; the tree
  of every open pull request head (`--open-heads-from FILE`, one commit a line; with `--fetch` each is fetched by id, so
  a fork's head works too) whatever its age; and the trees of main's first-parent commits within `main_days` of
  `as_of`, main's tip always.
- **live keys**: the input key each live tree projects to, for every check whose policy declares inputs, and every live tree's
  per-crate `rust-tests` keys. Those keys come from main's `scripts/rust-test-keys` over main's `.st/rust-test-inputs.json`, as the gate keys a pull request
  against its base. Until 2026-10-28 the per-crate keys were left out (the `rust-tests` policy declares no `inputs`), so a plan deleted
  the crate keys that open pull requests' `rust-tests` decisions still chose. A live tree whose crates cannot be keyed, because a
  manifest is unreadable, keeps every `rust-tests` key. In a partial clone (`--fetch`), the manifests the live trees lack are fetched
  by id in batches before keying. The summary's `crate_keys` reports how many trees were keyed, how many keys are kept, and any
  unreadable trees.
- **retention**: `.st/evidence-retention.json` at `--main` (`--retention FILE` to try another), reviewed like any
  change: `main_days` (3), `branch_days` (14), `grace_hours` (24), `soft_records` (3,000), `cap_records` (20,000).
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

**Measured 2026-10-26**, after sampling went to one in one: 22,885 refs, 11,469 records. The live set was 7,797 records:
6,452 on 196 main trees within 3 days (12,904 refs), 241 on the trees of 193 open pull requests, and 92 on 183 other
branch tips within 14 days. That is over the old cap of 5,000, so every daily run exited 3; `cap_records` is 20,000,
about 2.5 times that live set. The plan leaves 15,541 refs.

**On the remote.** A dead group goes whole or not at all. First go the names whose record keeps another name in its
own group: a note beside its tag, a migrated or duplicate first-layout name. No record leaves any group, so no verdict
changes whatever fails next. Then each group's remaining names go in one `git push --atomic`, several whole groups to a
push and at most 500 refs, a bigger group alone. Groups with a tag and groups without one never share a push. A push
the remote's protection refuses (GitHub's `GH013: Repository rule violations`) keeps its groups whole and is reported
as `kept_by_protection`, not as a failure. Any other refusal is a failure (exit 1). Either way the local refs follow
exactly what the remote deleted, and a ref already gone from the remote is forgotten, not pushed. The evidence-tags
ruleset refuses moving a tag but not deleting one, so GC deletes with its host's own git login and needs no window.
(Until 2026-10-26 it refused deletion as well, and GC turned the whole ruleset off around its deletions with an
administrator's login. Nathan's call: cheap and fast first. Nothing guards yet against someone who deletes a newer
failure to let an older pass decide; the signed evidence log reports a record that disappears without GC's line.)

**Where it runs.** `scripts/evidence-gc.timer` (daily, 03:40 UTC) starts `scripts/evidence-gc.service`, a systemd
user unit on a host we hold, never a GitHub workflow. Both are installed in `~/.config/systemd/user/`, and
`scripts/evidence-gc-timer` in `~/.local/bin/`. The wrapper keeps a bare repository under
`~/.local/state/evidence-gc/`, reads `scripts/evidence-gc` and `scripts/evidence-verify` from `origin/main` (a branch
cannot change what deletes). It reads the open pull requests' heads from the GitHub API without a login (if that
fails, nothing runs), runs GC with `--fetch --apply --open-heads-from`, keeps 30 days of summaries and plans, and
stores each summary as a new version of the Fractal document `doc/fleet/factory/evidence-gc/runs`. On a host that also runs the evidence log (`~/.local/bin/evidence-log-timer`
and `~/.config/evidence-log/signer.json` present), the wrapper passes `--before-delete`, so `evidence-log gc --publish`
writes GC's lines and publishes the log before anything is deleted; if that fails, nothing is deleted.

**Before deleting.** `--before-delete JSON_ARGV` (with `--plan-out`) runs a command, without a shell and with the plan
file's path as its last argument, after the plan is written and before any deletion or migration. If it exits non-zero
or cannot run, `--apply` changes nothing (`refused`, exit 1).

**Safety.** It fails closed: if a commit named by `--head` or `--open-heads-from` cannot be turned into a tree, no
branch tip is known at all, or the `--before-delete` command fails, `--apply` deletes nothing. The local refs follow the
remote, so a second run finds nothing to do.
Refs outside the evidence namespaces and refs that are not well-formed records are never touched. The first layout
(`refs/notes/evidence/<record>`) is read from the note: in a live tree it is migrated to
`refs/notes/evidence/<tree>/<record>` (the new name is created before the old one is removed), in a dead tree it goes
with the tree. Take a bundle first when running it by hand:
`git bundle create evidence-notes.bundle 'refs/tags/evidence/*' 'refs/notes/evidence/*' 'refs/notes/evidence-input/*'`.

### Records as tags (in progress)

No ruleset can protect `refs/notes/*` (GitHub refuses the target pattern), so anyone with write access can delete a
newer failing record and let an older pass decide again, and git keeps no trace of the deletion. The records are
therefore moving to `refs/tags/evidence/<tree>/<record>` under a tag ruleset (no update, no force push; deletion is open,
for GC); the ruleset is an extra, and the protection that does not depend on GitHub is a signed log
of every record kept on a host we hold (`doc/fleet/factory/skip-ci-plan`, "Portable evidence"). Until then the exposure is bounded: nothing rejects, a pull request skip is
undone by the merge queue's full run, and one run in five is sampled. The order, each step its own pull request:

1. The tag release workflow ignores `evidence/**` tags (done).
2. No workflow checkout fetches them (done). `fetch-depth: 0` fetches every tag, and 5,000 records as tags added 81.5 MiB and
   16 s to a full fetch (measured 2026-10-19), mostly source objects of tested commits that no branch holds. Every
   full-history checkout is now `fullHistoryCheckoutSteps` in `.github/workflows/workspace-ci.ts`: a one-commit checkout,
   then one fetch of every branch and tag with the negative refspec `^refs/tags/evidence/*`. Deepening the shallow clone
   took 21 s against 11 s for a full fetch, so the step drops the shallow marker, fetches as a fresh clone would
   (`--refetch`, git 2.36 or later) and checks connectivity: 15 s in all, against 11 s for `fetch-depth: 0` today
   (2026-10-20, from a build host). `scripts/ci-evidence-signing-test` allows only literal positive depths and runs that fetch
   against a real repository.
   A plain `git fetch` does not follow these tags either: a record's tag points at a commit no fetched branch holds.
   A fresh `git clone` does fetch them.
3. The verifier and the gate read tags first and notes as the legacy layout (done); then the producers write each
   record as a tag and a note (done, see "Tags, written and read first" above). Writing notes stops once no gate that
   still runs reads notes only.
4. GC as a pure plan of whole trees and its timer (done), the tag ruleset (done; deletion opened for GC 2026-10-26,
   which replaced GC's lock window), and the signed log.

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

A stalled pull alerts within minutes. `scripts/evidence-log-watch` (`scripts/evidence-log-watch.timer`, every 2 minutes)
fetches only the head of `refs/evidence-log/main` from the remote, not the log host's own copy, so a pull that commits but
does not publish counts as stalled too. It reads the `as_of` time from the head's subject. A head older than 360 s (three
missed pulls, inside the verifier's 600 s) is a stall, and so is a head that cannot be read for that long. The watcher
then runs `EVIDENCE_LOG_WATCH_NOTIFY`, a JSON argv with the alert line appended: once when the stall begins, hourly while
it lasts, and once when the log moves again. A notification that fails is sent again on the next run. Not every gap is a
stall. On 2026-10-28 a gate reported 21 per-crate passes as unlogged although the pulls ran on time: evidence-gc had
written its GC lines (03:42Z) for keys whose deletion was still pending, and those keys belonged to open pull requests.
GC now keeps the crate keys of live trees.

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

For a per-crate check (`rust-tests`) the view is read for every member at once. `unlogged_pass` then means some member's chosen record
is not listed, and the line's `record` and `observed_at` (its oldest member's) do not name it. The view adds `log_unlogged`, the number
of such members; `log_unlogged_crates`, the first five as `crate=record` (12 hex, joined by `+`, then `+N_more`); and
`log_unlogged_observed_at`, the oldest observed time among them, which is the one log lag would have to explain.

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
`fractal-web` lane and the three separate checks `genie-freshness`, `ci-script-tests` and `typescript-client`, which have their own notes
(the note job still waits for them, to publish those notes). It refuses, making no note, when the gate of a member
said skip (`evidence-gate-linux-gate`; a gate added later counts too unless it belongs to a separate check) or when a member reports `skipped` (only the merge-queue
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
It ignores `genie-freshness`, `ci-script-tests` and `typescript-client` the same way, and their gates' decisions.

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
The first one built this way is `checks.genie-freshness` (about 163 s cold, 0.4 s warm on a build host; a change to any file in the flake
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

Four non-required jobs, `evidence-gate-<check>` for each required check (`linux-gate`,
`typescript-client`, `genie-freshness`) and for the `fractal-web` lane, decide whether
a trusted note already covers the run's tree. They hold `contents: read` only, write nothing and dispatch
nothing, and every problem becomes `decision=run`. **Every required check now consumes its gate's decision**: `genie-freshness` and `typescript-client`
(by skipping their steps), and the Linux group (below); the `fractal-web` execution shards consume theirs the same way
(enforced for pull requests since 2026-10-23, like the others).

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
- A skip of `genie-freshness` or `ci-script-tests` skips the tests of this machinery too (`evidence-verify-test`, `genie --check` and the
  rest of `genie-freshness`; `ci-evidence-gate-test` and the other script tests in `ci-script-tests`) for that tree. That is accepted because the note is for the same tree and
  a change to those files is a protected path that always runs in full; a reader must not assume they ran.
- What is protected and what is bound otherwise. The protected paths are the gate, the verifier, both of their
  tests, the policy, the evidence keys, the workflow source (`fleet.yml.genie.ts`), the helper module it imports
  (`workspace-ci.ts`) and the generated `fleet.yml`. The signing and publishing wrapper scripts are not on that list:
  they are bound by the note itself, which carries their `wrapper_sha256`, so a changed wrapper cannot produce a note
  the policy accepts.
- A required check that is a job (`genie-freshness`, `typescript-client`) never reports `skipped`; under an
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

### Our pull requests are signed or refused, never run

Our pull requests are pre-signed (`scripts/st-test --all --push origin` before every push), so CI never runs their heavy jobs. CI is for
other people's unsigned pull requests and for merge-queue groups. On 2026-10-27, 20 of our 45 pull request heads still ran CI in full,
because a check had no valid note at the moment its gate looked. Those runs cost runner time, and the gate's quiet fallback hid them.

`.st/evidence-signed-only.json` lists the openers this applies to (`openers`, GitHub logins: the account that opened the pull request,
which GitHub authenticates). The gate reads it at the accepted BASE, like the author list, so a pull request cannot take itself off it.
For a same-repository, non-draft `pull_request` run whose opener is listed, an enforced check that the gate would run is refused
instead: `decision=refuse`, and the gate job's outputs carry `refuse=true` and a one-line `refusal`. That covers no note, a refused
note, a note that was satisfied but not usable here (say, a protected file main changed since the branch point), and a gate that could
not check the note. The refusal names the check and the reason, and ends with `sign it with scripts/st-test --all --push origin`. As
with a skip, the outputs come from a marker step (`evidence refused: not signed`) and are the last thing it writes. A note that
satisfied the verifier but cannot be used is usually a change to a file that decides skipping, covered only by a note another
signer made for the same inputs. That run needs this tree signed on our own host, which `st-test` does not redo for a check it
finds covered, so that refusal says `scripts/st-test --all --rerun --push origin`.

A host note is not hidden by a newer one from CI. CI keeps signing the same inputs for other pull requests (every run of
`typescript-client` whose inputs did not change carries the same key), and the latest record for a check decides. So on a branch that
edits a protected file, CI's later pass used to shadow the host's note, and the check ran: #4138 itself ran `typescript-client` that
way. When only our hosts' notes can cover the run (a `pull_request` run whose branch edits a protected file, compared in head-tree
mode), the gate asks the base's verifier with `--prefer-levels host`. The newest passing host record then decides, unless a failing
record is dated after it; a newer failure still decides, and every other rule applies to the record chosen.

The check jobs act on it:

- A job wired with the skip (`typescript-client`, `genie-freshness`, `ci-script-tests`) runs only its checkout. Every other step is
  skipped, and a last step, `evidence refused: sign it with scripts/st-test --all --push origin`, prints the refusal and fails the job.
- The Linux stages (`linux-test-build`, the test shards, clippy, fleet compatibility) do not start when `linux-gate`'s gate refuses.
  The `linux-gate` aggregate fails at its first step with the refusal.
- `fractal-web-execution` does not start, and `fractal-web` fails with the refusal when the change touches web inputs. With no web
  input changed, nothing runs and the check passes, as before.

Every other run is decided as before: another opener's pull request, a draft, a fork, a merge group, a push to main. So is a check
whose skipping is not on (`enforce` false, `rust-tests` today, which is part of `linux-gate`), because no note can cover it. If the
list is missing or unreadable at the base, or the gate cannot read the base, every run is decided as before, and so the check runs.
A gate job that dies before it decides also leaves the check to run: the refusal comes only from a gate that positively decided. To
fix a refused check, sign the head with `scripts/st-test --all --push origin` and push, or re-run the jobs once the notes are on the
remote. A failed check never needs a new commit.

### The Linux group

`linux-gate` is an aggregate of jobs that are not themselves required checks, so those jobs skip at the **job**
level, which allocates no runner at all: `linux-tests` and its three shards, `linux-clippy` and
`linux-fleet-compat` skip when `evidence-gate-linux-gate` decides skip. `linux-test-build` feeds the test shards, so
it skips only when **every one of them** is covered; that condition is derived from theirs in the generator, never
written twice. The VM tests (`isolation-vm`) and the mail redelivery canaries were members too, and required checks of
their own, until 2026-10-23: they now run once a day (`daily-long.yml`), and a red daily run comments on its one open issue.

`linux-gate` itself still runs always and still fails on anything but success, with one addition: it reads its gate's
decision, and a member that reports `skipped` is accepted only when that decision was exactly `true`, only
in the **positions** of the stages that are skipped at job level (the build, the test shards, clippy and fleet
compatibility, derived in the generator from the very list that skips them), never for a failure or a
cancellation, and the stage counts are still enforced. `upgrade-impact` is never accepted as skipped, so a condition that skipped them for any other reason cannot be waved through just because a
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

`enforce: true` means a covering signed note lets CI skip this job; it never fails or blocks a pull request that has no
note, that pull request just runs the job and CI signs it.

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

`linux-gate` declares its inputs too (enforced for pull requests since 2026-10-23, `sample_one_in` 5). The list is the Rust test stage's
(`doc/fleet/factory/nix-evidence/rust-test-declared-inputs`, a textual scan of what the Rust sources name) plus what the other
members of the aggregate read: the whole of `.github` (workflows, the clippy and fleet-compat baselines) and `scripts` (the stage
scripts and their tests), `fixtures` and `templates`. A pull request that changes none of those paths (documentation outside
`docs/st2`, `docs/st3` and `docs/vrs`, `apps`, `packages`, `release-notes`, `AGENTS.md`, `README*`) has the same key as a tree
on which the whole gate passed. `slo` (`include_str!` of `slo/targets.toml` into the Fractal crate), `evals` (the example and graph tests read `evals/fractal`),
`claude-channel` and `plugins` (the plugin files `include_bytes!`d into fractal-drivers) and `docs/st2` (`tests/invariants.rs` includes
`docs/st2/invariants.md`) are declared because Rust sources open them; a committed-policy test fails when any literal `include_str!`,
`include_bytes!` or `include!` path in the Rust sources resolves outside the declared inputs. `release-notes` is deliberately not declared: the Rust sources only name `release-notes` as an
artifact name, and `upgrade-impact`, which reads the fragments, always runs. The one file under `apps` the Rust tests read at run time is declared by name
(`apps/fractal-web/src/terminal/assets/JetBrainsMonoNerdFontMono-Regular.ttf`, opened by a test in `crates/fractal-daemon/src/attach_image.rs`). The scan misses paths a test builds at run time, so
the first runs are shadow: a miss shows up as a satisfied note beside a failing real job, and `scripts` and `.github` are declared
whole until a per-file list is reviewed against a trace of what each stage opens.

`fractal-web` declares its inputs too (enforced for pull requests since 2026-10-23, `sample_one_in` 5). The list is every path the lane's detector
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

**`ci-script-tests`: the CI scripts' self-tests, keyed on what they read.** `genie-freshness` used to run every self-test of the CI machinery with the
whole tree as its key, so every pull request paid for them (about 7 minutes). Measured with `strace` (2026-10-23), 44 of the 47 tests read only
`scripts/`, `.github/`, `.st/`, `nix/` and about fifteen root and config files; they are most of the test time (`ci-evidence-gate-test` alone is
two minutes). They now are the check `ci-script-tests` (`scripts/ci-script-tests`), whose skip-policy entry declares exactly those paths as `inputs`:
a pull request that touches none of them reuses an earlier signature. The check runs the tests in a **sparse copy of the tree holding only the declared
inputs** (`scripts/ci-sparse-tree`, a one-commit git repository; the megarepo members under `repos/` are linked in, pinned by `megarepo.lock`), so a
test that reads anything else fails instead of passing on a stale signature: the list cannot drift from what the tests read, and adding a read outside it
is a reviewed change to the list (policy, `.st/required-lanes.json`, and the list in `scripts/evidence-verify-test`). The four tests that read
`crates/`, `apps/` or the whole tree (`evidence-verify-test`, `check-ci-test-paths`, `ci-fractal-web-test`, `rust-test-keys-test`) stay in `genie-freshness` with the lints, the
licenses and `genie --check`; its key is still the whole tree and its run is about a minute. Both scripts run their tests in parallel
(`scripts/ci-parallel-tests`, `GENIE_FRESHNESS_JOBS`, default 6). The new check has a gate job, a workflow job, a lane, a `scripts/fractal-test` entry and the
same levels as `genie-freshness`; it is **not a required context of its own** (a head from before the check existed has no run of it, and a required context would block every such head): the `linux-gate` job, which every head reports and which always waits for the long stages, needs it and fails the gate when it failed or was cancelled (`skipped` counts only when its own gate decided to skip), so new heads enforce it and old heads are unaffected.

`isolation-vm` and `mail-redelivery-canaries` had entries of their own with the same list until 2026-10-23, when they moved to the daily
run; they have no policy entry now, so nothing on a pull request or in the queue waits on them.

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
- `genie-freshness`: `["bash", "scripts/ci-genie-freshness"]`; `ci-script-tests`: `["bash", "scripts/ci-script-tests"]`; `typescript-client`: `["bash", "scripts/ci-typescript-client"]`;
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
host-signed local note (its command is the `nix develop ...` line `scripts/fractal-test` runs) can satisfy a check whose CI step wrapper records a
different command and extra inputs, while each signer is still held to exactly its own rule. The verifier reports the `events` that apply
and the gate skips only on those events: a note whose signer level lists `pull_request` only never covers a `merge_group` run. Today the
`host` level is admitted for the `wrapper` kind of `genie-freshness`, `ci-script-tests` and `typescript-client`, for pull requests only.
A level's rule may also say `"lane": true` (default false): that level's notes are then held to the lane of the check in
`.st/required-lanes.json` at the same accepted base as the policy, exactly as `--lanes-from` holds any note: the tree must not change
the lane's definition paths (`definition_changed`) and the note must carry that base's `lane_definition_sha256` (`wrong_definition`); a base
without a lane for the check refuses such a note (`no_lane`). With `--lane-base COMMIT` the definition is the one at that commit instead
(the lane file must be the same at both; see "Which lane definition a pull request is held to"). Other levels are unaffected. The `agent`
level (below) uses it.

A check may also name `events` itself (optional; the same two values). It limits the runs ANY note for that check may skip, whoever signed it,
and a signer level's own `events` can only narrow it further (the verifier reports the intersection, which may be empty: then nothing skips).
Absent means every event the gate allows. This is how a check is enforced for pull requests while the merge queue keeps testing the combined
tree: `"enforce": true, "events": ["pull_request"]`.

The `queue` level is for a merge-queue signer: a key held in an account of its own, away from every agent seat, that fetches main and the
pull request heads itself and signs the merged tree the queue will test. Every check the `host` level signs admits it for the `wrapper`
kind with the host's command and inputs, `"events": ["merge_group"]` and `"lane": true`, so a queue note never covers a pull request and a
tree that edits its own lane never skips on one. It is inert until a check's own `events` gains `merge_group`: today every check lists
`pull_request` only, so the intersection is empty and a queue note skips nothing. That flip is made per check, on its own measured trigger,
and `.st/evidence-keys` names the key (a `queue/NAME` line) only once it exists; deleting that line revokes it.

#### Input values (`input_order`, `shard_inputs`)

`required_inputs` checks only that an input is present. Two optional keys on a kind's rule check values. They apply whatever the signer level.

**`input_order`** maps an input to its values, strongest first, for example `{"kit_inputs": ["true", "false"]}`. A note covers a run only
when its value ranks at or above the run's own value.
- The run's value comes from the gate: `evidence-verify --run-input KEY=VALUE`.
- A run that did not say its value is never covered (`no_run_input`).
- A note whose value is missing, unknown or weaker is refused (`wrong_input`).

It is for inputs that depend on the run, not the tree. fractal-web's `kit_inputs` says whether the diff from the run's base touched kit
paths; with `false` the lane skips every kit play. A note made without kit plays may cover a run that would skip them too, never one that
would run them.

**`shard_inputs`** names a sharded job's inputs: `count` (1 to 16) and `results`.
- `results` must read `0=success,1=success,...` for every shard.
- `pull_request_count`, when given, is the only count a note from a `pull_request` run may carry.
- `index`, when given, names the input only one shard's own note carries (fractal-web: `kit_shard`). Such a note is a part of a run, not
  its verdict. When the latest record is chosen, a passing part is passed over, and a failing part still decides (`check_failed`). Passing
  parts alone are `no_record`. The merge queue signs every shard's note beside the whole run's, under the same tree and input key, and
  the newest of them was often a shard's: refused as `missing_input`, it hid the whole run's note from pull requests that shared the key.

The fractal-web note's per-shard results come from the matrix's single result, which GitHub reports as success only when every shard
succeeded. On pull requests the matrix has one shard. `ci-evidence-signing-test` holds every shard count in the workflow to the
matrix's own condition and length.

fractal-web's policy checks both. The run's own `kit_inputs` reaches the verifier through the gate:
- `evidence-gate-fractal-web` needs `fractal-web-changes` and sets `RUN_INPUTS` to `{"kit_inputs": "<its kit-inputs output>"}`.
- The gate passes each value as `--run-input` once the base's verifier takes that flag.
- A malformed `RUN_INPUTS` is dropped, so no note covers the run.

`scripts/evidence-flip-ready` does not propose fractal-web's flip until both keys are in the policy.

`app_test_set=related` (pull requests run the app tests reached from their own diff) has no value rule: every pull request's base is a
main commit that passed the full set, and the merge queue runs everything. It must be revisited before the merge queue stops running what a
note covers.

### Which checks skip today

| Check | `enforce` | `events` | `sample_one_in` | Since |
| --- | --- | --- | --- | --- |
| `typescript-client` | true | `pull_request` | 1 | this change |
| `fractal-web` (whole-check note; one shard's note never counts) | true | `pull_request` | 1 | 2026-10-23 |
| `linux-gate` | true | `pull_request` | 1 | 2026-10-23 |
| `genie-freshness` | true | `pull_request` | 1 | 2026-10-23 |
| `ci-script-tests` | true | `pull_request` | 1 | 2026-10-23 |

`typescript-client` was turned on after 48 hours of shadow decisions (2026-10-17 to 2026-10-19, 585 pull-request runs): 378 satisfied by a
trusted note, 254 of which would have skipped (the rest were sampled or touched a protected file), and not one satisfied note sat beside a failing
real job; the published notes' shadow verification (`evidence-shadow v1`) was `ok` for every lane in every merge-group run since it landed. The
sampled one in five kept comparing the note with a real run until 2026-10-26, when sampling was turned off for every check (Nathan: our pull
requests skip CI entirely; the merge queue's full run is the comparison). Each further check is its own reviewed change to this table, the policy and the
`enforced` lists in `ci-evidence-gate-test` and `evidence-verify-test`.

### Which checks our pull requests skipped (`scripts/evidence-skip-report`)

`scripts/evidence-skip-report [--hours 24] [--actor LOGIN]... [--receipts FILE]... [--markdown]` counts, for the pull request runs in the
window (started by the given accounts), each check as `skipped` or as run, and why it ran: `lane_edit` (the pull request changes a file
of the check's lane, so a local run could not sign it), `merge_tree` (the gate asked about the merge tree, `tree_of=merge`, because a
protected file changed since the branch point, an author is not listed or the branch is too far behind, and no note covered it), `not_signed` (no note covered the tree although the lane is untouched: pushed
unsigned), `failed` (the note recorded a failure), `protected` (a file that decides skipping changed), `sampled` (the one-in-five canary),
`shadow` (the skip is not turned on) or `other`. It lists the not-signed pull requests with their branch and, from pre-push receipts
(JSON lines with `branch` and `agent`), the seat that pushed them. Read only; it reuses `scripts/evidence-flip-ready`'s GitHub reads.

### When a check is ready to flip (`scripts/evidence-flip-ready`)

The trigger for turning a further check on is read from the gate's own decision lines, once the base pins the evidence log key
(the log view, "The evidence log"). `scripts/evidence-flip-ready` reads the `evidence-decision v1` line of every `evidence-gate-<check>`
job of the `Workspace CI` pull request runs created in the last 24 hours (job logs through `gh api`, cached by job id) and, per check in
the policy at origin/main, says `ready` when all of these hold, and otherwise lists `why_not`:

- 24 hours of fresh published log: no line in the window says `log=stale`, `absent`, `unsigned` or `-` (the base did not pin the
  key yet), and the first line with a log value after the last `-` (`log_window_start`) is at least 24 hours old;
- at least 30 lines with `outcome=satisfied` and `log=ok`, except for a check with a lowered trigger (below);
- no `log=missing`;
- no unexplained `log=unlogged_pass`: one is explained when the published log (`refs/evidence-log/main`, fetched shallow into a
  bare repository under the cache) lists the record in its newest head at most 240 s (two log pulls) after the run was created, or
  when the record was observed after the run was created. An aggregate's records are the members `log_unlogged_crates` names.
  The run's creation is the clock, not the record's `observed_at`. On 2026-10-28 CI pushed its records at the end of its runs, 5-7
  minutes after it observed them (`ci-script-tests` at 08:33Z was 366 s past `observed_at`, but logged 54 s after its run). Two gates
  read the head one pull before the record they chose was published (00:13Z). Without a readable published log, or for an aggregate
  whose list ends in `+N_more`, the old clock applies: observed at most 240 s before the run was created. The time is then the
  aggregate's `log_unlogged_observed_at` when the line has one, else its `observed_at`; `None`, like `-`, names no time;
- no hidden red: no satisfied line in a run (and attempt) whose real job, the job named exactly the check, concluded `failure`. A real
  job the gate skipped is evidence neither way. For the aggregate `linux-gate` the red is hidden only when a member that a skip skips
  (the jobs whose `if` reads `needs.evidence-gate-linux-gate.outputs.skip`: `linux-test-build`, `linux-tests` and its shards,
  `linux-clippy`, `linux-fleet-compat`) failed or was cancelled; a red from a member that always runs (`upgrade-impact`) would show
  under enforce too, and is reported as `visible_red`, which does not block. A check with no job of its own (the per-crate
  `rust-tests`) is judged by the jobs its skip skips, those whose `if` reads `needs.evidence-gate-rust-tests.outputs.skip`
  (`linux-test-build`, `linux-tests` and its shards; not the `linux-gate` verdict, which reads it in a step only): its real result
  is `failure` when one of them failed or was cancelled, and that red is hidden. While no job skips on it, it is not ready.

**Lowered trigger for `rust-tests`** (Nathan via Factory, 2026-10-26, mission fleet/factory/no-whole-tree). The 30-line count is replaced by
**5 agreeing** lines and **0 disagreeing** ones. Only lines whose note was signed at the `host` level count (`trust_level=host`: our
seats' `st-test`); CI's `fleet` per-crate notes stay observe-only until the trusted-report hardening.
- An agreeing line is satisfied, has `log=ok`, and its real result (the jobs the per-crate skip would skip) was success.
- A disagreeing line is satisfied, and its real result was failure. The report lists those runs as `disagreeing`.

Every other condition holds as written above, among them the 24-hour log window, no unexplained `unlogged_pass` and no hidden red at
any level. The flip `--propose` makes for it also narrows the `wrapper` kind's `trust_levels` to `host`, `agent` and `queue`.
Pull requests skip on host or agent notes, so another owner's agents skip on their own signed notes exactly as ours do. Queue notes
apply to `merge_group` runs only, which are never skipped. No CI (`fleet`) note is trusted.

A check can also carry preconditions on its own policy entry at origin/main, whatever its decision lines say. `linux-gate` has one:
its clippy stage ratchets against a base, so it is not ready until the policy requires `clippy_base_sha` of both its `host` and `agent`
notes (each note then names the base it was measured against, not whatever a local `origin/main` happened to be).

It prints JSON (`log_clock`: `published log`, or `observed_at (why)` when the published log could not be read; `checks.<check>`: `enforce`, `lines`, `satisfied_log_ok`, `log_missing`, `unlogged_pass_unexplained`, `hidden_red`
and `visible_red` run ids, `log_window_start`, `ready`, `why_not`) and exits 0, or 2 when GitHub could not be read (it pages the pull request runs newest first back to the window start, without GitHub's `created=` filter, which answers an empty listing for about one request in four; a page that is empty or short of what the listing's total says it holds is asked again up to four times, then the run counts as unreadable, not as zero lines; so does a window of more than the listing's 1,000 runs). Enforced checks are reported too. `presign` counts, per account that started the runs, the distinct pull request heads in the window: `presigned` (some check had a host or agent note), `ci_note` (only an earlier CI run's note), `no_note`, and `presigned_share`.
`--propose` opens the flip for each ready check that is not enforced and has no open pull request from `evidence-flip/<check>`: in a
temporary worktree of origin/main it sets `"enforce": true` and `"sample_one_in": 1` (no run sampled; it replaces any other value), adds `"events": ["pull_request"]` where absent,
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

### Lane definitions (`.st/required-lanes.json`)

A note says a check passed; what the check RAN is decided by scripts that live in the tree under test, so an agent could change them and still hold a
valid note. `.st/required-lanes.json` (read from the accepted base, strictly validated) names, per lane, the exact `argv`, the `definition_paths`
(files or directories whose bytes decide what that argv does) and, for information, the `tree_inputs` the lane is meant to test. With
`--lanes-from BASE` the verifier (single check or `--require-all`) holds a check to its lane: every blob at or below a definition path must be
IDENTICAL in the verified tree and at BASE, otherwise `definition_changed` (the change edits its own checker, so the signed path is refused and CI
runs); and the note must carry `lane_definition_sha256`, the digest of those blobs at BASE plus the argv, otherwise `wrong_definition`. A check with
no lane entry is `no_lane`; a broken lane file or a definition path missing at the base is an error (exit 2 under `--require-all`), never a reject.
The committed file lists every lane `scripts/fractal-test` can run, with the same argv (a test pins that). The file is in use: `scripts/fractal-test` and `scripts/evidence-wrap` write the digest, the
verifier holds every lane-bound signer level (`"lane": true`) to it, and the gate reads it from the base, so `.st/required-lanes.json` is one of
the gate's protected paths. A pull request's head is held to the definition at its branch point (see "Which lane definition a pull request is held
to").

Producer half: `scripts/evidence-wrap --lanes-from BASE` (and `scripts/fractal-test`, which passes its base for every check the base's lane file defines)
refuses any command that is not the lane's argv, refuses a tree whose lane definition differs from BASE (nothing is signed; CI runs the check, and the
message names the paths), and records `lane_definition_sha256`, the digest the verifier recomputes from the same base. Without a lane file at the base
nothing changes. So this alone must never let a signed note skip a queue lane; shadow reporting and the pull-request signing path are the only
uses until every lane a gate relies on has a clean trace (typescript-client has; genie-freshness has not been traced).

#### Definition paths are what the lane executes, plus its verdict data (`scripts/lane-definition-closure`)

A lane's definition is what defines its verdict: the files it executes, plus the data that sets the bar (baselines, splits, budgets,
allowlists), declared as verdict data. A file a test merely checks for consistency is part of the tree under test, and the signed run tested
it. A file the lane executes that `definition_paths` leaves out would let a tree change the check and still be signed; a file listed that does
not define the verdict makes every change to it an unsignable lane edit. So the list is derived, not written by hand.

`scripts/lane-definition-closure` starts from each lane's `argv` and statically follows what the command graph runs, sources, imports or names,
transitively: the shell words, Python string constants and imports, JavaScript literals and relative imports that name a tracked file
(`bash scripts/x`, `. "$(dirname "$0")/x"`, `$root/scripts/x`, `SourceFileLoader(..., 'scripts/x')`, `open('.github/clippy-baseline.json')`, an
f-string such as `.github/{name}-compat-baseline.json` as a glob). It sorts each file it reaches into one of two kinds:

- **Executed**: the lanes' own code and its data in their code directories (`scripts/`; `nix/` when Nix code imports it; a lane's runners inside a
  package, such as fractal-web's `apps/fractal-web/scripts/ci` and `packages/fractal-ui/scripts`), the part of `flake.nix` that defines the dev
  shell the lane enters and what that Nix code reads, the package.json whose script a `pnpm` call runs, and the files a tool the lane runs reads
  by convention (`cargo` `.cargo/config.toml`, `nextest` `.config/nextest.toml`, `pnpm` the root manifest, lockfile and workspace file, `buck2`
  its cell, root, targets and toolchains). A tool counts when a shell word or a whole string constant names it, not a word inside an assert's
  text. Only executed code is followed, and a mention counts like a call, so it errs wide.
- **Read**: any other file the code names (a workflow, a doc, a fixture outside the code directories). This is the tree under test unless the
  lane declares it as verdict data (the `VERDICT_DATA` table, one reason per file: linux-gate's clippy baseline and splits, its compat
  baselines and the daily-only filterset; rust-tests' filterset and messaging compat baseline; fractal-web's license inventory and the web
  dev shell's input spec). A file that executed code reads as configuration of the environment it builds (`jq --slurpfile`, `--argfile`,
  `--rawfile` or `-f`, `nix eval|build --file`) must be executed or declared: `--check` fails on one that is neither, for every lane but
  the two kept by hand, whose script tests read the other lanes' configuration to test the scripts that read it.

The walk ends at the tree under test (the Rust workspace and the pnpm workspace packages, the lanes' `tree_inputs`; a lint that reads a
crate's build.rs reads its subject, not its definition). A lane's runners inside a package are followed only for that lane: for genie-freshness,
which imports `kit-play-shards.mjs` to test it, they are the tree under test. A lane whose tests run in a sparse copy (ci-script-tests) is walked
only inside that copy's inputs. A directory the code lists (iterdir, glob, `find`, `dir/*.json`) counts whole inside the code directories and is
reported, not added, elsewhere (a lint over `crates/`, every release note). The docstring lists the rules and the few explicit tables (an edge
no reference shows, such as rust-tests' JUnit report coming from the tests stage; `scripts/install-git-hooks`, whose pre-push stub runs the
base's scripts at push time, not in the lane).

`--why` prints each lane's executed paths and verdict data with the file that names each one, and what it only reads; `--write` sets each lane's
`definition_paths` to executed plus verdict data (review the diff: it changes each lane's `lane_definition_sha256`, so a note signed under the
old list is `wrong_definition` once the change is the base, and the tree is signed again); `--check` exits 1 naming any executed path or
declared verdict-data file that a lane's list does not cover. That is the rule it enforces: executed is a subset of definition for every lane,
and verdict data is a subset of definition where declared. `scripts/lane-definition-closure-test` (in ci-script-tests) tests each form on
fixtures and holds the committed lanes to it in that check's sparse copy, and `scripts/ci-genie-freshness` runs `--check` over the whole tree,
so a change that makes a lane execute a new file fails until the lane file lists it. A superset is allowed (a parent directory covers the files
below it): `ci-script-tests` and `genie-freshness` are kept by hand (`KEPT`): all of `scripts/` and `.st/`, the flake, and the few files outside
them that the walk finds they execute. Narrowed on 2026-10-25 from all of `scripts/` and `.st/` for the other four lanes: a change to a script
now is a lane edit only for the lanes that run it, and a change to a workflow or a doc that a test checks is not a lane edit at all. The gate and
the verifier read the policy, keys and lanes from the base, never the tree. Limits: the walk is static, so a path built at run time from names it
cannot see needs a table entry, and it cannot tell a test that runs a script from one that reads it as text (it errs wide).

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
protected file differs between the branch point and the base, or the current tip of the merge's first parent (main moved one since the
branch: that is about which tree, so it still falls back), except the three the gate and verifier only ever read at the base when they judge
a note (`BASE_READ` in the gate: `.st/evidence-keys`, `.st/evidence-authors.json` and `.st/evidence-log-key`). The head tree's copy of
those decides nothing, so a key rotation or an author-list change on main no longer voids head signatures made before it. A note the
change distrusts (a dropped key, say) is still refused, by the lists as they are at the base. Nor does main moving a protected file that the
run's check does not use, when the base declares that check's CI inputs (see "A check's CI inputs" below): a change to the gate or the
verifier on main keeps the head tree for linux-gate, say, and the decision line says `ci_inputs=kept`. The protected files the branch ITSELF changes no longer make it fall back: they are
the run's protected changes, and a protected change skips only on a `pull_request` run whose note is a host level's (`OWN_LEVELS` in the gate),
since 2026-10-26 (Nathan: our pull requests skip CI entirely). That is safe because the run is judged by the gate and verifier the workflow
fetches from `BASE_SHA`, never the pull request's copies, and the merge queue runs every check in full, so a change to the machinery is tested
before it lands. Any other note (a fleet note, an agent note) and every `merge_group` run still run a protected change. The gate, verifier, policy,
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

### A check's CI inputs

A lane entry in `.st/required-lanes.json` may declare `ci_inputs`: the protected CI files the check uses, in the forms a skip policy's
`inputs` take plus three more:

- `PATH` and `!PATH`: a file or directory, and one left out of a directory declared beside it;
- `PATH::jobs.ID`: one job of a generated workflow. That is the job's block, the block of every job it `needs`, transitively, and the
  workflow's top-level blocks (`on`, `env`, `permissions`, `concurrency`, ...), so a top-level `env` change moves every job's key;
- `PATH::KEY.KEY`: one entry of a JSON file, such as `.st/required-lanes.json::lanes.linux-gate` or
  `.st/evidence-skip-policy.json::checks.linux-gate`.

When main moved protected files since a pull request's branch point, the gate (`branch_point_verdict`, with the run's check) projects the
check's `ci_inputs` (as the base declares them, read with the base's verifier) at the branch point, the base and the current tip. If all
three keys are equal, main moved nothing this check uses, and the head tree is still asked about (`ci_inputs=kept`). Otherwise it falls
back as before (`ci_inputs=changed`). Every entry must also name something at all three revisions: an entry that names nothing (a
job main renamed, a typo) would project to nothing everywhere and look unchanged however the job it meant changes, so it counts as changed. A lane with no `ci_inputs`, or a base that cannot say, keeps the earlier rule: any protected change on
main falls back. `scripts/fractal-test` asks the same question for each check it signs before it decides to merge main in.

ci-script-tests and genie-freshness declare none, because their own tests read the gate, the verifier, the policy and the lanes. A change
to those therefore voids their signatures and no others: linux-gate, typescript-client, fractal-web and rust-tests keep theirs. fractal-web's
scripts read all of `fleet.yml` and `workspace-ci.ts`, so it declares both whole.

**Keeping the declarations honest.** `scripts/lane-definition-closure --check`, which genie-freshness runs on the whole tree, fails a lane
that declares `ci_inputs` and executes or reads a protected file they leave out. A selector covers only its part, so a lane that reads the
whole file must declare it whole; an excluded path is left out. It also fails an entry that names nothing in the tree (a file, job or JSON entry that isn't there),
read with the verifier's own selector. Files only ever read at the base (`BASE_READ`) don't count. Notes carry
nothing new: a head-tree note is about the head tree whichever way the gate decides, so older notes need no special case.

`.st/rust-test-inputs.json` is not protected. It is data that changes with ordinary code, and the verifier reads it only at the base when it
computes the per-crate keys.

### Which lane definition a pull request is held to

A lane's definition paths are wide (all of `scripts/`, `.st/`, `flake.nix`, `flake.lock` and the nextest and clippy settings for the Rust
lanes), and main changes one of them in more than a third of its merges (197 of 502 in the week to 2026-10-23). Compared with main's tip, a
branch that is merely behind such a merge reads as editing its lane, so its lane-bound note never counted, and a note signed against the tip
stopped counting as soon as main moved again. A lane edit is therefore judged by the pull request's OWN change (Nathan, 2026-10-23):
when the gate uses the head tree, it passes the branch point (`git merge-base BASE HEAD`) to the verifier as
`--lane-base`, and the head is held to the lane's definition at that commit. The definition paths must be byte-identical in the head and at
the branch point (`definition_changed` otherwise: the pull request edits its lane, CI runs it) and the note must carry the branch point's
`lane_definition_sha256` (`wrong_definition` otherwise). A level whose by_level rule says `"lane": "tree"` (the host level, since 2026-10-26)
may edit its lane: its note must then carry the digest of the lane as the SIGNED tree defines it (the base's argv and paths, the tree's bytes;
`tree_sha256` beside `sha256` in `lane_definition`), so it covers that edited definition and no other. `evidence-wrap` records that digest.
The verifier refuses a lane base that is not an ancestor of the base, and one
whose entry for this lane in `.st/required-lanes.json` differs from the base's (`definition_changed`), so the argv and paths are the base's
either way. Another lane's entry changing on main is not this lane's, and with "A check's CI inputs" the gate keeps the head tree then. What this admits, stated plainly: a note made
with an older lane definition that main had accepted, for a pull request that changed none of it; the merge queue still tests the
integrated tree with today's definition, `pull_request` only, sampling unchanged. A merge-tree run (a fallback, or a branch that holds the
base) is held to the base as before. To keep the head tree the gate does not fetch the base again at depth 1 in head mode (that would cut the
history the ancestry check reads).

`scripts/fractal-test` signs and reports the same way: it takes the branch point of the commit it signs (the base itself when the branch holds the
base's tip, or when main changed the lane entry of a check it signs since the branch left it), judges a lane edit against it, passes it to `evidence-wrap
--lanes-from` (so the note carries its digest) and asks the verifier with `--lane-base`, in a run and under `--status`.

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
kind of `genie-freshness`, `ci-script-tests` and `typescript-client`, the `scripts/fractal-test` command, `pull_request` only, never `observed` and never the merge
queue) with one addition, `"lane": true`: an agent note must be signed against the lane definition of the accepted base (of the pull
request's branch point, for a head tree: see "Which lane definition a pull request is held to"), so a tree that edits the lane's own scripts
never skips on an agent's note. The trust is still cooperative (the key's account runs the code it signs
for); what the separate level adds is the lane binding and revocation per account: deleting one `agent/...` line from `.st/evidence-keys`
on main stops every note that key signed from counting, for the next run after that change merges, without touching any other agent or
host. Revocation needs no ref deletion: the notes stay, they simply no longer verify.

Who can push the notes: no repository ruleset covers `refs/notes/*` (the only ruleset protects the default branch), so any identity with
write access, including an agent seat using the operator's credential, can create, move or delete `refs/notes/evidence/*`. Creating a
ref cannot forge evidence (a record counts only with a trusted signature and the right tree, check, command and inputs). Deleting one
can: if a newer failing record of a tree were deleted, an older passing record would decide again. That is the same write-access boundary
as editing a workflow, and it is why sampling stays on, the merge queue still tests the combined tree, and the gate's decision line names
the record it used.

### Running the checks locally (`scripts/fractal-test`)

The human-facing runner is `scripts/fractal-test`; its script tests remain `scripts/fractal-test-test`.
During the bridge (`arn:lmig:smalltalk:2026-10-27-st-test-shim`), `scripts/fractal-test` is an executable Python
exec shim to the adjacent `scripts/st-test`, which retains the complete, self-contained implementation.
Base-pinned readers always copy and run `scripts/st-test` with their existing Python invocation: supported
older heads copy only that file from the new base, so it must work without an adjacent shim or implementation file.
Internal Python module loaders also load `scripts/st-test`, not the exec shim. Human commands and diagnostics
remain `scripts/fractal-test`; host cache, config, evidence, report and run-ledger paths below, and the `ST_*`
environment settings, keep their existing names.

Contract only after every supported base includes this PR: first move the implementation to `scripts/fractal-test`,
then give base readers a `scripts/st-test` fallback for pre-contraction bases, and finally delete `scripts/st-test`
and this bridge marker. Do not prefer or copy the shim while copy-only bases remain supported.

`scripts/fractal-test` runs the whole per-PR suite: each required check that has a local command (today `typescript-client`, `genie-freshness`, `ci-script-tests`
and `linux-gate`, in that order; `--list` prints them; `--all` is accepted and means the same) once, through `evidence-wrap`, on the commit you are pushing (HEAD), and the host's signer
records the exit code and output digest. The checks that do not build (`typescript-client`, `genie-freshness`, `ci-script-tests`) run alongside
`linux-gate`, one after another in a lane of their own, as CI runs them on runners of their own: their output is kept and printed after
linux-gate's, never interleaved, and the summary lists every check as before (`ST_EVIDENCE_SERIAL=1` runs them one after another). The CI gate looks up the note for the pull request head's own tree (see "Which tree a pull
request run asks about"), so the tree you built and signed is the tree that counts even when main has moved since; the script tells you how
many commits behind the base the branch is. It no longer signs the merge tree (the merge of the branch into the base): the merge queue still tests
the integrated tree, and a pass of the merge tree is not made here. The command always runs in a clean
worktree of that commit, so untracked or ignored files in your checkout cannot change the result. The worktree is one of a small pool
of persistent ones per ORIGIN (`~/.cache/st-test/pools/<hash of the normalised origin URL>/tree-N`; a clone without an origin has a pool of its own, keyed by its git common dir), each held by an flock while a run uses it
(a run waits for a free one and never shares one with a concurrent run); before each run the tree is reset to the commit and stripped of
untracked and ignored files except the inputs that cost minutes to fetch again (the megarepo members under `repos/`, every `node_modules/`
and `target/compat-baselines/`), and its tracked files must equal the commit (a tree that is still dirty is a refusal). The path is the same every run, so cargo's
fingerprints and sccache's absolute-path keys stay valid and a second run is warm. `ST_TEST_TREES` sets the pool size (default: the
host's heavy slots plus one, counted as the cargo shim counts them: `CARGO_HEAVY_SLOTS`, else `~/.config/cargo-heavy-slots`, else 1; `off`
runs in a fresh throwaway worktree as before) and `ST_TEST_POOL` its directory. When every tree is in use, runs wait in the build host's order, not by a race to the next free tree. Each run takes a ticket in the pool's `queue/`, ranked in `~/.config/cargo-priority` (or `CARGO_PRIORITY_FILE`) by the cargo shim's caller (`ST_AGENT`, else `CARGO_PRIORITY_CALLER`), aged like the shim's tickets (one rank per 120 s, never below 5), then by arrival. Only the best live ticket tries the trees, so the CPU P0 builder at rank 3 gets the next free tree ahead of earlier rank-50 runs. The pool size is read again on every pass while a run waits, so a tree added when the host's heavy slots grow is taken at once. A ticket is dropped when its process is gone, or when its pid now names a process that started after the ticket was taken (a reused pid keeps no place). Each run also sets `ADMISSION_RUN=st-test-<pid>@<start-ns>` for its checks, so the host's finish-first admission order (`FINISH_FIRST`) can serve a run's later stages by the run's start. A run whose checks build nothing (`ci-script-tests`, `typescript-client` or `genie-freshness` on their own: none is in `BUILDS`)
takes a throwaway worktree too, since what a pool tree keeps is a warm cargo target, so it never waits behind building runs for a tree. Each check that is not one of those builds runs with `BUILD_ADMISSION_CLASS=light`, so the nix wrapper starts its `nix develop` shell as a light start rather than waiting in build admission as a heavy one (as the pre-sign worker starts `.#web` and `.#genie`); a building check never gets it, even when the caller set it. **Slot first:** a tree held while its run waits for a build slot is a tree nobody can use (2026-10-28: on one build host the slowest run waited
42,265 s for a tree while linux-gate in the held trees waited 1,000-2,400 s at a time for a build slot), and a tree cannot be given back while a
check is at work in it. So the best ticket takes a free tree only when a build started now would get a heavy slot at once: more of the shim's
heavy slots (`$XDG_RUNTIME_DIR/cargo-slots/heavyN`, probed with a non-blocking `flock` and let go at once) are free than heavy tickets wait in
its queue, and `build-admission check heavy` would admit it. Until then it holds no tree and says why once. After `TREE_SLOT_WAIT_LIMIT` (2700 s,
the gate's own `MAX_WAIT_SECONDS`) of a free tree left for want of a slot it takes the tree anyway, so a host whose slots never come free first
starves no run. A host without the shim's slots, or a gate that cannot be read, waits for nothing. Each run's record (and its summary) has
`tree_wait_s` (the wait for its tree), `tree_slot_wait_s` (the part of it with a tree free, waiting for a slot), `tree_held_s` and
`tree_blocked_s`: the time the build lane held the tree and built nothing, `tree_pre_start_s` before `ci-local` started (admission and the dev
shell) plus `tree_slot_blocked_s` with every running `ci-local` stage waiting for a build slot (one stage's wait while another builds is not
counted). **Every clone of one origin shares the pool**, so a fresh or throwaway clone takes a warm slot instead of building a target of its own:
the slot keeps its path (`tree-N` and `target-N`, so cargo's ids for the workspace crates, which contain the path, still match), and when the tree
there was last registered by another clone it is re-registered as a worktree of the clone now running (its notes land in that clone) while the inputs
the cleaning keeps (members, installed dependencies, compat baselines) move across by rename. The run still signs only what it built and tested in
its own exact tree: the tree is reset to the commit and cleaned every run, whoever used the slot before. A slot has one writer: its lease (an
`flock` on `tree-N.lock`) ends with the run that held it, but not always with what that run started. On 2026-10-28 a seat restart killed a run
during its checkout of `tree-2`, and the next five runs given the tree failed on the `index.lock` git left. So a free slot whose tree still has
a process at work in it (read from `/proc`) is busy: it is skipped, named once per waiter, until those processes end. A leftover `index.lock`
is removed before the checkout, and logged, only when no process is left in the tree. With one still there, the run fails rather than
sharing the tree. The pool cap (`ST_TEST_POOL_MAX_GB`)
evicts whole slots, least recently used first, the old per-clone layout included. When that tree has a
`megarepo.lock`, its members are materialized under `repos/` first with the command CI's `megarepoApplyStep` runs (`mr apply`
from the flake's effect-utils pin; a test pins the two together): `genie-freshness` imports `repos/effect-utils`, and the dev
shells only warn when it is missing. If that fails, no check runs and nothing is signed. A failing run is
recorded too. `evidence-wrap`, `evidence-sign` and `evidence-verify` come from the accepted base, and so does the `fractal-test` implementation:
a run started from a branch whose `scripts/st-test` differs from the base's re-executes the base's self-contained `scripts/st-test` first (once: `ST_TEST_REEXEC` names the base
commit; no opt-out), so how runs queue for pool trees and build slots, and which commit they sign, is always main's, however old the branch. A
branch's own `fractal-test` changes are tested by its `ci-script-tests`, which run `fractal-test-test` from the tree. It prints one
line per check: result, seconds, and whether the verifier, from the base, now counts the tree as covered
(`untrusted_signer` until the host's key line is on main). `--push REMOTE` pushes the notes after they verify, with the identity git uses for REMOTE (or
`EVIDENCE_PUSH_TOKEN`); whether that identity may create `refs/notes/evidence/*` is a repository rule not yet confirmed for agent seats.
Publication delegates the remote name to git unchanged: its configured URL, pushurl, URL rewrites and credential helper apply,
including transport configuration supplied through `GIT_CONFIG_*`. `EVIDENCE_PUSH_TOKEN` adds an HTTP header without replacing
that configuration. The child `evidence-wrap` argv is echoed before each signed check (never the token).
Only the selected checks' covering records are sent, not the checkout's cached evidence namespace; per-crate publication batches at most
40 explicit refs per push. Repeating `scripts/fractal-test --check CHECK --push origin` publishes already-covered evidence without rerunning that check.
Once every check is done, one `ls-remote` asks REMOTE for every ref of what the summary calls covered: each covering record's
tag, note and input-key index refs, the crates' records rust-tests pushed, and records pushed as they were. Its `on-remote` row
says `confirmed`, or `pushed again` when some were gone and are back. If one is still missing it says `FAILED` and the run exits 1.
A push git reported as done can be undone before the run ends: on 2026-10-29 the nightly GC deleted the first two records of a
run whose branch was not pushed yet, and the run said covered (#4186).

**Break-glass for a broken `fractal-test` on main:** `ST_TEST_REEXEC` set to any value skips fractal-test's re-execution of the base's copy, so the
branch's own copy runs. It is only for signing and pushing a fix to `fractal-test` itself, only on the coordinating lead's word for that one run, and
never otherwise.
Without `--push`, the run keeps its records in this repository's local refs: per record a tag `refs/tags/evidence/<tree>/<record>` and a note
`refs/notes/evidence/<tree>/<record>` (per-crate test records add `refs/tags/evidence/input/rust-tests/<key>/<record>` and
`refs/notes/evidence-input/rust-tests/<key>/<record>`). Run `scripts/fractal-test --status` to see each check's coverage, not the tree ID.
The run's summary prints a short tree ID; its JSON run record has the full `tree` in
`${XDG_STATE_HOME:-$HOME/.local/state}/st-test/runs.jsonl` (`ST_TEST_RUNS` overrides that path; `off` disables recording).
Use that run's full tree ID, not necessarily `HEAD^{tree}`: when protected files differ from the branch point, it signs the merge with
`origin/main`. Without a run record, expand the summary's short ID with `tree=$(git for-each-ref --format='%(refname)'
"refs/notes/evidence/SHORT*" | cut -d/ -f4 | sort -u)`; proceed only if `test "$(printf '%s\n' "$tree" | wc -l)" -eq 1 &&
test "${#tree}" -eq 40` succeeds (one full tree ID, not an absent or ambiguous prefix).
Prefer `scripts/fractal-test --check CHECK --push origin` for publication, so the runner selects the covering record and its input indexes.
To publish by hand, set `tree` to that full ID and batch only that tree's explicit refs (Bash):

```bash
refs=()
while IFS= read -r ref; do refs+=("$ref:$ref"); done < <(
  git for-each-ref --format='%(refname)' "refs/tags/evidence/$tree/" "refs/notes/evidence/$tree/"
)
for ((i=0; i<${#refs[@]}; i+=40)); do
  git push origin "${refs[@]:i:40}" || break
done
```

Add only that run's `refs/tags/evidence/input/...` and `refs/notes/evidence-input/...` names to `refs` as explicit `REF:REF`
arguments before the loop, if needed. Do not push the full evidence globs: a shared clone holds other runs' records too,
and sending thousands together can fail with a remote `Internal Server Error` per ref.
Push the evidence refs before `git push origin HEAD:BRANCH`, so the branch's first CI run finds them; existing evidence refs are never
updated or deleted (a ruleset protects `refs/tags/evidence/**`). An agent note can yield `decision=skip ... signer=<principal>` only for
wrapper-kind checks on `pull_request` runs, never in the merge queue (see "Agent keys"), and only when all skip conditions hold.
A sampled pull request (one in five), or a check whose skip policy is still shadow, instead shows
`decision=run outcome=satisfied ... signer=<principal>`: the note counts, but the real job still runs.
On a host that runs the evidence log (`evidence-log.service` installed as a systemd user unit, see "Portable evidence" in the plan), a
run with `--push` then starts that unit without waiting (`systemctl --user start --no-block evidence-log.service`, or the JSON argv in
`ST_EVIDENCE_LOG_NUDGE`): a pass counts only once the log lists it, so the agent's record is logged in seconds instead of at the next
2-minute pull. The pull itself is the unit's, with the log key's grant; fractal-test never signs the log. Asking is best effort and never
changes the result.
A check the tree already has a passing signature for (the verifier's answer, the same one `fractal-test` asks after a run) is not run again:
it prints `already signed` and, with `--push`, pushes that note as it is, so re-running on an unchanged tree is near instant. A failed
record is not a signature. `--rerun` runs the check anyway. The megarepo members are fetched only once a check has to run.

For a separate publisher, use `scripts/fractal-test --out DIR` instead of `--push REMOTE` (the two are mutually exclusive). The
directory is made absolute relative to the caller's working directory before any clean worktree is entered. Each wrapper writes
`DIR/<record>.pack` and `DIR/<record>.json`, without importing notes, pushing refs or nudging the log. To keep the exported set
complete, export mode reruns every selected, relevant check even if a local note already covers it, including every selected
`rust-tests` crate. When `linux-gate` and `rust-tests` run together, the aggregate and per-crate packs still share the gate's nextest
report. Lane-edit refusals and irrelevant checks retain their usual behavior. Failed runs still export failure records and fail the
command. A successful export reports `packs exported, not imported or verified`: the publisher must publish and verify the records;
export alone does not establish coverage, and `--status` continues to judge imported notes normally. `--seed [REF] --out DIR`
exports all selected crates of that ref (the base by default), without seed mode's usual implicit push to `origin`.

Every check runs with one fixed `PATH` (`~/.local/bin`, `~/.cargo/bin`, `/run/current-system/sw/bin`, `/nix/var/nix/profiles/default/bin`, `/usr/bin`,
`/bin` and the directory of `nix`, those that exist) instead of the caller's (`~/.cargo/bin` stays because the clippy ratchet's baseline holds
the `clippy 0.1.97` that `cargo clippy -V` prints with it, not the longer one it prints without): cargo's build scripts record `PATH`, so two seats whose
PATHs differ would otherwise rebuild nearly the whole workspace in the same pool tree. `ST_EVIDENCE_PATH=keep` leaves the caller's `PATH`.
All the repositories' pools together are kept under `ST_TEST_POOL_MAX_GB` (default 60; `0` or `off` for no cap): before a run starts, with its own
tree already held, `fractal-test` removes the least recently used trees and their targets whose lock is free, oldest first, until the pool directory
(`~/.cache/st-test`) fits, never one in use, and says on stderr what it removed. The per-target guard (below) stays. A run can exceed the cap by its own growth (one run's tests, clippy and fleet-compat builds end near 60 GB with incremental
compilation, which the checks now turn off as CI does), so size `ST_TEST_POOL_MAX_GB` as the disk a host can spare minus about 60. Where cargo's
`build-dir` points elsewhere (for example `/srv/build/cargo/<hash of the workspace path>`), the intermediates live there, not in the pool, and the cap does not count them.
`scripts/ci-local` (what the Linux gate stages run through) measures and prints every time: the setup before the stages, each stage's start,
end and seconds, and the time it spent waiting in the build admission queue (the shim's `cargo: waiting in order` / `got build slot` lines
get a UTC stamp, and the waits are counted and added up per stage), in a table at the end. Each stage's output is also kept in
`~/.cache/ci-local-logs/run-TIME-PID/STAGE.log` (`CI_LOCAL_LOG_DIR` names another place; the latest 10 runs stay), because the scratch home
of the stages is removed afterwards.
A host can set a **load limit** that no repository file names: a number in `~/.config/st-test/max-load` (or `ST_TEST_MAX_LOAD`; `0` or no file is no limit).
While the 1-minute load average is above it, `fractal-test` (before it takes a tree or starts any check) and `scripts/ci-local` (before its stages) WAIT, saying
`waiting: load1 34 > 20 (host limit)` at the start and every minute, and after `ST_TEST_LOAD_WAIT_MINUTES` (default 30) fail with that reason (`fractal-test`
exit 2, `ci-local` exit 3) and nothing started: never a silent refusal. It exists because work no build slot governs (verifier runs, test fixtures) can
push a shared host's load high enough to starve its daemons. Whoever runs the host sets the number; the repository names no host.
**The tree it signs is the tree the CI gate will judge.** Usually that is the pushed head. But when protected files moved on main since the branch
point (or the branch is more than `HEAD_TREE_MAX_BEHIND` commits behind), the gate falls back to the MERGE tree (`head_tree=fallback:...` in its
output), which a signature on the head does not cover. `fractal-test` asks the base's own `scripts/ci-evidence-gate` (`branch_point_verdict`, the function
`choose_tree` calls: one rule, not a copy). When a signing run hears fallback, for either reason, it **merges the base into the branch** in your checkout
first (`git merge --no-ff`, with your git identity: a new head commit), then signs and tests that head's own tree. It prints the reason and the merge
commit (`fractal-test: the branch is 37 commits behind main, more than 24: merged origin/main (...) into my-branch as <sha>; the head tree is what is signed`).
A signed merge tree is void at main's next commit, but the merged head's tree stays the one CI judges for `HEAD_TREE_MAX_BEHIND` more commits of main
(as long as no protected file moves), so a run that waited hours for a pool tree is not lost to what landed meanwhile. A conflict stops the run at once,
with the branch as it was (`git merge --abort`); so does a checkout that gained uncommitted changes to tracked files during the wait. With `--push`, a run
that merged and exits 0 also pushes the branch at that merge to the branch of the same name on REMOTE (never forced, through the pre-push hook as any
push). It refuses, and pushes nothing, when the remote branch has commits the local one lacks. A detached HEAD, or a branch named as the base's, is not
pushed: the command is printed. `--status` changes nothing: it asks about the tree CI would judge now, the merge tree (`git merge-tree`, nothing
checked out), so the pre-push hook refuses a push when that tree is not covered even if the head tree is. A run that waits for a pool tree fetches
the base again once it has one and decides at the base's tip then (a branch that does not merge with the base is still refused before the wait).
A check whose base policy compares a note's input with the run's own value (`input_order`: `fractal-web`'s `kit_inputs`) is judged the way the CI gate
judges it: `fractal-test` passes the verifier `--run-input kit_inputs=...`, computed by the base's `scripts/ci-fractal-web-changes` (`kit_paths` of the diff
from the merge base; `true` when that cannot be told) in `--status`, in the pre-push hook (which only asks `--status`) and in the skip of an
already-signed check. Without it a signed `fractal-web` tree read as `not covered: no_run_input`.
`scripts/fractal-test --status` runs nothing and says whether the tree is already covered (exit 0 if all, 3 if not). A tree that edits a check's lane
is signed like any other since 2026-10-26 (the base's `evidence-wrap` records the edited definition's digest, which the host level's `"lane":
"tree"` accepts), so it is covered only by such a note; against an older base, whose wrapper refuses it, the check reads `lane edit: CI runs it`,
and the pre-push hook refuses that row like a missing note. Unless `--check` names checks, every check of the skip
policy is listed, and one without a local command (fractal-web today) reads `CI runs it`.

**An agent's push is refused unless its tree is covered.** Our agents run the tests deliberately, as part of their work, with
`scripts/fractal-test --push origin`, which signs what it ran. When `ST_AGENT` is set and this machine's signer is listed in `.st/evidence-keys` on
main, the pre-push hook only asks `fractal-test --status` about HEAD's tree. Covered, and the push goes silently (receipt `covered`). Not covered,
or the status cannot be read, and the push is refused (receipt `refused`, with the uncovered checks): the hook prints the status and the command to
run, and never signs anything itself. That command is long (it builds and tests the Rust workspace, and the machine's build cap queues
it): it is run as a normal long or background shell command, and the agent waits for it. `ST_SKIP_EVIDENCE=1` and `ST_EVIDENCE_LOCAL=off` do not
apply to such a push. Only in an emergency, `ST_EVIDENCE_OVERRIDE='reason'` lets it go: one line is printed and the receipt records outcome
`override` with the reason; `git push --no-verify` stays git's own escape. A machine without a signer, or with one not listed on main, and a person's
push (no `ST_AGENT`) keep the behaviour below: nobody is refused for lacking a key. The installed stub blocks a push only on the hook's own refusal
(its exit 3); any other outcome, a crash included, lets the push go. A stub written before this change discards every exit code: it is replaced on
the next `nix develop` from a tree that has the new `scripts/install-git-hooks`, and by running that script from main.
`scripts/fractal-test --status` runs nothing and says whether the tree is already covered (exit 0 if all, 3 if not).
The check that builds the Rust workspace (`linux-gate`) builds in one cargo target per pool tree kept between runs,
`~/.cache/st-test/<hash>/target-N` (outside the tree, so the cleaning keeps it; `ST_EVIDENCE_TARGET=PATH` names one shared target for all
trees instead, which one run at a time builds and tests in: its lock is exclusive for the whole run, because two runs on different
trees sharing it once tested each other's binaries; `off` builds in the tree itself; a `CARGO_TARGET_DIR` you set is left alone), so a run does not start cold: a tree's path
is the same every run, so the dependencies, build scripts and unchanged workspace crates are reused.
Operations' terms: it is removed before a run once it passes 80 GB (a limit below one run's size would empty it every time; the run holds the tree's lock, so nobody else uses it), it is never
pinned, and the host's scratch sweep may reclaim it when idle (then the next run is cold). The old per-host `~/.cache/st-test-target` is
no longer used by default and can be deleted.
**The host's build cap governs the checks that build.** Each runs inside a `builds.slice` scope (`systemd-run --user --scope
--slice=builds.slice`), so everything it starts is slice-bound, including compiles a test runner starts without the shim (cargo-nextest's own
cargo, the dev shell's sccache client), and each cargo command inside takes the host's build slots through the cargo admission shim like any
build. It holds no heavy-slot ticket for the whole check (`cargo --stage-slot`): the tests run under the test guardian
(`scripts/fractal_test_process.py`), which detaches from its parents, and the shim lets a cargo command skip the queue under a held stage only
when the stage is its ancestor, so the guardian's cargo commands queued for the one heavy slot their own stage held until the stage limit
killed it (2026-10-23, on a build host: a canaries stage killed at 2400 s having built nothing). Only what starts `evidence-wrap` changes; the command it
records and signs does not. A host without a user systemd bus runs the check as before and says so on stderr; `ST_EVIDENCE_GOVERN=off` does the
same.

**How long a run takes, and the objective.** A full warm run of the per-PR suite (`scripts/fractal-test` with no `--check` and no `--crate`,
every default check including the per-crate `rust-tests` signing, on a pool tree whose kept target already holds a build) takes 12 minutes
(720 s) or less on a dev box. It is in breach when the p90 of those runs is over 12 minutes, or when any one of them takes over 15 minutes
(900 s). So that this is measured rather than remembered, every run that reaches its summary appends one JSON line to
`~/.local/state/st-test/runs.jsonl` (under `$XDG_STATE_HOME` when that is set; `ST_TEST_RUNS=PATH` names another file, `ST_TEST_RUNS=off`
records nothing; it is read before the seat scrub, like the other `ST_TEST_*` settings, so the checks never see it). One write with
`O_APPEND`, so concurrent runs never interleave; a file that cannot be written is one line on stderr and never changes the exit code. A
record (`"v": 1`) has `started` and `ended` (UTC, from before the base fetch to the summary), `total_s` (monotonic seconds), `host`, `tree`,
`commit`, `base`, `behind`, `full` (no `--check` and no `--crate`), `checks`, `pool_tree`, `warm` (the build target was not empty before the
run; `null` when no check builds), `parallel` (the light checks ran alongside the build lane), `rows` (every summary row's name, result and
seconds), `exit`, and `stages`: `linux-gate`'s stages (name, start, end, seconds, exit, queue waits and queue seconds) from the table
`scripts/ci-local` prints at its end, read from its output, which fractal-test copies to the terminal as it comes and keeps (`[]` when there is
no table). `scripts/fractal-test-report [FILE...] [--hours 24 | --since ISO] [--slo-s 720] [--max-s 900] [--markdown]` reads those files and
prints, over the full warm runs in the window that ran and signed every check (exit 0), the count,
the nearest-rank p50 and p90, the maximum, the share within the objective, whether it is breached and why, the same per host, and the five
slowest stages or checks; the runs left out are counted by reason (`not_full`, `cold`, `partial`: exit 4, a tree that edits a lane runs nothing for that check, and `failed`). It writes nothing.

A pre-push hook does this without anyone remembering. `scripts/install-git-hooks` (run by the default `nix develop` shell outside CI; it prints
one line the first time and does nothing when `GITHUB_ACTIONS` or `CI` is set) installs a stub in the repository's hooks directory, never replacing
a hook it did not write and leaving a configured `core.hooksPath` alone. **The hook and the runner are not the pushed branch's:** the stub takes
`scripts/evidence-prepush` and `scripts/fractal-test` from `<remote>/main` into a temporary directory and runs those copies (when the base has none it
does nothing). The checks they run are the tree's code (`nix develop`, `pnpm install`, the tests), on this machine as you with the host's signing key
within reach: that is cooperative trust (known defect 22), not something the base-pinned hook protects against. When you push the branch
you have checked out, the hook (for a push that the paragraph above does not refuse or let through) asks `fractal-test --status`; if every check is covered the push goes,
silently. Otherwise it runs `fractal-test --push REMOTE`
(an interactive push waits for it up to `ST_EVIDENCE_TIME_LIMIT`, default 1200 seconds, then stops it with TERM and KILL of its whole process
group) and the push goes. A push without a terminal on stderr (a script, or an agent whose signer is not listed) runs inside a tool call with a short timeout, so there the hook
waits at most `ST_EVIDENCE_AGENT_LIMIT` seconds (default 300; an agent's tool call allows 600 s), prints that signing continues in the background with its log path, and lets the push go; the notes
are pushed when the run finishes. `typescript-client` runs first because it is signed for nearly every tree, and each note is pushed as soon as its
check is signed, so a note signed within the wait is on the remote before git sends the branch (the hook finishes before git updates any ref) and the
first CI run sees it. A tree that edits a lane's definition is not signed for that lane: fractal-test says "not signed" for it without running it and exits
4 when nothing else failed, and the hook says CI runs that check, not that a check failed. One signing run per tree: a push of a tree that another
push on the machine is still signing (in the background, say) says so and goes, and that run's notes follow. Each push of the checked-out head appends
one line to `~/.local/state/st3/evidence-prepush/receipts.jsonl` (`time`, `tree`, `branch`, `agent`, `outcome`: `covered`, `signed`,
`signed_lane_edit`, `background`, `failed`, `skip_flag`, `local_off`, `not_signing:REASON`, and for an agent `refused` or `override`), append only, so the pre-sign rate can be read per push.
**The hook's environment is scrubbed (st2#3014).** git exports `GIT_DIR` (and `GIT_PREFIX`, `GIT_EXEC_PATH`, ...) to a hook, and `GIT_DIR` is set whenever the push is made from a linked worktree, which every agent seat uses. A script run with it that makes its own repository or says `git -C elsewhere ...` still acts on the CALLER's repository, because `GIT_DIR` wins over `-C` and over the working directory. The repository's own fixture tests do exactly that; run from the hook they committed on the caller's branches, rewrote the shared repository's identity and pushed. The stub, `evidence-prepush`, `fractal-test` and `ci-genie-freshness` (through `scripts/git-env-clean`) now drop the variables that select a repository or an identity (`GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE`, `GIT_OBJECT_DIRECTORY`, `GIT_ALTERNATE_OBJECT_DIRECTORIES`, `GIT_COMMON_DIR`, `GIT_NAMESPACE`, `GIT_PREFIX`, `GIT_QUARANTINE_PATH`, `GIT_AUTHOR_*`, `GIT_COMMITTER_*`, `GIT_PUSH_OPTION*`, ...) before they start anything; transport and credential variables (`GIT_SSH_COMMAND`, `GIT_ASKPASS`, `GIT_TERMINAL_PROMPT`, `GIT_HTTP_*`, `GIT_CONFIG_*`) stay, because the notes push needs them. The shell scrub uses only `export -p` (the non-interactive bash of a nix shell has no `compgen`, which made its first version a silent no-op there) and checks its own result: a scrub that did not work stops the caller, and makes the stub skip the hook with a message. The stub is a copy in each repository's hooks directory but runs `evidence-prepush` and `fractal-test` as they are on `origin/main`, so a stub installed before this change is covered as soon as it is on main; `install-git-hooks` replaces an older stub carrying its marker on the next shell entry. Tests push from a linked worktree through the real hook, also with `compgen` disabled, and require the caller's config, HEAD, status, branches and tags to be byte-identical.
Apart from an agent's refusal, it never blocks a push: a failing check is recorded and printed, a timeout is printed, and so is every other failure (an exception, a missing `python3`,
a slow fetch, a signer that `.st/evidence-keys` on main does not list, a missing signing key). `ST_SKIP_EVIDENCE=1` does not wait: the push
goes at once, the same signing runs in the background, and the hook says that a CI run that starts before a note is there runs that check in full.
`ST_EVIDENCE_LOCAL=off` does no local signing work at all, not even the status question, and says so in one line (for a seat or unit where
local builds are fenced; it wins over `ST_SKIP_EVIDENCE`); `git push --no-verify` skips the hook entirely. Tags, deletes,
other branches and the base branch are pushed untouched, and the notes push inside the run does not run the hook again. A new tree always takes the slow
path (the checks run once); so does every fixup that changes the tree; a tree that already has notes is instant.

The checks run without the agent seat's identity and session variables (`ST_*`, `ST2_*`, `ST3_*`, `CLAUDE*`, `ANTHROPIC_*`; `ST_AGENT`, `ST_SKIP_EVIDENCE` and the `ST_EVIDENCE*` switches stay, by exact name: that list in `scripts/fractal-test` is the only exception mechanism, and `ST_AGENT` stays visible to the checks because the `nix` wrapper that needs it starts them), as on a CI runner: tests that read them fail or behave differently under a seat. Tests that bind a request to the seat by process ancestry still do, so a trustworthy full run of the Linux gate must be started detached from any seat. It runs `nix develop` and `pnpm install` from the tree under test, on the signing host as its user: code in that tree runs
there with the key within reach (cooperative trust, see Host keys). It changes no policy: a check skips only where `enforce` is true.

**The signer starts with a scrubbed loader environment.** `evidence-sign` refuses to start when the loader or the interpreter was
told to load code (`LD_PRELOAD` and the rest of its `REFUSED_ENVIRONMENT`): a preloaded library inside the signer is an injection
point, and the refusal stays. Some hosts set `LD_PRELOAD` for every process (one build host preloads jemalloc), which made every signed push
there report "a check failed". So the launcher scrubs instead: `evidence-wrap` resolves the signer program with the caller's PATH,
then starts it by absolute path without any `LD_*`, `DYLD_*` or `PYTHON*` variable, `GLIBC_TUNABLES`, `GCONV_PATH` or `NLSPATH`,
and with `PATH=/run/current-system/sw/bin:/usr/bin:/bin`; `fractal-test` starts the signer's interpreter with `-I`. The checks themselves
keep the caller's environment. Tests set these variables in the caller and require a signed note, and require the signer to still
refuse when it is started with `LD_PRELOAD`.


**The VM tests (`scripts/ci-local isolation-vm`).** `isolation-vm` runs daily, not per pull request (2026-10-23), so fractal-test does not
run it and no policy accepts a local note for it; `nix develop -c bash scripts/ci-local isolation-vm` still runs it by hand, which runs `scripts/ci-local-isolation-vm` in the stages' own home: it probes `/dev/kvm` (read-write, and a VM can be
created), builds in place what CI's isolation-vm job downloads from linux-test-build (the workspace test archive, `cargo nextest
archive` with CI's own filter, and the standalone production gateway, refused when it carries `test-support` or a test profile), then
builds and runs the transport isolation VM driver with that archive and the sekrets VM driver with that gateway, with `TMPDIR` in the
run's scratch. Its lane (`.st/required-lanes.json`) declares `.config/nextest.toml`, `.st`, `flake.lock`, `flake.nix`,
`nix/sekrets-vm.nix`, `nix/transport-isolation-vm.nix` and `scripts`. A machine without a usable `/dev/kvm` fails it.

**The web lane (`scripts/ci-local-web`).** `fractal-web`'s local command is `nix develop .#web -c bash scripts/ci-local-web`, which runs
what CI's fractal-web-execution job runs (`scripts/ci-web-shell-test`, then `scripts/ci-fractal-web`) as ONE shard that covers every kit
play and app test (`KIT_INPUTS=true`, `KIT_SHARD_COUNT=1`, `KIT_PLAY_SET=all`, a11y on, no per-play cap, `APP_TEST_SET=all`): more than a
pull request's own run, and with no base to compare with. It runs in a home of its own on the account's disk, with a CI runner's umask,
no seat or credential variables and none of the account's tool wrappers on `PATH`; the pnpm store is kept, and Buck's file watcher is the
host's Watchman service, named by its socket. The note records `kit_shard_count=1`, `shard_results=0=success`, `kit_inputs=true`,
`kit_play_set=all`, `app_test_set=all` and `event_name=pull_request`, so the wrapper kind's shard-count and input-order rules apply to it
as to CI's aggregate note; its host and agent levels admit it for `pull_request` (`by_level`, `lane: true`). fractal-test asks the base's own
`scripts/ci-fractal-web-changes` whether any web input changed since HEAD's merge base with the base: when none did, CI does not run the
lane, so `--status` reads `not relevant` (covered) and a run skips it, neither a failure nor a note.

**Linux gate stages (`scripts/ci-local`).** The stages see a CI runner's temporary directory, not the caller's: the run's scratch
is on the account's disk under `~/.cache/ci-local` (`CI_LOCAL_SCRATCH_ROOT` names another) and its `TMPDIR` is a short directory,
`/dev/shm/cl.XXXXXX` when `/dev/shm` has 20 GiB free and `/var/tmp/cl.XXXXXX` on disk otherwise (`CI_LOCAL_TMP_ROOT` names another root;
tests put Unix sockets several levels below it, and a socket path is limited to 108 bytes, which the daemons' sockets in the delivery-probe
and seat tests exceeded under the scratch; in RAM the tests' SQLite commits do not wait for a disk sync: on a build host the linux-gate tests
took 849 s there against 1518 s on disk, and 519 s with 12 threads, at about 3.3 GB of `/dev/shm`), because a dev shell's `TMPDIR` is
under `/tmp`, which on some build hosts is a small tmpfs (tests that need tens of GB of scratch are refused there) or holds a stray
`/tmp/.git` (tests that look for the enclosing repository find it); a host's preloaded allocator (`LD_PRELOAD`) is dropped, as no
CI runner has one (the `malloc_trim` tests fail under it); the stages compile without sccache (`RUSTC_WRAPPER` is unset, as CI compiles
when its cache is unavailable), because a cache server started inside the run keeps the run's `HOME` and `TMPDIR`, removed at exit, and the
account's builds share that one server, so every compile through it failed afterwards (the kept build directory is the local cache);
each test process gets a `TMPDIR` of its own under the run's (`scripts/ci-test-tmpdir`, set as the cargo target runner, which nextest
honours for every test and every listing), because one test that left an empty `.git` at the top of a shared `TMPDIR` made every later
test there see a repository that is not one (a CI runner is fresh for every job); and the
stages run with a CI runner's umask, 022, in a scratch no one else can
write to (the secrets gateway's tests refuse a group-writable directory, and some hosts' umask is 002). fractal-test runs every check by
default, `linux-gate` with the others: there is no subset for signing. `linux-gate`'s local command is
`nix develop -c bash scripts/ci-local tests clippy fleet-compat`, the three stages a `linux-gate` skip covers (the test build with them, after preparing what CI's test-build job does for the tests: the
provider components the resource-provider tests load and the previous release the messaging compatibility tests run against; `upgrade-impact` is not skipped and runs in CI), with clippy's
ratchet compared with the base (as CI compares with the event's base): fractal-test gives the stage the base commit it has just
fetched (`CLIPPY_BASE_SHA`) and records it in the note as the input `clippy_base_sha`, which a host or agent `linux-gate` note must
carry; run by hand, `ci-local` fetches main itself and never trusts a local ref, and refuses when it cannot. Every other input
`evidence-wrap --input NAME=VALUE` adds is recorded the same way; it refuses the names it sets itself. Each stage is the `scripts/ci-linux` stage as its CI job runs it, after building and
installing the rendered hooks as the job's test-archive setup does, in a home of its own. HOME and the XDG state, config, data
and cache directories are a scratch directory that is removed afterwards, and no st variable (`ST_*`, `ST2_*`, `ST3_*` and,
since the rename, `FRACTAL_*`: a seat's live endpoint and identity; kept are the dev shell's own `ST2_OTELITE_BIN`, `ST3_OTELITE_BIN` and
`FRACTAL_OTELITE_BIN` when they name a store path, the collector CI's OTLP export tests run against), GitHub, SSH agent, evidence or sekrets variable reaches the stage, so the daemons and harnesses its tests start cannot read or write the account's Fractal state, harness
configuration or credentials, and no PATH entry inside the account's Fractal state stays. Kept from the account: the cargo and
rustup homes (the dependency cache), `XDG_RUNTIME_DIR` with the session bus, where the build admission shim queues, and the
host's admission itself: the shims and the `build-admission` program look for their program, limits, priorities and queue state
under the account's home, which the stages' `HOME` would hide (every build then took the shims' fallback cap under a lock of its own),
so `scripts/ci-local` names them from the account's home (`BUILD_ADMISSION_BIN`, `BUILD_ADMISSION_CONFIG`, `BUILD_ADMISSION_STATE`,
`BUILD_ADMISSION_FALLBACK_LOCK`, `CARGO_PRIORITY_FILE`, `CARGO_HEAVY_SLOTS`; a value the caller set is kept). The
stage takes an ordinary heavy admission slot, with no priority of its own. Test binaries run outside that gate, so on a
shared host the stages' tests run 12 at a time (`CI_TEST_THREADS`; CI's runners use 8 on a machine of their own) under
`ionice -c 3` and `nice -n 10`: an unbounded run (one test per core) held the dev host's IO pressure above its alarm line. The shim is handed the dev shell's own cargo as
`CARGO_REAL` (`CI_LOCAL_CARGO`, or the first cargo on PATH outside the account's home that is not the shim): a build host's
default real cargo may source a saved dev shell environment from another day, and on 2026-10-22 one such file put an older
pty first on PATH, which made every boot canary fail on that host (the seat launch passes environment names only). The lane in `.st/required-lanes.json`
holds both the host and the agent level to its definition (`scripts`, `.st`, `.config/nextest.toml`, the flake; for `linux-gate`
also the clippy ratchet's baseline and splits and the pinned previous release fleet-compat runs against), so a tree that edits
what decides what the stages run never skips on a local note. `linux-gate`'s tree inputs are the check's declared `inputs`, so an
input-keyed note serves another tree with the same key. The host and agent levels cover pull requests only, and the merge
queue runs the stages. Measured on the 16-core dev host: the canaries alone 345 s wall, build included; the four `linux-gate`
stages 1592 s wall with a warm target and the tests' scratch on `/dev/shm` (tests 871 s, clippy 390 s, fleet-compat 211 s, canaries
120 s), at 6 test threads, peak IO pressure 12.6%. A cold target adds about 10 minutes and roughly 19 GB of writes.

**The pre-sign worker (`scripts/evidence-presign-worker`).** Signing at push time depends on the push: a push with `ST_SKIP_EVIDENCE=1`
does not wait, and a push made where no hook runs signs nothing. The worker closes that gap on the signing
host, from the tools' side. Two systemd user timers (`scripts/evidence-presign-light.timer` every 2 minutes, `scripts/evidence-presign-heavy.timer`
every 5) each run one tick of one class through `scripts/evidence-presign-timer`, which reads the worker from `origin/main` of a clone the
worker owns (one per class, so a light tick never moves the checkout under a heavy run). A tick lists the open pull requests and keeps the
ready ones (not drafts) that target `main`, come from the repository itself (no forks) and are authored by a GitHub login in
`~/.config/evidence-presign/authors`. That file is the host's own configuration, not repository data a pull request could edit; without it,
or with no login in it, the worker refuses to run, and an outside author's code is never run (it would run with the host's key within
reach). Newest first, it takes the first head with a check of its class that `fractal-test --status` does not call covered and that it has not
run for that tree before, runs `fractal-test --check NAME... --push origin` on it, records the result and stops: one head per tick, serially per
class (a lock; a tick still running skips the next). The light class is what `fractal-test --list` prints, now every check (fractal-test has no
subset for signing since 2026-10-23); the heavy class is what `--all` adds to that, which is nothing. Both timers are off on the signing host. A
failure or a lane-edit refusal is a result, recorded and not retried for that tree (CI runs the check); a run stopped for load is not, and
that head waits 30 minutes before it is tried again. Load, for the heavy class: it starts only after IO pressure (some avg60) stayed under
8% for 10 minutes with no admitted heavy build active and at least 140 GiB free (it clears nothing to make room), runs with `TMPDIR` on
`/dev/shm`, and is stopped (TERM, then KILL of its process group) above 15% avg60, under 16 GiB available memory, under 136 GiB free disk
or past 20 GB of scratch; the light class starts below 15% and stops the same way. The heavy class keeps its cargo target between runs
in `~/.cache/evidence-presign-target`, so a head does not start cold (a cold target costs about 10 minutes and 19 GB of writes). It is
never pinned: the host's scratch sweep may reclaim it when it has been idle for two hours and disk runs short, and the worker removes it
itself before a run once it passes 25 GB. Every tick that does not start for load or disk appends a line to `skips.jsonl`. `~/.config/evidence-presign/limits.json` overrides these numbers (agreed with operations) and
`~/.config/evidence-presign/off` stops every tick. Both services run in `builds.slice`, with every other build on the host (operations' terms). The units' environment is minimal on purpose: they run pull request code with the
host's signing key within reach, so they carry no secret beyond that key and the git credential the notes push uses, and
`~/.config/evidence-presign/env` holds settings only (`EVIDENCE_PRESIGN_*`), never a token. The authors file lists only our own account.
The heavy class's kept target is shared between heads; that is accepted for our own authors' code (the same trust as a seat's warm
checkout), and it is one reason the authors file must stay that short. A check's result is read only from fractal-test's final summary. Each attempt appends a JSON line to `~/.local/state/evidence-presign/attempts.jsonl`
(pull request, head, tree, class, first seen ready, start, seconds, per-check result, peak IO pressure, outcome `worker_signed`,
`worker_signed_lane_edit` (signed, apart from a check whose lane the tree edits), `lane_edit_not_signed` (nothing
signed: the tree edits every lane), `failed` or `deferred`), and `evidence-presign-worker summary` prints, per class, the ready heads seen in the
window, how many had every check covered, the median and worst wait from first seen ready to covered, the runs and their outcomes, and
how many ticks did not start (for disk or load) and how many runs were stopped, by reason.

**The shadow merge-queue signer (`scripts/evidence-queue-signer`).** It measures whether a signer could keep the merge queue from
rebuilding what is already built, without changing anything: a host note never covers a `merge_group` run, so it skips, waits for and
blocks nothing (`seed`, below, is the exception: its notes are the per-crate ones pull requests reuse). Three systemd user timers run it through `scripts/evidence-queue-timer`, which reads it (and the pre-sign worker, whose host
rules it uses) from `origin/main` of a clone per command. `observe` (every minute, seconds) reads the merge queue, fetches each group
commit while its `gh-readonly-queue` branch exists (so a dropped entry is measured too), and records the group tree, whether `git
merge-tree` of the pull request head onto the entry's base gives the same tree, and, depth 1, the tree predicted for the next pull
request to be queued (from the host's optional `~/.config/evidence-queue-signer/next-command`, which prints `<number> <head sha>` lines
in queue order) merged onto the queue's tail. When an entry has left the queue and the queue's own jobs on its group commit finished, it
appends one line to `~/.local/state/evidence-queue-signer/entries.jsonl`: the pull request, whether it landed, the trees and whether they
are equal, per check when the queue's gate looked (the `evidence-gate-NAME` job's start) and its result (the `NAME` job), and the
signer's run for that tree, with whether each note was there before the gate looked and by how many minutes, and whether the results
agree. `sign` (every minute, one run at a time) takes the prediction first, since only a tree signed before enqueue can be ready in
time, then the queue's entries in position order, never a tree twice, and runs `fractal-test` with every local check on it (the group commit
as fetched, or the prediction's merge of the same two parents, which has the same tree), signing as the host. Only our own authors'
code runs: a tree runs when its pull request and every entry ahead of it are by a login in `~/.config/evidence-presign/authors`;
`~/.config/evidence-queue-signer/every-author` (the operator's decision) runs every entry. Its runs go through the host's build slot
like any `fractal-test`, and the worker's stop limits apply. It starts on operations' terms for the host: IO pressure under 12% (three
points under the stop), at least 165 GiB free and 20 GiB of available memory, without the worker's wait for ten calm minutes
(`~/.config/evidence-queue-signer/start.json` overrides them, agreed with operations). Each
run appends a line to `attempts.jsonl` (wait from first seen to start, seconds per check, and each note's time, read back from the
pushed notes' `observed_at`, or estimated from the start and the per-check seconds when none can be read, as `signed_at_source`
says). A tree is run once whatever the outcome, so a tree stopped for load is not retried. `seed` (every minute, one run at a time)
runs `fractal-test --seed COMMIT` on each new tip of main, which signs and pushes the per-crate `rust-tests` notes of that tree for the crates
whose key has none yet, so a pull request branched from main reuses them; most merges move a handful of crates, one that moves the lane
all of them. Main's code is merged code, the code every run on the host already builds under a pull request, so no author rule applies.
It starts on the same terms as `sign`, and not while another `fractal-test --seed` runs on the host (that tick is skipped; the next tries
again); its nextest takes the build slots through the admission shim like any `fractal-test` build. A tip is seeded once whatever the outcome
(the next merge's seed covers what one left missing), except that a seed load stopped before its tests started (waiting for a build
slot, or building; nextest's `Starting N tests` line is the boundary) tries the same tip again on the next tick, up to six tries, until
a newer merge replaces it. Each try appends a line to `seeds.jsonl` (wait from the merge to the start, seconds, the try number, whether
the tests had started, the pool cap `ST_TEST_POOL_MAX_GB` it ran with, and the crates that did not sign). `evidence-queue-signer report` sums a window, and `~/.config/evidence-queue-signer/off` stops
all three.

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
  on the hosted image or on timing (`typescript-client`). That is a choice, recorded in
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
tree=… tree_of=head|merge pr_head=… merge_tree=… head_tree=used|fallback:…|not-a-pull-request behind=… ci_inputs=kept|changed|- notes_fetch=ok|none|failed
policy_sha256=… keys_sha256=… head=… base=… event=… pr=… repo=… run=… attempt=… fork=true|false verifier_sha256=… gate_sha256=…
```

(one line; wrapped here). Each job also ends with exactly one of these steps running, which is how a
collector reads the outcome from the jobs API: `evidence satisfied by existing note`,
`evidence existing note is degraded`, `evidence no note`, `evidence note rejected`,
`evidence verifier unavailable`, plus `evidence skip taken` when the decision is skip. A note made for a pull request is reused by the merge queue's run
only when the tree is identical, that is, when main has not moved in between.

The gate step also writes the line as its output `line` (first, with control characters replaced, so it cannot stand in for
`outcome`, `skip` or `record`), and each gate job passes it on as `outputs.line`. Only `evidence-presign-notice` reads it.

### Not pre-signed: the notice (`scripts/ci-evidence-presign-notice`)

Nothing rejects a pull request for a missing note. A pull request that is not pre-signed runs its checks, and CI signs them.
The `evidence-presign-notice` job only says so. It runs on pull requests (not drafts) after the six gate jobs and takes
`scripts/ci-evidence-presign-notice` from the accepted base, like the gate. It never fails, is not a required check, and nothing
reads its result.

It lists a check only when all of these hold:

- `scripts/st-test` can sign it: the check is a lane of `.st/required-lanes.json` at the base.
- Its gate ran it for lack of a note: `decision=run outcome=no_note` on a same-repository pull request.
- The gate asked about the head's own tree (`tree_of=head head_tree=used`). After a fallback to the merge tree, a pre-signed
  note could not have counted.
- The pull request does not edit the lane's `definition_paths` (from the pull request's file list). `st-test` refused to sign
  such a tree before 2026-10-26; it signs it now.

With a check listed, it writes one notice annotation and the job summary:

> These jobs ran in full because no pre-signed note covered this head: genie-freshness, typescript-client. Run
> `scripts/st-test --push origin` before marking the pull request ready, and CI can skip them. How: (#3621). This never fails a
> check or blocks a merge.

The command is the same whatever is listed: st-test runs the whole suite. Every run also prints one counting line,
`evidence-presign-notice v1 pr=… listed=…|- passed_over=check:reason,…|-`. If the lanes file or the file list cannot be read,
the affected checks are passed over, never listed.

This is phase A, a notice only. Phase B, from a date announced in #3621 first, adds one comment on the pull request, edited
in place on later pushes. It still never blocks anything.

### Loops

No workflow has an unfiltered `push` trigger (the `push` triggers are limited to `branches: [main]` or
tags), so pushing `refs/notes/evidence/*` starts nothing; a test fails if one is added. The gate holds
`contents: read`, so it cannot push, dispatch or comment and cannot cause a rerun. The record's
`attempts` count (trusted records for the tree and check) is what a producer reads to refuse a
second production.
