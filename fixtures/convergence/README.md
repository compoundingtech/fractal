# Versioned convergence inputs

`claims-v1.json` is the fixed pretty-printed JSON serialization of
`fractal_schema::registry().claims` from the retirement candidate based on
`92c17ce0fe163cacbe907179cbafd7a98782f78d`. It includes `agent.retired` and its
reserved fields and remains unchanged for reproducible comparisons.

`claims-v2.json` is generated from the review-reader registry at
`5e9b1014226fca3aded0321eccc08279cf9ffded` (schema digest
`c84c80daf817aef1c390282883197a79c10604ce76094a7ac5da84998cae7270`).
Its generation expression is
`serde_json::to_string_pretty(&fractal_schema::registry().claims)`, followed by a
newline. It adds the four durable review claim kinds and captures the complete
review request field schema. The simulation includes this version and records
its filename in audit reports.

The seeded generator draws kinds and fields from its selected snapshot; each
tested revision still admits them through its own native registry and store.
The corpus conformance test requires every live registry kind to be represented.

`claims-v3.json` is the serialization of the registry from harness-question candidate
`7935729a8e564c178ac9b83e940d87dcc543c9f2` (registry digest
`2d3e67b713e9c515b0672761daebbadfe29cc4dc43f6721ed995dd8a2199daa7`).
It adds `harness-question.observed` and `harness-question.resolved`; v1 remains
unchanged. Version 2 belongs to main's review-reader registry; version 3 is the next free
corpus slot. After merging main, regenerate v3 with
`cargo run -p fractal-schema --example claims_json -- fixtures/convergence/claims-v3.json`.
The generator serializes the complete merged registry; do not hand-edit the snapshot.

`claims-v4.json` is the serialization of the registry that adds the four node-local
`client.*` diagnostic kinds on top of version 3; v1, v2 and v3 remain unchanged for
comparisons pinned to those corpora. The simulation includes v4 because its coverage
test requires those kinds. The generator handles the `client` subject family explicitly.
A focused regression constructs each diagnostic kind through the same input-generation
path used by random draws; corpus membership alone is not sufficient coverage.
After merging main, regenerate v4 with
`cargo run -p fractal-schema --example claims_json -- fixtures/convergence/claims-v4.json`.
The generator serializes the complete merged registry; do not hand-edit the snapshot.

The fixed v4 seed characterization has only seed 5 without a checkpoint trim;
the other 16 fixed seeds trim, including seed 34. The diagnostic kinds and their
`client` subject generation change the shared RNG stream, so v3's `[5, 34]`
characterization does not apply. Keep the exact per-seed trim assertion when
updating this characterization for a new corpus.

`claims-v5.json` is the serialization of the registry from the harness-inventory branch
merged with main's version 4, generated with
`cargo run -p fractal-schema --example claims_json -- fixtures/convergence/claims-v5.json`.
It adds `harness.inventory.observed`; v1 to v4 remain unchanged. The kind changes the shared
RNG stream, so v4's never-trim characterization `[5]` does not apply to it: with v5 only
seed 1597 has no checkpoint trim and the other 16 fixed seeds trim, including seed 5. After merging
main, regenerate v5 the same way; do not hand-edit the snapshot.

`claims-v6.json` captures the merged identity-rename registry, including the durable
`agent.renamed` and `mission.renamed` facts and their naming-chain fields.
Regenerate it with
`cargo run -p fractal-schema --example claims_json -- fixtures/convergence/claims-v6.json`.
The simulation selects v6 and records that filename in its audit summary; prior
corpus versions stay fixed for comparisons.

Mission publications use the owning `Store::apply_as` declaration path. Unlike ordinary
claims, `mission.published` carries a canonical top-level `MissionSpec` with its identity
and content-hashed revision; wrapping random schema fields in `ClaimInput` produces a
body the mission projector cannot read. The generator still draws that kind and all its
fields from the snapshot, then publishes a valid mission without additional RNG draws.
The focused publication regression checks its canonical body and replication to the oracle.
This input bug also exists in main's generic publication generator; it is not a rename
dispatch bug. In CI run 39562565916, the seed-34 failure details explicitly identify
`claim_kind: mission.published`, not `mission.renamed`. The rename kinds remain in v6.

