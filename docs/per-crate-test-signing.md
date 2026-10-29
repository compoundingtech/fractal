# Per-crate signing for the Rust tests (design, step 1)

Goal: a warm `scripts/fractal-test` and the pull-request gate re-run only the crates a change can affect. Today one `linux-gate`
signature covers all ~8,700 tests for one input key (`source_inputs`, docs/st3/gate-evidence.md "inputs"), and `crates` is one declared
path, so any Rust change re-runs everything. This note defines one key and one signature per workspace crate. It changes nothing
by itself.

## 1. The key

`key(C) = source_inputs(paths(C))`: the existing projection (`evidence-verify project`: `git ls-tree` of exactly those paths, then
`write-tree`), so a producer, the observer and the gate compute it from a tree and a path list alone.

`paths(C)` is the union of:

1. **Own and closure directories.** The directory of C and of every crate in its dependency closure. The closure is every
   `path` dependency (normal, dev, build, target-specific, `workspace = true` resolved through `[workspace.dependencies]`), workspace
   member or not, transitively. It is computed with `tomllib` from the manifests *of the tree being keyed* (`git show TREE:path`),
   not with `cargo metadata`, so the gate needs no toolchain and no checkout; a policy test compares it with `cargo metadata` for the
   current tree. A crate whose manifest is the repository root (`fractal-legacy-runner`) contributes its target source dirs
   (`src`, `tests`, `build.rs`, `benches`), never `.`. Because manifests are inside the keyed paths, an edge added or removed
   changes the key.
2. **Global paths** (change them and every crate re-runs): `Cargo.toml`, `Cargo.lock`, **every workspace member's `Cargo.toml`** (the
   tests run on one `--workspace` build, so a feature one member turns on changes a shared dependency's build for all; found in
   skip-ci-sigs-2's read), the `[patch]` paths, `.cargo`, `flake.lock`, the toolchain pin
   (`rust-toolchain` if present, else `flake.nix` and `nix/`), `.config/nextest.toml`, `.config/daily-only.filterset`, and the test-stage
   scripts: `scripts/ci-local`, `ci-linux`, `ci-nextest`, `ci-nextest-flaky`, `fractal_test_process.py`, plus the
   new per-crate runner. Not the whole of `scripts`: that is the largest source of unrelated churn.
3. **Per-crate extras** from a reviewed file, `.st/rust-test-inputs.json`: `{"global": [...], "crates": {"<crate>": {"extra": [paths]}}}`.
   Extras are what the crate's tests read outside the closure: fixtures, `docs/st3/invariants.md`, `evals`, `slo`, `plugins`,
   `claude-channel`, the font under `apps`, scripts a test runs. They start from the compile-time `include_*!` scan and are completed
   by the trace in section 5.

**Fail closed.** A crate with no entry in the file gets `extra: ["."]`, whose key is the whole tree: it re-runs on every change, as
today. A policy test requires an entry for every workspace member, and an unreviewed crate is one line that says `"."`. So the
migration is monotone: shrinking a crate's extras is a reviewed change with its trace attached, never a default.

What a change then costs (workspace of 52 members; "reruns" counts crates whose closure holds the changed one, dev deps included):
a change in a leaf such as `fractal-tui` or `fractal-daemon` re-runs 1 to 5 crates; in `fractal-store` about 17; in `fractal-kernel`
or `fractal-runtime` about 23. Honest limits: `fractal` (1,219 tests, 41-crate closure) re-runs on nearly every non-leaf change, and
a change to `Cargo.lock`, any member's `Cargo.toml` or `.config/nextest.toml` re-runs everything. The saving is the tests of every crate outside the changed crate's dependents (by a source-line count: `fractal-drivers` 1,220, `smallclaims` 937, `fractal-store` 795,
`fractal-reconcile` 620, `fractal-tui` 495), and a no-op re-run (a docs, TypeScript or script-only change) costs nothing.

## 2. Notes and records

