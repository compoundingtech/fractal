# Keyed clippy and fleet-compat (design)

Status: 2026-10-30. Mission `fleet/factory/no-whole-tree`, steps `clippy-keyed` and `fleet-compat-keyed`. PR 1 (#4178, shadow)
implemented sections 1 and 4 with linux-clippy as an opt-in local check; the fleet-compat PR implements section 3 the same way. PR 2 makes
linux-clippy a default local check, signed from linux-gate's own clippy stage, and turns its dep-info check into a failure (below);
linux-fleet-compat stays opt-in and warns. The linux-gate shrink waits for the flip. Factory approved the design on 2026-10-26 and PR 2's
plan on 2026-10-29.

## The problem

Clippy and fleet compatibility are covered today only by linux-gate's whole-check note. That note's `inputs` (27 paths in
`.st/evidence-skip-policy.json`) include `docs/st2`, `docs/st3`, `docs/vrs`, `evals`, `.github` and `scripts`, because the tests
read them. So a docs-only pull request changes linux-gate's input key, and its clippy stage runs again even though no file clippy
reads has changed. Measured over 32 local runs, clippy's stage has a mean of 1975 s and a median of 1541 s, including its
build-queue wait. On a warm target its own work is about 390 s (`docs/st3/gate-evidence.md`).

The aim is a note for clippy keyed by exactly what clippy reads, so a tree that leaves those files alone reuses an existing note.
The same applies to fleet-compat, with its own key.

## What clippy reads

The `clippy` stage is `clippy_and_codegen` in `scripts/ci-linux`:

- `cargo clippy --workspace --all-targets --locked --message-format=json`, piped through `scripts/clippy_ratchet.py --render`;
- `cargo clippy -p fractal-conversation-ui --no-default-features --all-targets --locked`;
- `python3 scripts/clippy_ratchet.py --logs DIR --base-ref $CLIPPY_BASE_SHA`;
- `cargo run --locked -p fractal-client-codegen -- --check`;
- before those, the stage's self-tests and the guards every ci-linux stage runs.

Its inputs are therefore:

| group | paths |
|---|---|
| Rust sources and manifests | every workspace member's directory (`crates/`, the root package's `src`, `tests`, `benches`, `build.rs`), `Cargo.toml`, `Cargo.lock`, `.cargo/` |
| toolchain | `flake.lock`, `flake.nix` (nixpkgs' cargo, rustc and clippy in `devShells.default`; there is no `rust-toolchain` file) |
| lint config | none in files today: no `clippy.toml`, no `[lints]` table, no crate-level `deny`/`warn`. The key must still include `clippy.toml`, `.clippy.toml` and every manifest (already included), so that adding one changes the key |
| verdict data | `.github/clippy-baseline.json`, `.github/clippy-splits.json` |
| the base | only through the base's copy of `.github/clippy-baseline.json` (`baseline_at_ref`). The ratchet fails if the tree's baseline rose above the base's |
| codegen check | `docs/st3/client-v0/schemas/`, `clients/swift/FractalClient/Sources/FractalClient/`, `clients/typescript/fractal-client/` (its `*.generated.ts`) |
| scripts the stage executes | `scripts/ci-linux`, `scripts/ci-local`, `scripts/clippy_ratchet.py`, `scripts/ci-rustc-wrapper`, the self-tests (`ci-build-snapshot-test`, `build-stamp-watch-test`, `clippy-ratchet-test`, `crate-cut-test`) and the guards (`ci-cargo-tools-env-test`, `ci-rustc-wrapper-test`, `sccache-evidence-test`, `check-st-boundary`, `check-st-boundary-test`, `st3-boot-canaries-offers-test`) |

The scripts are listed one by one, not as `scripts/`. A change to an unrelated script must not move clippy's key; that is the point.

## The design

### 1. A check of its own: `linux-clippy`

CI already signs the clippy stage's note under the check name `linux-clippy` (`fleet.yml`, artifact `evidence-note-linux-clippy`), and
publishing already allows it. The change:

- **Policy.** `.st/evidence-skip-policy.json` gets a `linux-clippy` entry with `inputs`: the paths above. It starts with
  `enforce: false` (shadow). `add_source_inputs` in `scripts/ci-evidence-signing` then keys CI's stage note as soon as the base's
  policy declares inputs, and `evidence-wrap` indexes it under `refs/notes/evidence-input/linux-clippy/<key>/`.
- **The base, as a digest, not a commit.** `clippy_base_sha` changes with every merge, so a note keyed on it is never reused.
  What the verdict depends on is the content of the base's baseline file. The policy's `base_inputs` says so:
  `{"clippy_base_baseline_sha256": ".github/clippy-baseline.json"}`. The verifier computes the run's value itself, the sha256 of that
  file at the accepted base (`--policy-from`), or `absent`, and a note covers the run only when it recorded the same value. No caller
  passes it, so none can forge it. A signer reads the value from `evidence-verify --project` (`base_inputs`); CI's clippy job takes
  it at `BASE_INPUTS_SHA`, the base its ratchet compared against. A job that compared against no base records none, so its note
  covers no run. The baseline changes rarely, so notes stay reusable across many merges; when it does change, every clippy note is
  measured again, as it must be.
- **Gate and workflow.** `linux-clippy` joins `evidenceGateChecks` in `.github/workflows/fleet.yml.genie.ts`. Its job skips on
  linux-gate's skip OR its own (`jobSkips['linux-clippy']`, as `linux-tests` already does with rust-tests). linux-gate's verdict
  already accepts a member that its own gate skipped. `scripts/evidence-flip-ready`'s `skipped_members` learns the new skip.
- **st-test.** `linux-clippy` is a default check (PR 2), run right after linux-gate. Its recorded command is
  `python3 scripts/ci-clippy-result`, which reads a clippy stage's report, as rust-tests records `ci-rust-test-result`. The stage runs
  once:
  - When linux-gate runs in the same invocation, its clippy stage writes the report (`CI_CLIPPY_REPORT`: the cargo messages, the
    metadata, and `passed` only when the whole stage passed), and linux-clippy signs from it.
  - Otherwise (linux-gate already signed, a lane edit, or not selected) st-test runs the stage alone (`RUN_CLIPPY`,
    `scripts/ci-local clippy`) and signs from its report.

  The note records which run made the report (`run_command_sha256`), the base inputs and the input key (`source_inputs`) the base's
  verifier reports, so a tree with the same key reuses it, as it reuses CI's. The caller's command stays
  `scripts/st-test --all --push origin`: the keys are computed inside st-test and CI, never by the caller.
- **The linux-gate shrink waits for the flip.** linux-gate keeps its clippy stage while linux-clippy is in shadow. The policy pins
  linux-gate's local command, so a linux-gate run without the stage would sign no linux-gate note, and a shadow linux-clippy note skips
  nothing: CI would run clippy again. After the flip, linux-gate's command can drop the stage when linux-clippy covers the tree, with
  a policy change that lets its note cover that run; it is done together with rust-tests' shrink (`docs/per-crate-test-signing.md`
  section 10). A test (`KeyedClippy` in `scripts/fractal-test-test`) fails while linux-clippy is in shadow if linux-gate's command or
  its policy commands lose the clippy stage.

