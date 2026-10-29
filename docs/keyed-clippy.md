# Keyed clippy and fleet-compat (design)

Status: 2026-10-26. Mission `fleet/factory/no-whole-tree`, step `clippy-keyed`. PR 1 (shadow) implements sections 1 and 4, except
st-test, which comes with the linux-gate shrink (below). Factory approved the design on 2026-10-26.

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
- **st-test.** In shadow, the notes come from CI's clippy job, which the merge queue runs on every merge. PR 1 adds
  `linux-clippy` as an opt-in local check (`scripts/st-test --check linux-clippy`; not in `--all`, so a default run never runs
  clippy twice), which records the base inputs the base's verifier reports. With the linux-gate shrink (PR 2) it becomes a default check (`LOCAL['linux-clippy'] = nix develop -c bash scripts/ci-clippy-keyed`, the clippy stage with linux-clippy's dep-info check). When
  it is already signed for its key, linux-gate's local command drops the `clippy` stage. This is the "shrink" `scripts/st-test`
  already describes but does not do. Either way the caller's command stays `scripts/st-test --all --push origin`: the keys are
  computed inside st-test and CI, never by the caller. Until linux-gate itself is split (a later step of this mission), a run that
  needs linux-gate still runs clippy inside it, and signs `linux-clippy` from the same run's clippy stage, so no work is done twice.

### 2. Per crate: not in v1

Clippy runs as one workspace invocation, and the ratchet counts per (crate, lint) against one baseline. Splitting it into
`cargo clippy -p C` per crate would re-check shared dependencies once per invocation, and the ratchet would need a per-crate
baseline. The gain is small next to the whole-workspace key, which already lets every PR that touches no Rust, lint or toolchain
file reuse the note. Per-crate clippy is worth measuring once keyed clippy is trusted: if most Rust PRs touch one or two leaf
crates, a per-crate key (the rust-tests closure plus the baseline rows for that crate) would cut those too.
`docs/per-crate-test-signing.md` section 4 keeps that option open.

### 3. Fleet-compat, the same way

The fleet-compat stage runs the released binaries pinned by `.github/fleet-compat-baseline.json` and
`.github/messaging-compat-baseline.json` (v0.3.20, archive URLs and sha256 values, read by `scripts/compat-baseline`) against the
tree's own build. Its key is:

- the Rust sources and manifests of the binaries it builds (the same group as above, narrowed to the binaries' dependency closure
  when `scripts/rust-test-keys`' closure walker is reused);
- the toolchain;
- both baselines, which are verdict data;
- `scripts/compat-baseline` and the stage's own scripts.

It gets its own check name, `linux-fleet-compat`, with the same policy, gate and st-test steps as clippy. Its exact script list is
taken from `scripts/lane-definition-closure`'s executed set for the stage, as for clippy. The step for it follows clippy's.

## The risk, and how each key is kept honest

A key that leaves out an input lets a note cover a tree it never measured. Three guards:

1. **One test per declared input.** For each path group above, `scripts/evidence-verify`'s key test changes one file in it and sees
   the `linux-clippy` key change. It also changes one file outside the inputs (a doc, an unrelated script) and sees it unchanged,
   in the style of `scripts/rust-test-keys-test`.
2. **The executed set is declared.** `scripts/lane-definition-closure` already computes what a lane executes. A test asserts
   that the clippy stage's executed files are a subset of `linux-clippy`'s inputs, so a new script the stage starts to run fails the
   test until it is declared.
3. **What cargo read is declared.** `scripts/dep-info-check` runs inside the clippy stage (`scripts/ci-linux`) when linux-clippy
   itself runs it: `scripts/ci-clippy-keyed` (linux-clippy's lane) and CI's linux-clippy job set `CI_KEYED_CLIPPY_POLICY` to the skip
   policy, and the check reads linux-clippy's inputs from it, so it judges the stage in CI and in a local run alike. linux-gate runs the
   same stage without that variable: its result never depends on the policy file, so a policy change on main does not void its
   signature (`lane-definition-closure --check` fails a lane that reads a protected file its `ci_inputs` leave out). It reads the dep-info file (`.d`, every file rustc opened) of each unit the stage's
   cargo clippy runs reported, keeps the repository files outside the target directory, and reports each file the key does not
   cover. In shadow (PR 1) it reports them as `::warning::` lines and passes (`--warn`): no default local run checks linux-clippy yet,
   so a failure would first show in the merge queue. With PR 2, which makes linux-clippy a default local check, it fails with an
   `::error::` line per file. Its last line, `dep-info: check=linux-clippy units=N files=M uncovered=K`, is counted in the daily report.
   Only units the stage reported are read, so a stale `.d` file in a restored cache cannot fail it. The first measurement (a warm
   target, 2026-10-26) found the files `--all-targets` compiles in from outside the member directories, among them
   `docs/st3/invariants.md`, `docs/st3/schema.md` and `docs/vrs/requirements.md` (tests' `include_str!`) and the `wit/` files a proc
   macro reads. They are in the key, so editing one of those docs does re-run clippy; any other doc does not.

## Rollout

1. PR 1, shadow: the policy entry (`enforce: false`), the run input, the gate and skip wiring, the key tests, and st-test signing
   `linux-clippy`. CI keeps running clippy on every PR; the gate prints what it would have decided.
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
