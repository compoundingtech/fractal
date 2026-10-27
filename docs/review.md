# The review standard

Every pull request to this repository is reviewed against this one standard (other repositories use it where it fits), by
whoever or whatever runs the review: a contributor's agent before it pushes, `scripts/review` (which runs the standard over a head
and signs the verdict), or a second reader. The standard is written here so that no reviewer depends on what was said in a
conversation, and so that two reviewers given the same head reach the same verdict.

A change goes from idea to main in this order, and each agent does all of it for its own change:

1. **Idea and development.** The author works on a branch and merges `main` into it (never rebases, never force-pushes).
2. **Signed checks.** `scripts/fractal-test --all --push origin` runs every check on the head and signs what it ran. No push goes out
   with a check unsigned; a check that fails or flakes is fixed first.
3. **Signed review.** The review below runs on the signed head and produces a structured verdict, which is signed like a check.
4. **Merge queue.** A head with every check signed and a signed review that is `GO` at **low** or **medium** risk is put in the
   merge queue by its own author, following the queue rules below.

There is **no central intake, curator or watcher** in this flow: nobody reads, approves or queues a change on its author's behalf.
Each agent runs the review on its own change and queues its own change.

The review is a reading. It does not run the tests (the signed checks did), it does not judge whether the product should do
the thing, and it does not decide questions that only a person can answer. It answers: *does this exact head do what it says, is
it safe to put in front of everyone, and what could go wrong that the checks cannot see.*

## The verdict

A verdict is one structured artifact, so a machine can gate on it and a person can read it. It is a JSON file that must validate
against `.st/review-verdict.schema.json` (`scripts/review-validate`); a review whose artifact does not validate **fails locally and
nothing is signed**. The signature covers the whole artifact, the review's own output included, so what is signed is exactly what
was said.

**Status of the tools.** The risk script (`scripts/review-risk`), the schema and its validator
(`.st/review-verdict.schema.json`, `scripts/review-validate`) are merged. The runner (`scripts/review`) follows in the next pull
request; until it is merged the reviewer does its steps by hand: it writes the verdict against the table below, runs
`scripts/review-risk` for the deterministic score and `scripts/review-validate` on the verdict. The daily miss rate starts when the
independent re-review exists.