### 2. Per crate: not in v1

Clippy runs as one workspace invocation, and the ratchet counts per (crate, lint) against one baseline. Splitting it into
`cargo clippy -p C` per crate would re-check shared dependencies once per invocation, and the ratchet would need a per-crate
baseline. The gain is small next to the whole-workspace key, which already lets every PR that touches no Rust, lint or toolchain
file reuse the note. Per-crate clippy is worth measuring once keyed clippy is trusted: if most Rust PRs touch one or two leaf
crates, a per-crate key (the rust-tests closure plus the baseline rows for that crate) would cut those too.
`docs/per-crate-test-signing.md` section 4 keeps that option open.

### 3. Fleet-compat, the same way

The fleet-compat stage (`fleet_compat` in `scripts/ci-linux`) runs one ignored test of crate `fractal`,
`fleet::previous_release_replicates_with_new_members_and_preserves_history_on_upgrade`, with `FRACTAL_COMPAT_BIN` set to the published
binary that `.github/fleet-compat-baseline.json` pins (`scripts/compat-baseline fleet` fetches and checks it). It reads no file at the
base, so it has no `base_inputs` and is not base-bound in st-test. Its check is `linux-fleet-compat` (shadow, `enforce` false), and its
`inputs`, which are also the lane's `tree_inputs`, are:

- crate `fractal`'s dependency closure as `scripts/rust-test-keys` walks it: the directory of every crate it depends on and the
  manifest of every workspace member, but not `fractal`'s `extra` paths in `.st/rust-test-inputs.json` (what the rest of its tests
  read at run time) and not the root package `fractal-legacy-runner`, which it does not depend on;
- every file a `#[path = ...]` in those crates compiles in from outside them (`crates/pty-launch-environment.rs`,
  `scripts/ci-test-paths.rs`);
- the files the workspace's compiles read from outside the crates (`include_str!` of docs, fixtures, `wit/`, `slo/`, `plugins/`,
  `examples/`), as linux-clippy's dep-info found them, without clippy's codegen and lint data;
- the toolchain (`.cargo`, `Cargo.toml`, `Cargo.lock`, `flake.nix`, `flake.lock`);
- `.github/fleet-compat-baseline.json`, its verdict data (`VERDICT_DATA` in `scripts/lane-definition-closure`). The messaging baseline
  is not an input: no messaging test runs in this stage;
- what the stage executes (`scripts/lane-definition-closure --lane linux-fleet-compat --write`).

