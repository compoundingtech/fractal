# Repository instructions

## Fix causes; do not hide broken paths

Fix the cause of a slow or broken path. Do not add fallbacks, retries, caches or timeouts that hide it. A workaround that makes the symptom disappear while leaving the defect in place is not an acceptable fix.

Retries for peers and networks are valid because those connections can be unreachable by nature. Keep that distinction explicit; retries must not hide a defect in a local path.

Build reactive behavior instead of polling. Use incremental view maintenance when necessary to keep updates and reads efficient.

Nathan approved this policy on 2026-10-06. See issue #1566 and st document doc/fleet/smalltalk/no-workarounds/hunt-2026-10-06 for examples.

## Evidence: run the tests, they sign

Pushing a branch signs its checks first when they have no signed evidence yet: `scripts/install-git-hooks` (run by `nix develop`) installs a pre-push hook that runs `scripts/st-test` on this machine and pushes the notes, then lets the push go. It never blocks a push. `ST_SKIP_EVIDENCE=1 git push` skips it for work in progress (the pull request then simply runs in CI); `scripts/st-test --status` says whether your tree is covered; `scripts/st-test` runs and signs by hand. See [docs/st3/gate-evidence.md](docs/st3/gate-evidence.md), "Running the checks locally" and "Host keys".
