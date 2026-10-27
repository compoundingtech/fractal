# Local eval corpus

This repository keeps an st2 and an st3 form of each migration eval.

An st3-only eval can prove a new runtime feature that st2 does not implement.

The initial st2 corpus came from `compoundingtech/evals` commit `3db48ab56d40ce27dfd94f89d2db9b692d93836a`.

Future imports require an explicit eval review before they enter this corpus.


| Eval | st2 | st3 |
| --- | --- | --- |
| License MIT | `st2/license-mit` | `fractal/license-mit` |
| Ghost bug | `st2/ghost-bug` | `fractal/ghost-bug` |
| Signal rename | `st2/signal-rename` | `fractal/signal-rename` |
| Restart continuity | `st2/restart-continuity` | `fractal/restart-continuity` |
| Fork in the road | `st2/fork-in-the-road` | `fractal/fork-in-the-road` |
| Poisoned pull request | `st2/poisoned-pr` | `fractal/poisoned-pr` |
| Test Writing | `st2/test-writing` | `fractal/test-writing` |
| Weird Git Setup | `st2/weird-git-setup` | `fractal/weird-git-setup` |
| Claude Skill Inheritance | `st2/claude-skill-inheritance` | `fractal/claude-skill-inheritance` |
| Resource cold start | `st2/resource-cold-start` | `fractal/resource-cold-start` |
| Resource retarget | `st2/resource-retarget` | `fractal/resource-retarget` |
| Resource handoff | `st2/resource-handoff` | `fractal/resource-handoff` |
| Context and resource continuity | `st2/context-resource-continuity` | `fractal/context-resource-continuity` |
| Crash escalation | `st2/crash-escalation` | `fractal/crash-escalation` |
| PTY attach machine stream | `st2/pty-attach-machine-stream` | `fractal/pty-attach-machine-stream` |
| PTY attach only | `st2/pty-attach-only` | `fractal/pty-attach-only` |
| PTY send and peek | `st2/pty-send-peek` | `fractal/pty-send-peek` |
| Network smoke | `st2/network-smoke` | `fractal/network-smoke` |
| Network isolation | `st2/network-isolation` | `fractal/network-isolation` |
| Mission Document Lift | Not supported | `fractal/mission-document-lift` |
| Mixed Worker Pool | Not supported | `fractal/mixed-worker-pool` |
| Cross-harness message wake | Not supported | `fractal/cross-harness-message-wake` |
| Work wake reliability | Not supported | `fractal/work-wake-reliability` |
| Seat queue | Not supported | `fractal/seat-queue` |
| Planning Mode | Not supported | `fractal/planning-mode` |
| Run Generation Revision | Not supported | `fractal/run-generation-revision` |
| Mission Inputs | Not supported | `fractal/mission-inputs` |
| Local File Refresh | Not supported | `fractal/local-file-refresh` |
| Constraint Inheritance | Not supported | `fractal/constraint-inheritance` |
| Continuous Stewardship | Not supported | `fractal/continuous-stewardship` |
| Agent Migration Rehearsal | Not supported | `fractal/agent-migration-rehearsal` |
| Automatic GitHub Intake | Not supported | `fractal/automatic-github-intake` |
| Codex compaction message delivery | Not supported | `fractal/codex-compaction-message-delivery` |
| Native session import | Not supported | `fractal/native-session-import` |
| Graceful messaging | Not supported | `fractal/graceful-messaging` |
| Messaging faults | Not supported | `fractal/messaging-faults` |
| Codex minimal access | Not supported | `fractal/codex-minimal-access` |

The License MIT, Restart continuity, and Claude Skill Inheritance pairs use Claude Sonnet in both runtimes.

The Ghost bug, Signal rename, Fork in the road, Poisoned pull request, Test Writing, and Weird Git Setup pairs use Codex.

The ten remaining pairs are model-free.

Mission Inputs, Local File Refresh, and Constraint Inheritance are also model-free.

Run Generation Revision starts one Codex planner through the launch API.

The Fractal corpus has 46 active evals. Twenty-three are model-free, and twenty-three use at least one model.

Each eval KDL starts with a document version. A missing version means version zero.

st2 accepts versions zero and one. st3 accepts version two.

## Harness inventory

The seat counts include every native agent seat. The LLM judge counts are separate.