Its lane is `scripts/ci-fleet-compat-keyed`, which sets `CI_KEYED_FLEET_COMPAT_POLICY` (CI's linux-fleet-compat job sets it too). With
it, the stage first builds the test with `--no-run --message-format=json` and runs `scripts/dep-info-check --check linux-fleet-compat
--jsonl 'fleet-compat-*.jsonl' --warn` (clippy's warned the same way until PR 2); linux-gate runs the stage without it and never depends on the policy file. The
first measurement (2026-10-26, a warm target) read 474 units and 1035 files, and found one uncovered, `crates/pty-launch-environment.rs`,
which is why `#[path]` targets are in the key. The job skips on linux-gate's gate or its own, as clippy's does. st-test signs it on
request (`--check linux-fleet-compat`), not with `--all`, until linux-gate shrinks. `scripts/evidence-verify-test` (`FleetCompatKey`)
changes each declared input alone and sees the key change, and changes files the stage does not read (a doc, `scripts/st-test`, the
root package, a crate outside the closure, the messaging baseline) and sees it stay.

## The risk, and how each key is kept honest

A key that leaves out an input lets a note cover a tree it never measured. Three guards:

1. **One test per declared input.** For each path group above, a key test (`LinuxClippyKey` in `scripts/dep-info-check-test`)
   changes one file in it and sees the `linux-clippy` key (`scripts/evidence-verify`'s projection of the committed inputs) change. It
   also changes files outside the inputs (docs, unrelated scripts) and sees it unchanged.
2. **The executed set is declared.** `scripts/lane-definition-closure` already computes what a lane executes. A test asserts
   that the clippy stage's executed files are a subset of `linux-clippy`'s inputs, so a new script the stage starts to run fails the
   test until it is declared.
3. **What cargo read is declared.** `scripts/dep-info-check` judges every clippy stage linux-clippy's notes come from. CI's
   linux-clippy job runs it inside the stage (`scripts/ci-linux`, with `CI_KEYED_CLIPPY_POLICY` naming the skip policy), and a local
   note's command, `scripts/ci-clippy-result`, runs it on the stage's report afterwards. linux-gate runs the same stage without the
   policy: its result never depends on the policy file, so a policy change on main does not void its signature
   (`lane-definition-closure --check` fails a lane that reads a protected file its `ci_inputs` leave out). linux-clippy's lane does
   read the whole policy file and declares it; for a keyed check that costs little, since the merge tree it then falls back to has the
   same key unless the pull request changed one of clippy's inputs. The check reads the dep-info file (`.d`, every file rustc
   opened) of each unit the stage's cargo clippy runs reported, and the `rerun-if-changed` paths each workspace build script printed
   (its reads are in no dep-info file). It keeps the repository files outside the target directory and `.git`, and fails with an
   `::error::` line per file the key does not cover. `.git` is the repository's metadata, not its tree: fractal-build-stamp watches
   HEAD, the index and HEAD's ref, which lie under `.git` in a plain checkout such as CI's and outside the repository in the linked
   worktrees local runs use, so only CI would have seen them. PR 1 only warned, while no default local run checked it; since PR 2 every local run checks it
   before the merge queue does. It cannot see files the codegen check reads; those are declared by hand. Its last line, `dep-info: check=linux-clippy units=N files=M uncovered=K`, is counted in the daily report.
   Only units the stage reported are read, so a stale `.d` file in a restored cache cannot fail it. The first measurement (a warm
   target, 2026-10-26) found the files `--all-targets` compiles in from outside the member directories, among them
   `docs/st3/invariants.md`, `docs/st3/schema.md` and `docs/vrs/requirements.md` (tests' `include_str!`) and the `wit/` files a proc
   macro reads. They are in the key, so editing one of those docs does re-run clippy; any other doc does not.

## Rollout

1. PR 1, shadow: the policy entry (`enforce: false`), the run input, the gate and skip wiring, and st-test signing `linux-clippy`
   on request. CI keeps running clippy on every PR; the gate prints what it would have decided. PR 2, still shadow: `linux-clippy`
   in the default local suite, signed from linux-gate's clippy stage; dep-info failures fail; build-script reads; the key tests;
   linux-clippy's `ci_inputs`.
2. Shadow period: `scripts/evidence-flip-ready` counts lines where a reused note and the real clippy job agree. The trigger is the
   one this mission sets for keyed checks: 5 agreeing, 0 disagreeing, 0 hidden red. Factory then signs the flip PR.
3. Evidence for this step: a docs-only pull request whose `linux-clippy` decision line says `match=inputs` with an older tree's
   note, and whose clippy job is skipped once enforced.
4. Fleet-compat follows the same steps.

## Open questions

- The codegen check reads the client schemas under `docs/st3/client-v0/`. Should it move to its own small check, so a
  schema-only docs change does not re-run all of clippy? It is cheap; for v1 it stays in clippy's key.
- `scripts/ci-linux` runs the guards for every stage. A guard change then moves every Linux stage's key. That is correct, but the
  guards could be split from the stage script later, so a guard change re-runs only the guard.