| Field | Meaning |
| --- | --- |
| `head` | the full 40-character commit the verdict is about; a verdict is void the moment the head moves |
| `base` | the full `main` commit the head was read against |
| `round` | 1 to 3: which attempt this is (see the loop) |
| `verdict` | `GO` or `CHANGES` |
| `risk.deterministic` | the level (`low`, `medium`, `high`) and reasons computed from the diff alone by `scripts/review-risk`, using the rules named in `.st/review-risk.json`, read from the base and never from the head under review, so a change cannot lower its own floor; no model is involved, so two runs agree. Changing the trust files under `.st/` (keys, authors, the skip, policy and required-lanes files, and the review's own rules, classes, schema and harness) is high |
| `risk.llm` | the reviewing model's own level, with its reasons |
| `risk.final` | the **higher** of the two: the model may raise the deterministic floor and never lower it |
| `findings` | each with `severity` (`blocking` or `advisory`), `file`, `line`, `reason` and the fix that would be accepted |
| `flags` | `public_contract`, `stored_format`, `trust_or_permission`, `global_view`, each `yes` or `no` with one sentence |
| `read` | what was read in full, what was read as structure only, what was served from the review record |
| `patch_ids` | the per-file patch-ids of the head's diff, which the next review's delta read starts from |
| `answers` | for a round after the first: which earlier finding each change answers |
| `reviewer` | the model, its family and the harness that ran it |
| `standard` | the digest of this document, so a verdict names the rule it was made under |
| `text` | the full review text, as the reviewer wrote it |

Words reviewers use, and what they mean:

- **GO** — every item under "What a GO means" holds at this head. A GO says whether the change may batch in the queue or must
  go alone, and that it is void if the head moves.
- **GO, delta** — the same, for a head that differs from one already approved: it says what was served from the record and what
  was read.
- **CHANGES** — at least one blocking finding. It names the file, the line, the reason and the fix that would be accepted, so the
  author can act without asking anything.
- **HOLD** — not a verdict on the code: the review waits on something named (a conflict to resolve, checks still running, a
  flake that has to be fixed first). It says what, and who gives it. There is no "GO, not final": a change either has a `GO` at
  its head, or it is a `HOLD` naming what is open.

## What a GO means

All at one exact head:

1. **Identity.** The verdict names the full head and base. A verdict is never reused for another head except by a delta read,
   which includes a head that is only a merge of `main` with an empty `git show --remerge-diff`.
2. **It merges cleanly.** `git merge-tree --write-tree origin/main <head>` succeeds. Whether a clean merge still breaks something
   that changed on `main` is judged by the checks signed on the merge (see "Behind main"), not guessed by the reviewer.
3. **Every check is signed at this head** (`scripts/fractal-test --status`; or the merge of this head with `main`, where the protected
   files moved), and none is red; the required checks on GitHub agree. A skipped check is a pass only where its signed note says
   why it was skipped.
4. **No flake.** A crate with a test that passed only on a retry is not signed in its own per-crate record, so on every path
   that check is left for CI and item 3 fails. The whole-tree gate note still signs a retried pass and records the tests as
   `flaky_tests`: a signed note with `flaky_tests` other than none is a finding to be dispositioned (the test is fixed, or the
   owner records why it is unrelated), not an automatic red. A flake is never waived by assumption, and the reviewer does not
   record one as unrelated on its own.
5. **It does what its description says, with tests that would fail without it.** The reviewer finds the test that fails on the
   old behaviour (or checks the author's claim that it does) and asks whether its assertions would catch the mistake it claims
   to catch. For a change that only touches tests, the reviewer reads intent: no coverage dropped, no assertion weakened, the
   daily or full-scale switch preserved.
6. **The diff was read as much as its risk needs, and the verdict says what was not read.** Read in full: scripts, workflows,
   evidence and signing code, runtime, reconcile and store code. Structure only: generated files, large fixtures, test corpora.
   There is no line cap, but a change too large to read in the time available gets a verdict that names the parts that were not
   read, never a bare GO.
7. **A release note** is present with every field filled and honest (replay, schema, rules, compatibility, downtime, manual
   step, recovery), and the documentation matches the change.
8. **The repository is public.** The public-repository check passes, and nothing names a real host, person or customer in code,
   documentation, a commit message or a pull request comment.
9. **Compatibility flags are right.** Any claim kind, claim field, client or wire change, schema change, new decision name,
   retention or replication change, or CLI contract change is flagged and raises the risk (see the rubric), with the rollout rule
   it needs (for example, daemons before clients).
10. **Evidence and trust paths are read the hard way.** For `scripts/fractal-test`, `scripts/evidence-*`, `scripts/ci-evidence-*`,
    `scripts/ci-local`, `scripts/ci-linux`, workflows, rulesets, `.st/` and the store: who can influence the input, which version
    of the script runs (the base's or the tree under test), whether it fails open or closed, and what a buggy or hostile tree
    could make it sign.
11. **The global-view rule holds.** The graph is distributed and eventually consistent, and an agent's state is local to one
    machine. A change that needs another machine's state, a total graph, a cross-machine lock, or that refuses because replicated
    knowledge is incomplete, is flagged `global_view: yes` and is not a GO. Local validation, authorisation, scopes, signatures
    and refusals about one machine's own state are ordinary: judge them on their merits.
12. **No containers.** No docker or container on any machine, for proofs, seats or CI jobs; a two-member proof is two
    `st --instance NAME` instances.

## The risk rubric

The verdict carries a risk, `low`, `medium` or `high`, and the reasons. Risk replaces asking a person: nothing that gates `main`,
a deploy, a release or the merge queue waits for a person's approval. **Only `low` and `medium` may enter the merge queue.**

**High** — any one of these makes the change high:

- a stored format or a claim kind: a new claim kind, a field whose meaning changes or is removed, a schema or table change, a
  rules-version change, a checkpoint or digest change
- deleting data, or an opt-in deletion that has no way back
- a trust boundary or a permission: who may sign, who may do what, keys, grants, an authorisation check relaxed
- a breaking change to a public contract: the client API, the wire, the CLI, a published file format, in a way that older
  readers or writers cannot tolerate
- an irreversible step: something that cannot be undone by reverting the commit
- a change that hides a red: it makes a check pass, skip or be waived where it would otherwise fail

**Medium** — none of the above, and any one of these:

- runtime, reconcile, store or replication behaviour changes, with a revert as the way back
- an additive, optional, negotiated change: a new optional claim or wire field that older readers ignore (with its rollout rule,
  for example daemons before clients), a read-only route, a new local-only mode that is off by default
- the evidence, signing, CI or policy path changes without weakening it
- a workspace-wide toolchain or dependency pin (the flake, the lock files)
- a change too large to read in full, with the unread parts named

**Low** — everything else, which is usually one of:

- tests, fixtures, documentation or release notes only
- production behaviour provably unchanged (a test-only knob, an off-by-default diagnostic, an in-memory-only change)
- a refactor with the same observable behaviour and the tests that prove it

### How to lower a change's risk

An agent iterates on a high-risk change until it is medium or low. The ways, in the order to try them:

1. **Split it.** Land the additive, reversible half now (the new optional field, the reader that tolerates it) and the half that
   changes behaviour afterwards.
2. **Put it behind a flag**, off by default and opt-in per member, so landing the code changes nothing until someone turns it on.
3. **Make it additive.** A new field, route or kind beside the old one, never a change to the old one.
4. **Make it reversible.** A way back that is exercised by a test: a backup or snapshot before a first deletion, a rollback test
   that passes before the removal merges, the first member alone and measured.
5. **Prove the older side tolerates it.** A client test that an unknown status, field or kind is ignored.

A **contract change is scored like everything else**; nobody approves it as such. A breaking change to a public contract is high,
and the agent iterates it down by the ways above (an additive half first, a flag, proof that the older side tolerates it). An additive or negotiated change (an optional field older readers ignore, a
route beside the old one) is medium and queues. The `public_contract` flag exists so the reviewer scores it, not so a person is asked.

If a change truly must stay high, the agent takes it to its **own human**, with the recommendation first and the risk in plain
words. It does not enter the merge queue until a person has answered and the answer is recorded.

What only a person can give — a fact only they know, a credential, or money — goes to that person; everything else is decided by
the change's owner, who takes the reversible or bounded option and records why.

## Findings, and the loop

Each finding is `blocking` or `advisory`.

- **Blocking:** a required check red at the head; a correctness or security bug the reviewer can show; a test that would pass
  without the change; a rule in this document broken; a missing release note; a latent bug that could panic on a hot path or
  sign something wrong, even if no current caller reaches it.
- **Advisory:** a defect that is latent, cosmetic or outside the change (a misplaced comment, an unexplained lock-file
  re-resolution, a fragile test split). The finding is named, marked "not blocking", and given an owner for the follow-up.

The author loops on the review until no blocking finding remains, **at most three rounds**, and then takes the change to its own
human. The loop has rules that keep it honest:

- **Every attempt is recorded and signed**, not only the last one.
- **A GO counts only after a code change that answers the previous findings.** Running the same review again on the same head
  until it passes is not a loop; the verdict's `answers` field says which change answers which finding.
- **A different model family reviews, where one is available**, so the reviewer does not share the author's blind spots.
- **An independent seat re-reviews a random share of merged changes** and the disagreements are counted, so the reviewer itself is
  measured. The **miss rate** (merged changes the independent read finds a blocking problem in) is published daily beside the skip
  and SLO numbers. A rising miss rate **tightens this standard** (a rule added, a risk class raised); it never brings central
  review back.

## Reading only what changed

A review is a cache that is appended to and never redone. Each review of a change keeps a record: the full head, the base, the
per-file patch-id of the change's own diff against the merge base (`git patch-id` over that file's diff), the verdict, and the
files and lines actually read.

- A reviewer loads the record first, computes the per-file patch-ids of the current head, and reads only files whose patch-id is
  not already covered, plus every conflict-resolution hunk and every new commit.
- A **merge of `main`** whose `git show --remerge-diff` is empty carries the earlier verdict to the new head. If the remerge diff
  is not empty, the reviewer reads it.
- A **rebase**, or a conflict resolution that leaves each file's own change equal, is free for the same reason. Generated files
  and appended documentation or release-note fragments are read as a delta too.
- Done by hand, the same read is `git diff OLDHEAD NEWHEAD`, `git log --no-merges` for commits not on `main`, and
  `git show --remerge-diff` for every merge; the patch-id record is how `scripts/review` does it, and either way the verdict says
  what was served from the record.
- A **delta read is enough** when a verdict exists for an ancestor of the head. The delta verdict says what was served from the
  record and what was read.
- A change that was **never reviewed gets a full first read**, whatever its author calls the request.

## Behind main

- A head that merges cleanly is **not refreshed** just because `main` moved: a stable head keeps its signed checks and its
  review. A refresh is made when the head conflicts or goes dirty.
- A refresh is a **plain merge of `main`** into the branch, then a delta read of the merge. Textual conflicts are resolved by
  keeping both sides' intent; generated files are regenerated, never hand-merged; the touched tests run; then the checks sign
  again and one fast-forward push goes out.
- A conflict that needs a **semantic decision** (two different pins of one dependency, two incompatible changes to one rule) is
  not resolved by the refresher. The merge is abandoned, nothing is pushed, and the exact decision is given to the owner.

## The merge queue

- **Up to three** changes may be in the queue at once.
- A change goes in the queue **alone** when it changes `.st/`, a workflow, a ruleset or repository setting, the evidence or
  signing path (`scripts/fractal-test`, `scripts/evidence-*`, `scripts/ci-evidence-*`, `scripts/ci-local`, `scripts/ci-linux`, the lane
  scripts, `scripts/ci-rust-test-notes`), the store's core file, or a workspace-wide toolchain or dependency pin. A reviewer may
  also keep a change alone when it judges it risky (a reconcile or schema change), and says so in the verdict.
- Everything else may **batch** when the changed files do not overlap.
- A change with a **high** risk, a missing signed review, or a signed review at a different head is not queued.
- **Never** merge by hand, rebase, force-push, push to a head that is already in the queue, retry only to confirm queueing, relax a
  required check, or waive an adverse optional check. A change lands only through the merge queue.
- A change that fixes an outage happening now (CI or the queue down, a daemon stall, data at risk) may go first, with the reason in
  the pull request.

## What the review does not do

- It does not run the tests or the build; the signed checks did.
- It does not judge whether the product should do the thing; that is the owner's call.
- It does not wave a contract change through: a public or client contract change is flagged and scored (breaking is high,
  additive or negotiated is medium), like any other risk.
- It does not decide a flake disposition on the evidence path; it holds the change and asks for one.
- It does not approve a pull request from a fork, arm auto-merge, or grant a waiver, retention rule, schema change or rules
  change by assumption.

## What a script cannot do for you

Some of what a good reviewer does is hard to script, and the standard names it so that it is done on purpose: reproducing a shell
semantic trap in six lines; following an input to the code that consumes it; checking that a "skipped" claim is backed by a signed
note; noticing that the proof a change claims is not run by the checks it relies on, and asking for a post-merge run when the only
proof is daily. `scripts/review` asks the agent it runs to do each of these, and a person reading a verdict can check the same
list.
