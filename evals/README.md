# Local eval corpus

The [Fractal corpus](fractal/) contains the current eval fixtures. Each KDL-backed eval
starts with `version 2`; complete harness variants can live in its `variants/` directory.
Future imports require an explicit eval review before they enter this corpus.

## Eval and harness inventory

The model-free and model-backed lists in
[`crates/fractal/tests/examples.rs`](../crates/fractal/tests/examples.rs) are the maintained
inventory. Its tests compare those lists with the actual `evals/fractal/*/eval.kdl` files,
check that model-free fixtures have no model seats or gates, and verify the declared native
harnesses and model judges. Read the current fixture and controller for exact models and
seat counts rather than using a second hand-copied aggregate here.

To list KDL files from the repository root:

```sh
rg --files evals/fractal -g eval.kdl
```

The inventory includes attention inbox, free mode, loop exhaustion/feedback/until-green,
queue revision, replication convergence, review escalation, runtime readiness recovery and
seat mission work. [Codex minimal access](fractal/codex-minimal-access/eval.kdl) declares one
Codex worker and no model judge; its gates check managed minimal permissions.

A runner may create its agents dynamically. Graceful messaging uses
`scripts/fractal-graceful-messaging-eval/run` and records the selected harnesses in its report;
it has no eval KDL. Messaging faults uses real native drivers and embedded extensions with a
token-free provider stand-in across two isolated nodes, so it adds no native model seats.

Model agents use a native `harness` block. Setup and fixture processes can use `command`.
A variant is a complete eval KDL that changes only the seat under test. Keep native agents
and model judges distinct in the run report, and record the concrete provider model when
a fixture selects an alias. Fixture declarations pin their own task models and judges.

## Fractal message discipline

The graph owns planned work, assignment, dependencies, progress, products, and judgement state.

An eval does not author a harness prompt; Fractal refuses one. A seat starts idle, and its work arrives as the message Fractal posts when an assigned step is ready.

Put mission work in goals and steps. Keep only stable repository facts in `AGENTS.md`, `CLAUDE.md`, or a persona file.

Do not disable harness features or reveal held-out gate criteria in agent instructions.

Fractal sends one durable Fractal message when an assigned parent becomes ready.

Inherited nested steps use the parent message. A nested step sends a new message only when its assignee changes,
or when its agent submitted the parent while that nested step was still ready.

The native driver transports a graph message. It does not create a second message source.

An agent sends a direct message for a blocker, an exception, a requested result, or explicit coordination work.

An agent does not send routine progress when graph work state already expresses that progress.

An eval that tests coordination must judge the required Fractal message sequence.

An eval must keep enough work state in the graph to survive an agent or daemon restart.

## Run reports

Each eval run gets one concise Markdown report in its eval folder.

Use `reports/YYYY-MM-DD-<run-id>.md`. Copy [run-report-template.md](./run-report-template.md) as the starting form.

Record exact values when the runtime exposes them. Mark an unavailable value and give the reason instead of estimating it.

Commit the report with the run evidence. Do not leave the only report in terminal output or temporary state.

## Validation

```sh
cargo test -p fractal --test integration examples::
```

Fractal eval orchestration is exercised through repository integration controllers and is deliberately
not a public CLI command. The Rust test validates every Fractal fixture against the normative grammar
and its structural contracts. Paid live runs must use an explicit development controller so the
public product CLI does not acquire an unfenced raw-fixture launcher.
