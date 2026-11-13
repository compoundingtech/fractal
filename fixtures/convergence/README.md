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

`claims-v13.json` captures the registry that adds `member.admission-override` (a person's or an
agent's release of a restarted member from waiting for the named peers, for its current daemon
start). Versions 1 through 12 stay frozen. Generate v13 from the merged registry with
`cargo run -p fractal-schema --example claims_json -- fixtures/convergence/claims-v13.json`;
do not hand-edit or overwrite v12. The simulation selects v13 and records its filename in audit
summaries.

With v13 every fixed seed converges. The new kind shifts the draws: seeds 2, 34 and 106 now trim,
and seed 1597 is the exact never-trim set. Characterization used
`cargo nextest run -p fractal --features test-support -E 'test(/convergence::/)'`.

`claims-v14.json` captures the registry that adds `work.handoff-returned`, the durable
record that returns handed-off work to its owning agent. Versions 1 through 13 stay
frozen. Generate v14 from the merged registry with
`cargo run -p fractal-schema --example claims_json -- fixtures/convergence/claims-v14.json`;
do not hand-edit the snapshot. The simulation selects v14 and records its filename
in audit summaries.

With v14 every fixed seed converges; seeds 21 and 74 are the exact never-trim set.
Characterization used
`cargo test --locked -p fractal --test integration convergence:: -- --nocapture --test-threads=4`.
The first characterization run failed only the three stale v13 trim expectations;
the per-seed equality and majority-trim assertions remain enforced.

`claims-v15.json` captures the registry that adds `fleet.shard-bounds`, a leaving member's signed
record of how far it wrote in the messages shard. Versions 1 through 14 stay frozen. Generate v15
from the merged registry with
`cargo run -p fractal-schema --example claims_json -- fixtures/convergence/claims-v15.json`;
do not hand-edit the snapshot. The simulation selects v15 and records its filename in audit
summaries.

With v15 every fixed seed converges; seeds 2 and 27 are the exact never-trim set (seeds 21 and 74
trim again). Characterization used
`cargo test --locked -p fractal --test integration convergence:: -- --nocapture --test-threads=4`.
The first characterization run failed only the four stale v14 trim expectations.

`claims-v16.json` adds `runtime.refresh` and optional receipt `launch_contract` fields; v1–v15 stay frozen.
Generate with `cargo run -p fractal-schema --example claims_json -- fixtures/convergence/claims-v16.json`.
Shared RNG draws shift: all 32 fixed seeds converge; 30 trim, exactly `[2, 34]` never trim (27 now trims). Refresh claims stay whole.

`claims-v18.json` captures the complete registry with `message.expired`, the durable,
runtime-authored mail expiry outcome. Versions 1 through 17 stay frozen. Generate it
from the merged expiry registry with
`cargo run -p fractal-schema --example claims_json -- fixtures/convergence/claims-v18.json`;
the example serializes `fractal_schema::registry().claims` with
`serde_json::to_string_pretty`, followed by a newline. Do not hand-edit the snapshot.
The simulation selects v18. Expiry draws select an existing sent message in stable
subject order and use `daemon/runtime`; native admission still rejects mail with a
terminal receipt. The focused expiry regression generates the outcome through the
same path as random schedules and checks native admission and replication to the
oracle, rather than treating corpus membership as write coverage.

`claims-v19.json` captures the complete registry with `program.step`, the system-only, append-only
journal entry of a program step run (docs/st3/program-engine.md §5). Versions 1 through 18 stay
frozen. Generate it from the merged registry with
`cargo run -p fractal-schema --example claims_json -- fixtures/convergence/claims-v19.json`;
do not hand-edit the snapshot. The simulation selects v19. Draws use the existing `step-run`
subject family and the generic system-only append path.

