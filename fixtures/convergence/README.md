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