| Runtime | Eval | Native agent seats | LLM judges |
| --- | --- | --- | --- |
| st2 | License MIT | Claude Sonnet × 2, Codex × 1 | None |
| st2 | Ghost bug | Codex × 2 | None |
| st2 | Signal rename | Codex × 4 | None |
| st2 | Restart continuity | Claude Sonnet × 2 | None |
| Fractal | License MIT | Claude Sonnet × 2 | Codex × 1 |
| Fractal | Ghost bug | Codex × 2 | Codex × 1 |
| Fractal | Signal rename | Codex × 4 | Codex × 1 |
| Fractal | Restart continuity | Claude Sonnet × 2 | None |
| st2 | Fork in the road | Codex × 4 | None |
| Fractal | Fork in the road | Codex × 4 | None |
| st2 | Poisoned pull request | Codex × 2 | None |
| Fractal | Poisoned pull request | Codex × 2 | None |
| st2 | Test Writing | Codex × 2 | None |
| Fractal | Test Writing | Codex × 2 | None |
| st2 | Weird Git Setup | Codex × 1 | None |
| Fractal | Weird Git Setup | Codex × 1 | None |
| st2 | Claude Skill Inheritance | Claude Sonnet × 1 | None |
| Fractal | Claude Skill Inheritance | Claude Sonnet × 1 | None |
| Fractal | Mission Document Lift | Codex × 1 | None |
| Fractal | Mixed Worker Pool | Claude Sonnet × 1, Codex × 1 | None |
| Fractal | Cross-harness message wake | Claude Opus × 1, Codex × 1, Pi × 1, OMP × 1 | None |
| Fractal | Work wake reliability | Codex × 1 | None |
| Fractal | Seat mission work | Codex × 1 | None |
| Fractal | Seat queue | Claude Sonnet × 1, or Codex × 1 or OMP × 1 chosen by its runner | None |
| Fractal | Planning Mode | Codex × 1, created by the launch API | None |
| Fractal | Run Generation Revision | Codex × 1, created by the launch API | None |
| Fractal | Continuous Stewardship | Codex × 1 | None |
| Fractal | Agent Migration Rehearsal | Codex × 1 | None |
| Fractal | Automatic GitHub Intake | Codex × 1 | None |
| Fractal | Codex compaction message delivery | Codex × 1 | None |
| Fractal | Native session import | Claude × 1, Codex × 1, Pi × 1, OMP × 1, OpenCode × 1 (raw, then imported) | None |
| Fractal | Graceful messaging | One seat for each harness its runner names: Claude, Codex, OMP, Pi, OpenCode | None |

The paired and st3-only corpus has 14 Claude seats, 43 Codex seats, two Pi seats, two OMP seats,
and one OpenCode seat. It also has three Codex LLM judges.

Graceful messaging is not in those seat counts: `scripts/fractal-graceful-messaging-eval/run` chooses
its seats at run time, and its committed reports name them. It needs no eval KDL, because it restarts
and replaces its own daemon.

Seat queue is counted with its default Claude seat. `scripts/fractal-seat-queue-eval/run` can put Codex
`gpt-6-luna` or omp `openai-codex/gpt-5.6-luna` in that seat instead. Its chief agent is a
model-free command, not a native agent seat.

The twenty-three model-free Fractal evals add no model seats and no LLM judges.
Messaging faults runs real native drivers and embedded extensions with a token-free provider API
stand-in across two isolated nodes; it adds no native model seats.

Claude task baselines remain explicitly pinned to `claude-sonnet-5`; the native-channel wake
canary uses Claude Code's `opus` alias and records the concrete model selected by the provider.

Model agents must use a native `harness` block. A setup or fixture process can use `command`.

An eval can keep seat variants in `variants/`. A variant is a complete eval KDL that changes only the
seat under test, so one fixture can compare harnesses. The omp readiness runs use omp on
`openai-codex/gpt-5.6-luna`, Codex `gpt-6-luna`, and Claude `claude-sonnet-5` variants of
cross-harness message wake, work wake reliability, seat mission work, and restart continuity.
Cross-harness message wake also has omp `openai-codex/gpt-5.6-sol` and `openai-codex/gpt-6-astra`
variants for the omp model comparison.

Codex `gpt-5.6-sol` is the default model judge.

An eval can use a Claude judge for a specific reason. Record the choice in this inventory and the run report.

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
st2 eval ./evals/st2/license-mit
st2 eval ./evals/st2/network-smoke
cargo test -p fractal --test integration examples::
```

Fractal eval orchestration is exercised through repository integration controllers and is deliberately
not a public CLI command. The Rust test validates every Fractal fixture against the normative grammar
and its structural contracts. Paid live runs must use an explicit development controller so the
public product CLI does not acquire an unfenced raw-fixture launcher.