One policy check, `rust-tests`, whose input key is per crate. The signed record carries `observed.inputs.crate = C` and
`observed.inputs.source_inputs = key(C)`, `tests_run` and `tests_failed`. Refs: `refs/notes/evidence-input/rust-tests/<key>/<record>`
(the existing layout; the key is a git tree id, so two crates cannot collide, and the gate still requires the signed `crate` to equal the
crate it asked about). Listing is by prefix, so one batched fetch of 52 key prefixes replaces 52 lookups. Every other rule applies
unchanged: signature, trust level, exact command, `lane` binding, `max_age`, latest record for the key decides (a newer failure stops
the reuse), `sample_one_in`.

The wrapped command is pinned by the policy like every other (`nix develop -c bash scripts/ci-local rust-tests`), with the crates under test
passed as `--input crate=C`. A crate with no tests signs `tests_run=0`; a note with `tests_run=0` covers a crate only if the tree has
no test in it (checked by the verifier from the nextest list of the same run), so "ran nothing" is never a pass.

## 3. fractal-test

1. Compute `key(C)` for every member from HEAD's tree (one `ls-tree` per crate; milliseconds).
2. Ask the verifier which keys already have a passing note (local notes and `refs/notes/evidence-input/rust-tests/<key>` from the
   remote, one fetch). The missing set is M.
3. Run **one** `scripts/ci-nextest run -E 'package(=A) or package(=B) ...'` for M, in the throwaway worktree on the kept target. It is
   the same `--workspace` build as CI, so feature unification is the same as in the gate (a `-p C` build would unify differently and
   would be a different measurement). Nextest writes JUnit.
4. Sign each C in M separately: `evidence-wrap --check rust-tests --input crate=C --input tests_run=N -- scripts/ci-rust-test-result --junit J --crate C`,
   a small command that exits 0 iff every test of C in the run's JUnit passed (retries as nextest counts them today) and prints the
   per-crate result, whose digest is the signed output digest. Open question for skip-ci-sigs-2: is a signature over a result derived from
   one shared run acceptable at the `host`/`agent` level, or must each crate be its own `nextest` invocation (simple, but loses cross-crate
   parallelism and pays startup N times)? I recommend the derived result: the host key already signs what its own run measured.
5. Push the notes (`--push`) as today. Failing crates are signed as failures and do not poison other crates.