With v6 and canonical mission publication inputs, the measured never-trim set is `[34]`.
All 17 fixed seeds converged; seeds 1, 2, 3, 5, 8, 13, 17, 21, 55, 89, 144, 233, 377,
610, 987 and 1597 trimmed, while seed 34 did not (16/17 trim coverage). The scoped
2026-10-21 run used
`cargo test -p fractal --locked --test integration convergence:: -- --nocapture --test-threads=4`
through the heavy gate and pinned Nix environment. It passed the canonical-publication
regression and all convergence oracles; its only two failures were the old v5 trim
assertions for seeds 34 and 1597. Characterization follows those measured outcomes,
not an RNG prediction; the exact per-seed equality and majority coverage checks remain.

`claims-v7.json` is the serialization of the registry that adds the system-only claim kind
`slo.breach-transition` on top of version 6; v1 to v6 remain unchanged for comparisons pinned to
those corpora. It is the next free corpus slot after the identity-rename registry. After merging
main, regenerate it with
`cargo run -p fractal-schema --example claims_json -- fixtures/convergence/claims-v7.json`;
do not hand-edit the snapshot. The kind changes the shared RNG stream, so v6's never-trim
characterization `[34]` does not apply: with v7 seeds 2 and 34 do not trim a checkpoint and the
other 15 fixed seeds do, measured with
`cargo test -p fractal --locked --test integration convergence:: -- --test-threads=4`.

`claims-v8.json` captures the six durable version-1 deployment audit kinds added
on top of v7. Regenerate it with
`cargo run -p fractal-schema --example claims_json -- fixtures/convergence/claims-v8.json`;
do not hand-edit the snapshot. Versions 1 through 7 stay frozen. The seeded
generator supplies schema-valid deployment package tuples, bounded RFC3339 issue
and expiry times, preview/signoff references and STOP host lists, rather than
generic empty nested objects. A focused regression generates and appends every
deployment kind through the same path used by random schedules.
The measured v8 never-trim set is `[13, 34, 144, 987]`: the other 13 of 17 fixed
seeds trim. Every seed converges, and exact per-seed trim assertions and majority
coverage checks remain enforced. The characterization run used
`cargo test -p fractal --test integration convergence:: -- --nocapture --test-threads=4`.

`claims-v9.json` captures the current helper-launcher registry, including
`agent.launcher-bound`, `agent.helper-launched` and `client.action-result`.
Versions 1 through 8 stay frozen. Generate v9 from the merged registry with
`cargo run -p fractal-schema --example claims_json -- fixtures/convergence/claims-v9.json`;
do not hand-edit or overwrite v8. The simulation selects v9 and records its filename
in audit summaries.

The helper binding, helper launch history and client action receipt kinds use the
owner-local log. The existing generator and write accounting already handle
`Retention::Local`: generated inputs go through the native store, but accepted
local observations do not enter the replicated claim set or the oracle. Their
`SystemOnly` write policy does not require a generator exclusion or a replicated
claim fallback; the internal append path is the same path used for other generated
system observations.

The measured v9 never-trim set is `[3]`: seeds 1, 2, 5, 8, 13, 17, 21, 34, 55,
89, 144, 233, 377, 610, 987 and 1597 trim (16/17 coverage). All 17 fixed seeds
converge. Characterization used
`cargo test -p fractal --test integration convergence:: -- --nocapture --test-threads=4`
on a gated Linux build host. The first v9 run failed only the five stale v8
trim expectations; exact per-seed equality and majority trim coverage remain.

Keep the snapshot fixed when comparing revisions. For a future schema change,
create a new version with `serde_json::to_string_pretty(&fractal_schema::registry().claims)`
and deliberately update the simulation's include. Apply identical simulation
support and the same snapshot to the baseline; do not copy production admission
or retirement changes into it. The existing checkpoint and planning-session
generator exclusions remain in the generator, where their reasons are documented.

The simulation also scopes checkpoint build/executable identity to fixed values.
Actual production proof identity is unchanged. A run's clock and random generator
come from its seed, rather than host time or the executable hash.

