# Per-crate signing for the Rust tests (design, step 1)

Goal: a warm `scripts/st-test` and the pull-request gate re-run only the crates a change can affect. Today one `linux-gate`
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
   scripts: `scripts/ci-local`, `ci-linux`, `ci-nextest`, `ci-nextest-targets`, `ci-nextest-flaky`, `st3_test_process.py`, plus the
   new per-crate runner. Not the whole of `scripts`: that is the largest source of unrelated churn.
3. **Per-crate extras** from a reviewed file, `.st/rust-test-inputs.json`: `{"global": [...], "crates": {"<crate>": {"extra": [paths]}}}`.
   Extras are what the crate's tests read outside the closure: fixtures, `docs/st2/invariants.md`, `evals`, `slo`, `plugins`,
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

## 3. st-test

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

Interplay with the other `st-test` changes: skip-ci-sigs-2's defaults (full suite; isolation-vm and canaries to daily) change `LOCAL`
and `HEAVY`; this adds a `rust-tests` entry beside `linux-gate`, which stays for clippy, fleet-compat and the canaries until section 4.
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
   `st-test` change: compute keys, run the missing set, sign per crate, push. Behaviour-neutral in CI.
3. Verifier + policy (`rust-tests` check, `--crates` projection, batched prefix fetch) + gate (test stage skip, pull_request only, shadow
   first, then enforced). Needs skip-ci-sigs-2 (trust) and skip-ci-lead-2 (gate).
4. Shrink the extras crate by crate with traces; optional per-crate clippy and run-only-the-missing-crates in the shards.

## 7. As built in step 2

- `scripts/rust-test-keys` (keys, `--paths`, `--json`; reads `.st/rust-test-inputs.json` from the base with `--inputs-from`) and
  `scripts/rust-test-inputs-scan` (the textual first pass of the extras: `--write` produced the committed file, a policy test fails when a
  crate's scan names a path its entry does not cover). `.st/rust-test-inputs.json` starts from that scan; the daily trace of section 5
  is what lets a crate's extras shrink or grow after review. Test file: `scripts/rust-test-keys-test`.
- `scripts/st-test --check rust-tests` (opt-in; `--all` still signs `linux-gate` until the gate accepts per-crate notes): computes the keys,
  fetches the per-crate index refs in one fetch, asks the verifier (`evidence-verify --crate-key CRATE=KEY`) for each crate, runs ONE
  `scripts/ci-local tests` for the missing crates (`CI_TEST_PACKAGES`, `CI_TEST_JUNIT`; the workspace build, so features unify as in CI), and
  signs each crate with `scripts/evidence-wrap --check rust-tests` over `scripts/ci-rust-test-result`, which reads that crate's part of the
  JUnit report. The record signs `crate`, `source_inputs`, `tests_run`, `test_list_sha256` (the exact test list), `ignored_list_sha256`, `filterset` and `run_command_sha256`.
  A run that did not complete (build failure, timeout, a nextest exit other than 0, 4 or 100, no report) signs nothing. A failing crate is
  signed as a failure and the others are still signed. `--status` names the crates it would run.
- `evidence-wrap` also indexes a record that signs `source_inputs` under `refs/notes/evidence-input/<check>/<key>/<record>` and
  `refs/tags/evidence/input/<check>/<key>/<record>` and pushes them with `--push`. Until now only CI's signer made those. Whether an agent
  seat's identity may create them on the remote is a repository rule that has not been confirmed (as for `refs/tags/evidence/*`).
- Not in this step: the skip-policy entry, the lane definition and the gate job (step 3, with skip-ci-sigs-2 and skip-ci-lead-2). Until
  the policy names the check at the base, `st-test` judges coverage by signature, crate and key alone (a local convenience), and the notes
  carry no lane digest, so step 3's notes are signed again.

## 8. As built in step 3 (policy, lane, verifier)

- `.st/evidence-skip-policy.json`: `rust-tests` (shadow: `enforce` false, `pull_request` only, `sample_one_in` 5, no `inputs`: the keys are
  per crate). Fleet level: the wrapper command; host and agent levels: `python3 scripts/ci-rust-test-result`, lane-bound, requiring `crate`,
  `source_inputs`, `tests_run`, `tests_ignored`, `test_list_sha256`, `ignored_list_sha256`, `filterset` and `run_command_sha256`.
- `.st/required-lanes.json`: the `rust-tests` lane (the test-stage scripts through `scripts`, `.config/nextest.toml`, the filterset, the
  flake files, and `.st`, which holds `rust-test-inputs.json`), so a tree that edits how keys are computed never skips on a local note. `Cargo.toml` and `Cargo.lock` are deliberately not definition paths: they are in every crate's key, so a change re-runs every crate, and as definition paths they would make every lockfile change an unsignable lane edit. `tree_inputs` is informational (the policy consistency test and the lane trace read it); nothing indexes or verifies rust-tests notes by it.
- `scripts/evidence-verify --check rust-tests --members --policy-from BASE --repository R ...`: the member list and each key come from the
  tree's manifests with the `rust-test-keys` that sits next to the verifier and `.st/rust-test-inputs.json` at the base; the check is satisfied
  only when EVERY member has a satisfying note for its own key (the usual rules per note, with the crate and key), else `crate_uncovered` with
  a `missing` map of crate to reason. A satisfied result carries the oldest note's time, the weakest signer level, the sampling seed (a hash of
  the member keys), and `crates` (crate to record). `--members --project` prints `{crates: {name: key}}` for the gate's one batched fetch.
  The gate must copy the base's `evidence-verify` and `rust-test-keys` side by side.
- `PROTECTED` in the gate now includes `scripts/rust-test-keys`, `.st/rust-test-inputs.json` and `scripts/ci-rust-test-result`.
- The gate job itself (skip the test stage on `evidence-gate-rust-tests`, pull_request only) is skip-ci-lead-2's part of step 3.

Measured with the committed extras over the last 57 first-parent merges of main (2026-10-23): the median merge changes the key of 5 of 52
crates; 29 merges change at most 5, 5 change 6 to 20, 14 change 21 to 49, and 9 change 50 or more (global paths: a manifest, the lock, the
stage scripts). So about half the merges re-run a handful of crates and one in six re-runs everything.
