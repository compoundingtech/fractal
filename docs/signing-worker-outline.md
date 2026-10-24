# Dedicated signing worker outline

This is an operational design, not an implemented service. Use qualified x86_64 Linux hosts (initially dev3/dev5); authoring on another platform does not require distributing signing keys there.

## Identity and trust

- One dedicated, non-root Unix signing account and independently generated enrolled key per host. Run `scripts/evidence-keygen --level agent --name ALIAS` as that account. Use invented aliases, not account/host names.
- Keygen stores the private key at `~/.config/st/evidence-sign-key` (0600), configuration and signing scratch repository in the same private directory. Enroll only the public allowed-signers line through a reviewed `.st/evidence-keys` change on main; do not move private keys between hosts. Keep a private alias-to-host inventory and a rotation/revocation owner.
- A signing account executes repository code. Account separation limits access from ordinary authoring accounts, but is not proof of honest execution or a sandbox against malicious submitted code reading that account's key. Restrict submitters to trusted fleet agents; stronger adversarial isolation requires a separately designed signing boundary, not arbitrary remote signer commands.

## Exact-commit request

- An authenticated agent submits an allowlisted repository, an already-pushed branch/ref, and the complete immutable commit SHA. Never accept shell command strings, arbitrary remotes or caller-supplied signer executables. The publishing manager, not this design task, owns source pushes.
- Fetch that ref from the configured origin, resolve it and verify the requested SHA is reachable. Pin the job to that SHA, not the branch's later tip. A branch moving during execution cannot change the requested commit.
- Use a clean worker-owned detached checkout of that commit outside canonical branch-bound megarepo paths. Verify HEAD equals the request before invoking `scripts/st-test --all --push origin`. st-test builds the committed head tree in its own disposable pool. Do not merge main or rewrite the submitted commit. Enrollment and policy acceptance still come from accepted main.
- The response names requested commit, tested tree, accepted base, host alias, run ID and evidence refs. Later commits require fresh coverage; exact-head review/GO remains a separate responsibility.

## Concurrency and disk

- One full run per host, protected by a host-wide exclusive lock across worker processes, with bounded queued requests. Set `ST_TEST_TREES=1`; runner pool locks alone permit multiple trees and do not impose this host limit.
- Give the account a stable short pool path outside canonical megarepo worktrees, a short noncanonical job scratch root for fixtures/sockets, and stable account-owned cargo/sccache caches. Scope TMPDIR to the job; do not use canonical-mutation or agent-policy bypass flags.
- Prefer its single retained `target-0`; alternatively `ST_EVIDENCE_TARGET` supports an explicitly shared, runner-locked target. Do not have two hosts concurrently write one cargo target. Configure sccache through the existing Nix/toolchain contract rather than inventing a second compilation path.
- Apply host admission before starting: reserve RAM/cores, honor the manager's free-disk fence, and account for targets, pinned members, Nix and sccache together. The current per-target pre-run cap is not an aggregate quota and does not bound growth during a run. The dev5 cold trial consumed 17.54 GB peak RAM and 11.52 GB allocated cargo target; do not admit many cold runs based on apparent file sizes or a promised warm runtime. Pending #3713/#3722 cache/retention changes need evaluation before relying on them.

## Publication and failure

- st-test's `--push origin` publishes signed evidence notes and evidence tags, not the source branch. Retain/publish the refs it actually reports; do not invent a separate signature file transport. Give the account only required repository/evidence publication access and never force-push source branches.
- Return per-check exit/result, elapsed time, unsigned/refused reasons, durable redacted log location and publication errors. Failed checks must not be reported as usable passes; partial existing coverage must be distinguished from newly signed evidence. Keep successful signatures and failed run diagnostics attributable to the exact SHA/tree.
- Do not automatically retry red tests or bypass lane-definition edits, policy refusals, missing enrollment, resource fences or publication authorization. Report actionable failures to the requesting agent/manager. Bounded service restart recovery may resume queue processing, but must not claim an interrupted job passed.

## Platform qualification

The documented full-suite route requires Linux x86_64 with Nix. st-test has no early platform gate. Native aarch64 Linux currently lacks a supported compatibility baseline in `scripts/compat-baseline`; arm64 Darwin has a baseline but Linux-gate parity is not established. Both dev4 and mbp2021 should submit exact commits to qualified Linux workers until native lanes, baselines and acceptance policy are explicitly qualified. Generating a key on those platforms alone does not qualify a full-suite signer.