Run the opt-in wide audit with `cargo test --release -p fractal --features
convergence-audit --test convergence_audit -- --ignored --exact
convergence::fixed_sweep --nocapture`. Set `CONVERGENCE_REPORT_DIR` to a fresh
directory, `CONVERGENCE_START` (default 1), `CONVERGENCE_RUNS` (default 500), and
`CONVERGENCE_WORKERS` (default 4). Set `CONVERGENCE_SAVE_FAILURE=1` to preserve
failed worlds and their report. Use the host's configured test runner when
executing a copied binary.

The audit records every seed and fails after collecting all outcomes. Reusing the
same output directory resumes completed outcomes, including failures; it is only
valid with the same input snapshot, simulation source and native revision.
Keep the source and binary hashes with the reports. Never discard a failed
outcome when resuming an interrupted audit.

`claims-v10.json` captures the merged person-repair registry, adding
`work.person-reassigned`, `gate.reviewer-reassigned` and `person.rename.completed`.
Versions 1 through 9 stay frozen. Generate it with
`cargo run -p fractal-schema --example claims_json -- fixtures/convergence/claims-v10.json`;
do not hand-edit the snapshot. Slot 10 was selected after fetching current main
immediately before generation. The simulation and audit summaries select v10.
With this corpus, all 17 fixed seeds converge and trim a checkpoint, including seed 3;
the exact `NEVER_TRIMS` set is empty. The remote characterization used
`cargo test -p fractal --test integration convergence::seed_ -- --nocapture --test-threads=1`.
That run observed every seed trim; its sole failed assertion was v9's obsolete
expectation that seed 3 never trims. The per-seed equality remains enforced.

`claims-v11.json` captures the registry that adds the GitHub plugin's two compact kinds,
`vcs.item.state` on `resource` subjects and `github.event` on the new `github-event` family
(docs/st3/plugins.md). Versions 1 through 10 stay frozen. Generate v11 from the merged
registry with
`cargo run -p fractal-schema --example claims_json -- fixtures/convergence/claims-v11.json`;
do not hand-edit or overwrite v10. The generator names `github-event` subjects explicitly;
each draw takes a fresh event key, since `github.event` is written once per subject. The
simulation selects v11 and records its filename in audit summaries.

With v11 all 17 fixed seeds converge and trim (17/17 coverage), so the never-trim set stays
empty. Characterization used
`cargo test -p fractal --locked --test integration convergence:: -- --nocapture --test-threads=4`
(26 passed, 189.5 s).

Under corpus v11 a wide audit of seeds 1–300 failed 15 seeds in `resource_observations`
(#4426). A rebuild of the resource projection, which a replay or a checkpoint proof runs, kept
only subjects with a `resource.observed` claim. So an item that only `vcs.item.state` claims
describe lost its row on whichever node rebuilt. With the rebuild fixed, all 300 seeds converge.
The 15 seeds are now fixed regression seeds: 27, 74, 106, 121, 130, 133, 179, 209, 241, 246,
277, 281, 291, 293 and 294. Each of them trims, so the never-trim set stays empty. The audit
used
`CONVERGENCE_START=1 CONVERGENCE_RUNS=300 CONVERGENCE_WORKERS=4 cargo nextest run -p fractal -E 'test(=convergence::fixed_sweep)' --run-ignored only`
(1363 s); the 14 seeds of 1–300 that did not trim are not fixed seeds.

`claims-v12.json` captures the registry that adds `fault.dismissed` (a person's or the fleet
fault agent's dismissal of a fault whose run has ended), and the current `daemon.diagnostic`
fields. Versions 1 through 11 stay frozen. Generate v12 from the merged registry with
`cargo run -p fractal-schema --example claims_json -- fixtures/convergence/claims-v12.json`;
do not hand-edit or overwrite v11. The simulation selects v12 and records its filename in audit
summaries.

With v12 every fixed seed converges. The new kind shifts the draws, so seeds 2, 34 and 106 no
longer reach a checkpoint trim; they are the exact never-trim set, and every other fixed seed
trims. Characterization used
`cargo test -p fractal --locked --test integration convergence:: -- --test-threads=4`.