A drawn `client.operation-authorized` (in the corpus since v17) is not written. Every replica
admits that kind only from a keyed member's signed envelope and holds an unkeyed writer's as
unsigned; the simulated nodes have no member keys. Written, it stayed pending on every other node
and the oracle and held every later checkpoint back, so the run never settled and the writer held
a claim the oracle never saw. On hosted CI, v19's draws first wrote one, in seeds 1 and 34; off
CI, schedules diverge and more seeds wrote one (seeds 3, 13, 17, 277 and 1597 in burst run
42880734602). The input is still drawn, so a seed whose store refused it keeps its schedule.
With v19 every fixed seed converges and trims (hosted run 42884923668), so the never-trim set is
empty. The never-trim set is characterized by hosted CI: seed runs on a loaded shared host or a
lightly loaded burst runner diverge from CI at the same commit (#6210); burst run 42884913070
leaves seeds 55 and 610 untrimmed.

`claims-v20.json` captures the complete registry with the source-enforcement kinds
`source.debt.recorded`, `source.debt.closed` and `source.audited`. Versions 1 through 19 stay
frozen. Generate it from the merged registry with
`cargo run -p fractal-schema --example claims_json -- fixtures/convergence/claims-v20.json`;
do not hand-edit the snapshot. The simulation selects v20, and the draws use the generic append path.

`claims-v21.json` adds `read-position.advanced` to the merged program-step and source-enforcement
registry. Main's versions 1 through 20 remain byte-identical. The simulation now selects v21.
Generate it with
`cargo run --locked -p fractal-schema --example claims_json -- fixtures/convergence/claims-v21.json`;
do not hand-edit it. Read-position draws use the person's native admission path and the focused
regression checks admission and oracle replication. Preserve the keyed-only write exclusion:
the unkeyed simulated fleet still draws, but cannot admit, operation authorizations.
Changing selected kinds and random draws changes subsequent actions in the shared SplitMix64
schedule, including checkpoint work. Earlier trim characterization is not evidence for v21;
do not add trim exceptions without observing the generated inputs.

`claims-v22.json` adds `source.mapped`, the durable, system-written source map of one agent or
mission revision (docs/st3/owned-sets.md). Versions 1 through 21 stay frozen. Generate it with
`cargo run --locked -p fractal-schema --example claims_json -- fixtures/convergence/claims-v22.json`;
do not hand-edit it. The simulation selects v22. A draw stages one text on the writer and claims
it under `blob_hash` on `agent/<id>@<revision>`, as a set publication does, so the claim's append
promotes the text into the shared blobs that replicate with it. The focused regression checks
admission and that the oracle receives the claim with its text.
With v22 every fixed seed converges; seed 34 is the exact never-trim set, observed by hosted
linux-tests (run 43045279669) and the st-test-all linux gate (burst run 43045420319). Burst
cargo-test run 43045414076 also left seed 74 untrimmed, a runner divergence (#6210).

`claims-v23.json` adds the durable, system-written `mission-run.retry-linked` kind from
`fractal_schema::registry()`. Versions 1 through 22 stay frozen. Generate the complete
registry snapshot with
`cargo run --locked -p fractal-schema --example claims_json -- fixtures/convergence/claims-v23.json`;
do not hand-edit it. The simulation and its audit reports select v23. The kind uses the generic
draw and append paths: `source` and `retry` are required `mission-run` references, and `reason`
is an optional string. The focused regression validates a generated input against the native
registry, admits it, and replicates it to the oracle. Each revision still applies its own
admission rules; old-build checkpoint and comparison semantics are unchanged.

Adding this kind changes the shared random schedule. Hosted
[run 43269741954](https://github.com/compoundingtech/fractal/actions/runs/43269741954)
exercised every fixed seed: all trimmed, including seed 34. Therefore v23 requires actual
checkpoint trimming for every fixed seed; v22's negative seed-34 expectation is not carried
forward. Convergence and checkpoint checks remain mandatory.

`claims-v24.json` adds the durable, system-written `mission-run.supersede-linked` kind and
the optional run-admission and fault-publication metadata from `fractal_schema::registry()`.
Versions 1 through 23 stay frozen. Generate the complete current snapshot with
`cargo run --locked -p fractal-schema --example claims_json -- fixtures/convergence/claims-v24.json`;
do not hand-edit it. The simulation and audit reports select v24. Supersession uses required
`source` and `replacement` mission-run references and a required publication string. The
focused lineage regression validates, admits, and replicates both retry and supersession
inputs through the existing generic paths. No old-build admission or checkpoint comparison
exception is added. Earlier per-seed trim characterizations do not establish v24 behavior:
every fixed seed must converge and trim, without inheriting v22's negative exception.

`claims-v25.json` adds `github.workflow-run`, the GitHub poller's once-per-run claim of a completed
Workspace CI run on `github-workflow-run/OWNER/REPO/RUN_ID` (docs/st3/plugins.md), and carries the
optional `gate.requested`, `runtime.action.requested` and `runtime.observed` fields the registry has
gained since v24. Versions 1 through 24 stay frozen. Generate it with
`cargo run --locked -p fractal-schema --example claims_json -- fixtures/convergence/claims-v25.json`;
do not hand-edit it. The simulation selects v25; a draw writes the kind through the generic append
path on a fresh run subject, as it does `github.event`. With v25 every fixed seed converges; seeds
2 and 233 are the exact never-trim set, observed by hosted linux-tests (runs 43416435882 and
43424149648) and hosted burst cargo-test run 43411988505. In both seeds the old build draws the new
kind, and that one replicated claim makes it a writer every later checkpoint waits for; an old build
never seals. Seed 233 never excuses it (quiesce's even draw says no). Seed 2 does, but its schedule
ends with `checkpoint/2100-01-24` sealed under two participant sets and no later cut. Hosted burst
runs on throwaway tracing commits show this: in
[43425100378](https://github.com/compoundingtech/fractal/actions/runs/43425100378) excusing seed
233's old build trims every modern node; in
[43428226636](https://github.com/compoundingtech/fractal/actions/runs/43428226636) skipping only
the old build's `github.workflow-run` write, after drawing it so the schedule is unchanged, trims
both seeds; in [43431112343](https://github.com/compoundingtech/fractal/actions/runs/43431112343)
one more cut trims seed 2 on every modern node. So the kind changes which writers a checkpoint
waits for, not how claims trim. Each never-trim seed's test goes on to excuse the old build and let
the next cut fall due, and requires every modern node to trim then. Hosted linux-tests remains the
authority (#6210).

`claims-v26.json` adds the person-availability kinds `person.status-set`,
`person.interruptions-set`, `person.deadline-clock-set` and `person.presence-observed`, the
optional `source` of `person.directive-note-set` and the optional `priority` of
`work.person-asked`. Versions 1 through 25 stay frozen. Generate it with
`cargo run --locked -p fractal-schema --example claims_json -- fixtures/convergence/claims-v26.json`;
do not hand-edit it. The simulation and audit reports select v26. Draws use the generic append
path: only the subject person's own claims are admitted, as on every admission path. The new
kinds and fields shift the shared random schedule.

Hosted [burst run 43455339387](https://github.com/compoundingtech/fractal/actions/runs/43455339387)
observed every fixed seed converge and trim a checkpoint, including v25's non-trimming seeds 2
and 233 and the earlier availability corpus's seed 21. The exact v26 `NEVER_TRIMS` set is empty:
no previous corpus's schedule-specific exception carries forward. There are no untrimmed seeds
requiring the old-build excusal and next-cut continuation in this corpus.