Interplay with the other `fractal-test` changes: skip-ci-sigs-2's defaults (full suite; isolation-vm and canaries to daily) change `LOCAL`
and the default set; this adds a `rust-tests` entry beside `linux-gate`, which stays for clippy, fleet-compat and the canaries until section 4.
The worktree pool (#3697) and "skip checks whose tree already has a passing signature" compose: the per-crate missing set is the finer form
of the same skip.

## 4. Gate, clippy, fleet-compat

- **First gate version (pull_request only):** the test stage (`linux-tests` and shards, `linux-test-build`) is skipped when
  every member has a passing, trusted note for its key; otherwise it runs as today. The merge queue runs everything; `sample_one_in` stays
  5, and a sampled run compares per crate. The member list and closure come from the head tree's manifests via the base's script
  (policy code always comes from the base). A new crate has no note, so it runs.
- **Later:** run only the missing crates in the shards (the filterset above), with the build archive built for that set.
- **Clippy:** stays whole in v1 (its ratchet is base-bound and `cargo clippy` is a check build, so it is the cheaper half). Per-crate
  clippy (`cargo clippy -p C`) is possible later with the same keys plus the baseline file and the base sha as inputs; not before the
  tests land. **fleet-compat** and the mail canaries stay whole, keyed as today.

## 5. The stale-pass risk and how it is checked

The dangerous direction is an under-declared input: a crate's tests read something outside `paths(C)`, the thing changes, the key does not,
and a stale pass is reused. Four controls, in order of when they bite:

1. **Static:** the committed-policy test that checks `include_str!/include_bytes!/include!` against declared inputs becomes per-crate (the
   path must lie in the crate's own paths); a second scan flags `CARGO_MANIFEST_DIR` joins containing `../` and `env::current_dir`/`git`
   reads of the tree; `test_no_rust_source_lints_the_whole_tree` stays.
2. **Trace:** `scripts/evidence-lane-trace` already reports repository files a run opens that no list covers. A daily job runs each crate's
   nextest under it (strace, one crate at a time) and fails when a crate opens a repository path outside `paths(C)` (this is also how the
   first extras are found, and the only way a crate leaves `"."`). The accepted trace is committed beside the extras and a policy test
   requires declared ⊇ traced.
3. **Sampling and the queue:** one run in five really runs even with a matching note, and the merge queue always runs everything, so a stale pass
   is caught at the latest before merge. A sampled or queue failure of a crate whose note was reused is logged by key as a divergence; the target
   is zero, and any divergence moves that crate back to `"."` until the extra is found.
4. **Not path-shaped inputs:** time, the network, ports and host binaries are not in any key, as today; the toolchain and `flake.lock` are
   global. Features are the same as CI's because the run is the workspace build.

## 6. Delivery

1. This note (PR 1, docs only).
2. `scripts/rust-test-keys` (closure, global, extras, fail-closed default, per-crate policy tests, `.st/rust-test-inputs.json` all `"."`) plus the
   `fractal-test` change: compute keys, run the missing set, sign per crate, push. Behaviour-neutral in CI.
3. Verifier + policy (`rust-tests` check, `--crates` projection, batched prefix fetch) + gate (test stage skip, pull_request only, shadow
   first, then enforced). Needs skip-ci-sigs-2 (trust) and skip-ci-lead-2 (gate).
4. Shrink the extras crate by crate with traces; optional per-crate clippy and run-only-the-missing-crates in the shards.

## 7. As built in step 2

- `scripts/rust-test-keys` (keys, `--paths`, `--json`; reads `.st/rust-test-inputs.json` from the base with `--inputs-from`) and
  `scripts/rust-test-inputs-scan` (the textual first pass of the extras: `--write` produced the committed file, a policy test fails when a
  crate's scan names a path its entry does not cover). `.st/rust-test-inputs.json` starts from that scan; the daily trace of section 5
  is what lets a crate's extras shrink or grow after review. Test file: `scripts/rust-test-keys-test`.
- `scripts/fractal-test --check rust-tests` (in the default suite since step 5a, after `linux-gate`; see section 10): computes the keys,
  fetches the per-crate index refs in one fetch, asks the verifier (`evidence-verify --crate-key CRATE=KEY`) for each crate, runs ONE
  `scripts/ci-local tests` for the missing crates (`CI_TEST_PACKAGES`, `CI_TEST_JUNIT`; the workspace build, so features unify as in CI), and
  signs each crate with `scripts/evidence-wrap --check rust-tests` over `scripts/ci-rust-test-result`, which reads that crate's part of the
  JUnit report. The record signs `crate`, `source_inputs`, `tests_run`, `test_list_sha256` (the exact test list), `ignored_list_sha256`, `filterset` (the expression, or the crate count and its sha256 when it is longer than the signer's 500-character input limit) and `run_command_sha256`.
  A run that did not complete (build failure, timeout, a nextest exit other than 0, 4 or 100, no report) signs nothing. A failing crate is
  signed as a failure and the others are still signed. `--status` names the crates it would run.
- `evidence-wrap` also indexes a record that signs `source_inputs` under `refs/notes/evidence-input/<check>/<key>/<record>` and
  `refs/tags/evidence/input/<check>/<key>/<record>` and pushes them with `--push`. Until now only CI's signer made those. Whether an agent
  seat's identity may create them on the remote is a repository rule that has not been confirmed (as for `refs/tags/evidence/*`).
- Not in this step: the skip-policy entry, the lane definition and the gate job (step 3, with skip-ci-sigs-2 and skip-ci-lead-2). Until
  the policy names the check at the base, `fractal-test` judges coverage by signature, crate and key alone (a local convenience), and the notes
  carry no lane digest, so step 3's notes are signed again.

## 8. As built in step 3 (policy, lane, verifier)

- `.st/evidence-skip-policy.json`: `rust-tests` (`pull_request` only, `sample_one_in` 1, no `inputs`: the keys are per crate; shadow with
  `enforce` false until 2026-10-28, enforced since: a pull request skips rust-tests on per-crate notes). Fleet level: the wrapper command; host and agent levels: `python3 scripts/ci-rust-test-result`, lane-bound, requiring `crate`,
  `source_inputs`, `tests_run`, `tests_ignored`, `test_list_sha256`, `ignored_list_sha256`, `filterset` and `run_command_sha256`.
- `.st/required-lanes.json` (since narrowed to what the lane's commands execute plus its verdict data, without `.st/rust-test-inputs.json`,
  which only the base's copy keys a note: docs/st3/gate-evidence.md, "Definition paths are what the lane executes, plus its verdict data"): the `rust-tests` lane (the test-stage scripts through `scripts`, `.config/nextest.toml`, the filterset, the
  flake files, and `.st`, which holds `rust-test-inputs.json`), so a tree that edits how keys are computed never skips on a local note. `Cargo.toml` and `Cargo.lock` are deliberately not definition paths: they are in every crate's key, so a change re-runs every crate, and as definition paths they would make every lockfile change an unsignable lane edit. `tree_inputs` is informational (the policy consistency test and the lane trace read it); nothing indexes or verifies rust-tests notes by it.
- `scripts/evidence-verify --check rust-tests --members --policy-from BASE --repository R ...`: the member list and each key come from the
  tree's manifests with the `rust-test-keys` that sits next to the verifier and `.st/rust-test-inputs.json` at the base; the check is satisfied
  only when EVERY member has a satisfying note for its own key (the usual rules per note, with the crate and key), else `crate_uncovered` with
  a `missing` map of crate to reason. A satisfied result carries the oldest note's time, the weakest signer level, the sampling seed (a hash of
  the member keys), and `crates` (crate to record). `--members --project` prints `{crates: {name: key}}` for the gate's one batched fetch.
  The gate must copy the base's `evidence-verify` and `rust-test-keys` side by side.
- `PROTECTED` in the gate now includes `scripts/rust-test-keys`, `.st/rust-test-inputs.json` and `scripts/ci-rust-test-result`.
- A note counts for a crate only when its run ran at least one test (`tests_run` > 0) once a policy judges it: a run whose selection matched
  nothing (nextest exit 4, or a filter that no longer names the crate) signs `tests_run=0` and passes, and that must not stand for a crate.
  A crate that really has no test is listed under `no_tests` in `.st/rust-test-inputs.json` at the accepted base (reviewed; a tree cannot list
  its own crate, and a list that is not an array of names is a verifier `error`, never an empty allowance). `fractal-test` names the crates that
  ran no test so the list can be reviewed. The same rule applies to `--crate-key` when `--policy-from` is given; without a policy a note
  counts by signature, crate and key alone, as before.
- The verifier keys a tree only with the accepted base's `rust-test-keys` and `.st/rust-test-inputs.json`: a base without them is an `error`
  (`--members` refuses; the gate reports it unavailable), never the tree's own copy. `fractal-test` falls back to the tree's own tool only while
  the base's policy does not name `rust-tests` (the change that introduces them is judged by itself); once it does, a base without them is a
  refusal there too.
- The gate job itself (skip the test stage on `evidence-gate-rust-tests`, pull_request only) is skip-ci-lead-2's part of step 3.

Measured with the committed extras over the last 57 first-parent merges of main (2026-10-23): the median merge changes the key of 5 of 52
crates; 29 merges change at most 5, 5 change 6 to 20, 14 change 21 to 49, and 9 change 50 or more (global paths: a manifest, the lock, the
stage scripts). So about half the merges re-run a handful of crates and one in six re-runs everything.

## 8a. As built in step 3 (the gate job, shadow)

- `evidence-gate-rust-tests` is one of the evidence gate jobs (`evidenceGateChecks` in `.github/workflows/fleet.yml.genie.ts`), and the
  aggregate note job waits for it as for the others. With the policy entry in shadow (`enforce` false) it decides nothing: it reports a
  decision line on every run, and no job skips on it yet.
- `scripts/ci-evidence-gate`, for a check in `MEMBER_CHECKS` (`rust-tests`):
  - copies the base's `scripts/rust-test-keys` beside the base's verifier;
  - fetches the tree's `Cargo.toml` blobs and the base's `.st/rust-test-inputs.json` that the clone lacks, in ONE fetch by object id. A
    head tree's clone has trees but no blobs, and the verifier reads them without fetching;
  - fetches every member's note refs in ONE fetch built from `--members --project`;
  - asks `--members`.
  A base whose verifier lacks `--members`, or that has no `scripts/rust-test-keys`, makes the outcome `unavailable`, and the check runs.
- The decision line adds `members` (workspace members), `missing` (how many have no satisfying note), `manifests_fetch` and `members_fetch`.
- The evidence log's view (`log`, `log_head`, `log_age_s`, `log_missing`) covers every member at once: the log head is judged once, then
  each member's own key file in the log. `missing` when the log lists a record under any member's key that is not here, else
  `unlogged_pass` when some member's chosen note is not listed yet, else `ok`. Without it every line said `log=-`, and the flip trigger's
  24 h log window could never start.
- The test jobs are wired to it: `linux-tests` and its shards (and through them `linux-test-build`) skip when linux-gate's note OR every
  crate's per-crate note covers the tree; clippy and fleet compatibility still need linux-gate's own note. The linux-gate verdict accepts
  a skipped stage only when one of those gates decided skip and that stage's own rule says skip. Nothing changes while the rust-tests entry is
  shadow (its gate never says skip). The enforce step was then the policy flip alone (`enforce: true`, `events: ["pull_request"]`,
  every run (`sample_one_in` 1), merge queue unchanged, and the wrapper kind's `trust_levels` narrowed to host, agent and queue), on its
  own trigger; before it, scripts/evidence-flip-ready counts the jobs that read rust-tests' skip as its members, so a red they would hide
  is seen.

## 9. Signing time (measured 2026-10-24)

The first full run on main's tree signed 52 crates in 671 s (07:52:51Z to 08:04:02Z), as long as the tests (nextest 941.6 s): each
signature was a serial `evidence-wrap` of 11.8 s (the dev shell's `nix eval` 2.6 s, the signer 1.4 s, the verifier's check of the new
record 6.8 s) plus its own push. `fractal-test` now evaluates the digest once, signs the first crate alone (the signer creates its scratch
repository when it is missing) and the rest `ST_TEST_SIGN_JOBS` (default 8) at a time, and pushes every record's four refs in one push.
Each record's content and signature are as before. Fifty-two signatures from a saved report took 74.7 s on this host, before the push.
The four workspace crates that ran no test in that run (demo-resolver-wasm, fractal-build-stamp, fractal-github-issue-component,
fractal-github-pr-component) are listed under `no_tests` in `.st/rust-test-inputs.json`.
A full run once sat 30 minutes in the megarepo member step (`mr apply`, whose `git fetch --tags --prune` was blocked writing to its parent while
the network was fine). The step now has a limit (`ST_TEST_MEMBERS_LIMIT`, default 1200 s), writes its output to a file, and is killed with
everything it started when the limit passes; the run then reports "NOT RUN" with the reason. The `ST_TEST_*` settings are read before the seat
environment scrub, which would otherwise remove them.

## 10. Step 5a: rust-tests in the default suite, signed from linux-gate's own run

While the per-crate gate is in shadow, `linux-gate` still runs the tests stage. Running the tests a second time for `rust-tests` would double the
test time of every push, so when both are selected (the default) `fractal-test` asks linux-gate's run for nextest's JUnit report (`CI_TEST_JUNIT`, an
absolute path in its scratch: the same `--workspace` build and the whole selection, `conversation_model` included) and `rust-tests` then signs the
crates that have no passing note for their key from that report, running nothing. Each such record says `filterset=workspace` and carries the
sha256 of linux-gate's command as `run_command_sha256`, so it is told apart from a filtered run. When linux-gate did not run (a lane edit) or left no
whole report, `rust-tests` runs the crates it lacks itself, as before. When the enforce step has landed (linux-gate's test jobs skip on
`evidence-gate-rust-tests`), linux-gate's local command shrinks to clippy and fleet-compat and `rust-tests` is the only run of the tests.

## 11. Seeding main's tree: `fractal-test --seed`

A merge that moves a crate's key, or the lane's definition (anything under `scripts/` or `.st`), leaves main's tree with crates that have no note, so
the gate asks for them again on the next pull request. `scripts/fractal-test --seed [REF]` is the entry point a merge-queue signer runs after such a merge,
from any checkout and whatever HEAD is: it signs the per-crate `rust-tests` notes of REF's tree (default: the base, `origin/main`, fetched first)
for the crates that lack one, runs only those crates, and pushes the notes (to `origin` unless `--push` says otherwise). It takes no other check; it
exits 0 when every crate is covered or signed, and `--crate NAME` narrows it. Measured: a full seeding of 52 crates is about 12 minutes on this host
(prep 158 s, nextest 480 s, signing 53 s); after a merge that touched 2 crates it runs only those 2.

For a separate publisher, `scripts/fractal-test --seed [REF] --out DIR` exports `<record>.pack` and `<record>.json` for every selected
crate, including crates already covered locally, and does not implicitly push to `origin`. The same `--out DIR` works with
`--check rust-tests` and with the full suite (including linux-gate's aggregate and per-crate records from its shared nextest report).
The output directory is resolved against the caller's working directory, not the clean worktree; nested relative directories work.
`--out` is incompatible with `--push`. Export mode deliberately reruns the selection to produce a complete set rather than copying
existing notes. It does not import the packs, so success means passing checks and exported packs, not verified coverage: publication
and verification belong to the publisher. Failures still export failure records and fail the run; lane-edit refusals remain unsigned.


## 12. Verification and push time (measured 2026-10-24)

Two warm full runs on hetz2 took 935 s and 1116 s against the 12-minute budget, with 123 to 270 s outside every check row. Where it went, and what
changed:

- **Asking the verifier about 52 crates.** `fractal-test` ran `evidence-verify --crate-key` once per crate: 52 processes, each loading all of the tree's
  52 records (2 880 `load` calls, two git runs each). The same 52 crates in one `evidence-verify --members` run, with each record read once per
  process (`load` is cached: a record is content-addressed) and the members checked 8 at a time, give the identical answer. Measured on one input
  (tree e9fd13c5b808, policy at main): per-crate processes 110.0 s, `--members` before the cache 105.9 s, with the cache 32.8 s, with the cache and
  the members side by side 8.6 s. `fractal-test` now asks once (`--members`, falling back to the per-crate loop only when the verifier cannot answer).
  The CI gate's `--members` call gets the same speed-up.
- **The row's time.** The `rust-tests` row used to show only the tests (0 s when signed from linux-gate's report). It now shows all of its time, with
  `[keys, verify, tests, sign, push]` in the detail, and a `setup` row shows the fetch, the clean tree and the megarepo members before any check.
- **The push.** Records are pushed in chunks of 40 refs with git's porcelain output, and a ref that failed is retried (3 tries, 2 s and 4 s apart); the
  error names the refs left and git's reason. One 76-ref push once reported only `(failed)` for a ref that pushed alone afterwards.
- **What did not change.** Signing 52 crates still costs about 48 s: the post-sign check inside `evidence-wrap` (6.8 s per record, in its own process)
  is the base tool's, and a change to it is its own trust review.

## 13. The check after signing is the crate's own (2026-10-25)

`evidence-wrap` verifies each new record before it pushes it, by asking the verifier about the check on the record's tree. For a per-crate check
that asked for the tree's newest `rust-tests` record whatever its crate, so one crate's failed record on the tree made every record signed after it
come back "did not verify locally" (50 of 52 crates on hetz2 at feefaa9c95 when a mailbox test failed). A record that signs a `crate` and a
`source_inputs` key is now verified with `--crate-key CRATE=KEY`: as that crate's, the same rule the gate and `fractal-test` use. Nothing about the record
or the verifier's rules changes.


## 14. CI signs per-crate notes (2026-10-25)

Until now only a local `scripts/fractal-test` push signed per-crate `rust-tests` notes, so a pull request whose crates were unchanged found a note
only when someone had run it. CI now signs them too, with its own `fleet/ci` key, from the test shards' own nextest run. These notes don't
skip rust-tests for a pull request: since the check is enforced, the wrapper kind's `trust_levels` are host, agent and queue, so a pull
request skips only on a note fractal-test signed on one of our hosts or an agent signed. CI's notes are still produced, published and
verified.

- **Where.** The test shards (`linux-tests`, `linux-tests-shard-2/3/4`) write nextest's JUnit report (`CI_TEST_JUNIT`) and
  `scripts/ci-rust-test-notes describe` (job, partition, attempt, how nextest ended, the dev shell digest, the system) and upload both as
  `rust-test-junit-<job>-<attempt>`. A separate job, `evidence-note-rust-tests`, signs from them. The report is written on every run (so a pull
  request's own test jobs show that nextest writes it), but it is described and uploaded, and the job runs, only for the merge queue,
  push to main and manual runs on main; on a pull request the job does not run and the script refuses one (`guard`). The job reaches the key through the `evidence-signing` environment like the other note jobs, takes every script from the
  base commit, and is fail-soft: no required check needs it and every step continues on error.
- **Nothing from partial data.** `ci-rust-test-notes plan` runs before the key exists. It signs nothing for any crate when either gate the
  test jobs read (`evidence-gate-linux-gate`, `evidence-gate-rust-tests`) decided to skip, when any shard of this event (four in a merge group,
  two otherwise, the merge-only shards then skipped) did not end `success`, when a shard's report of THIS attempt is missing, cut short or
  describes another partition, attempt or outcome, when the shards ran on different dev shells or systems, or when one test case appears in two
  shards. A re-run of only the failed jobs therefore signs nothing (the other shards' reports are the previous attempt's).
- **Per crate.** The reports are joined into one, so a crate split across shards is judged once, from all of them. A crate is signed only when
  every one of its test cases passed on the first try: a failure, an error, or a test that passed only on a retry (flaky) gives it no note,
  and so does a failure or flaky test of that crate in an earlier attempt's report of the same run. `scripts/ci-rust-test-result` still
  counts flaky as passed (its output is the recorded command's), but `scripts/fractal-test` holds its local per-crate signing to the same rule: a
  crate with a test that passed only on a retry is left unsigned (exit 4, "left for CI"), so CI runs it. linux-gate's whole-tree note still
  signs, as the gate counts a retried pass, and records those tests as the input `flaky_tests` (`BINARY TEST` lines from
  `scripts/ci-nextest-flaky` under `CI_TEST_FLAKY`; `none` when there are none). A crate that ran no test is signed only when the
  base lists it under `no_tests`, the rule the verifier applies to every note. A crate that does not qualify gets no record at all.
- **The same record as fractal-test's.** The key is computed by the same code path as `scripts/fractal-test`'s `crate_keys` (the base's
  `scripts/rust-test-keys` over the tested tree, with the base's inputs file and the base's verifier projection; a test compares both on a
  fixture and on this repository's tree). Each record is `evidence-wrap --aggregate` with the command `python3 scripts/ci-rust-test-result`,
  the digest of that command's output over the crate's part of the joined report, and the inputs `crate`, `source_inputs`, `tests_run`,
  `tests_ignored`, `test_list_sha256`, `ignored_list_sha256`, `filterset=workspace`, `run_command_sha256` (the tests stage command and the
  shard partitions) and the shards' dev shell digest as `toolchain_sha256`, plus the run's id, attempt, event, partitions and shard jobs.
  `evidence-verify --members` verifies it as it does fractal-test's (the wrapper rule, no lane) and reports its level as fleet, which the
  enforced policy does not trust for a pull request (`untrusted_level`).
- **Published like the other CI notes.** `evidence-publish --checks rust-tests` files each record under the tree's tag and note and the
  input-keyed index refs (`refs/notes/evidence-input/rust-tests/<key>/<record>`, `refs/tags/evidence/input/rust-tests/<key>/<record>`) and
  pushes them. A signature that fails is reported in the job summary and the others are still published.
- Tests: `scripts/ci-rust-test-notes-test` (in `scripts/ci-script-tests`).
