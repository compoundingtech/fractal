# @fractal/declarations

Typed authoring and deterministic KDL v2 encoding for Fractal agents, missions and schedules. This is a declaration package, not the daemon HTTP client (`@fractal/client`). It has no runtime dependencies.

```ts
import { encodeKdl } from '@fractal/declarations'
import type { Agent, Mission, Schedule } from '@fractal/declarations'

const editor = {
  id: 'docs/editor',
  workspace: '.',
  launch: { command: 'true' },
} as const satisfies Agent

const writer = {
  id: 'docs/writer',
  workspace: '.',
  launch: { argv: ['review-docs', '--daily'] },
  under: { agent: editor, reason: 'The editor reviews the writer’s work.' },
} as const satisfies Agent

const review = {
  id: 'docs/daily-review',
  state: 'ready',
  reportTo: editor,
  goals: ['Review the documentation each day.'],
  completion: 'all-steps-exhausted',
  steps: [{
    id: 'review',
    assignedTo: writer,
    goals: ['Review the documentation and report corrections.'],
  }],
} as const satisfies Mission

const daily = {
  id: 'docs/daily-review',
  host: 'local',
  every: '1d',
  anchor: '2026-01-01T09:00:00Z',
  catchUp: 'latest',
  work: {
    mission: review,
    revision: 'b283ff12e887f06fbeb2da3094b10f8fd2b9b7840f7af8031445b124829f25f7',
    workspace: '.',
  },
} as const satisfies Schedule

const kdl = encodeKdl({ agents: [editor, writer], missions: [review], schedules: [daily] })
```

## Contract

- IDs are bare declaration IDs, such as `docs/editor` and `docs/daily-review`. An `AgentId` must contain `.` or `/`, such as `remote.editor` or `docs/editor`: the parser preserves exactly those identities. Simple agent IDs such as `editor` are host-folded by the parser and are deliberately unsupported, because this SDK does not know or guess the default host.
- Agent references accept an `Agent` object or a qualified string such as `agent/docs/editor` or `agent/remote.editor`; the same ID rule applies to declarations, `under`, mission `reportTo` and step `assignedTo`. The encoder throws an `Error` before returning KDL if an untyped caller supplies a host-folded agent ID/reference. Schedule work accepts a `Mission` object or its **bare** ID string, plus a separate explicit revision. Mission IDs are never host-folded, so simple mission IDs remain supported. Object references encode their ID; they do not implicitly emit the referenced declaration or compute revisions.
- Launch is exactly one of `command: string` or a nonempty readonly `argv` tuple. Use `as const` to preserve tuple inference.
- Mission states are `draft`, `ready` and `retired`; completion is `all-steps-exhausted`. Schedule catch-up is a discriminated union: `all` requires `maxCatchUp`, while `latest` and `skip` forbid it. The bound must be a positive u32, as checked by daemon validation.
- Mission goals are readonly tuples of 1–3 strings; step goals are readonly tuples of 0–3 strings. A mission's step list has no parser-imposed minimum or maximum, so an empty step list is supported.
- Collections are readonly and individually optional at the document root, but at least one collection must be nonempty, matching the publication parser. Input order is preserved within collections, goals and steps. Top-level kinds emit agents, missions, then schedules.
- Required fields cover agent workspace and launch; mission state, report recipient, goals, completion and steps; step assignee and goals; interval schedule host, every, anchor and work; and work mission, revision and workspace. Optional `under` has one agent and reason. Some are stricter than parser defaults. Singleton fields serialize once; collection fields serialize their elements. The encoder emits only one completion condition, one agent launch and one interval timing form; competing completion conditions, harness launches and calendar/one-time schedules are outside this SDK.
- Scalar and cross-declaration validation remains with the daemon: ID/name/reference syntax and lengths, duplicate IDs (including step IDs), reference resolution and ownership, registered variable interpolation, `under` reason length (1–500 bytes), interval duration and UTC anchor syntax, positive-u32 catch-up bounds, exact 64-hex mission revisions, nonblank schedule workspace, and the 16 MiB document limit. Types do not validate numeric ranges or string contents; validate encoded KDL before publication.
- Output starts with `version 2`, uses two-space indentation and newline-delimited children, separates top-level blocks with a blank line and ends with a newline. Strings escape quotes, backslashes, newlines, carriage returns and tabs; other controls use KDL v2 braced Unicode escapes, including `\u{8}` and `\u{c}` for backspace and formfeed.
- This package exposes only the supported declaration vocabulary, not runtime state, arbitrary KDL children, hooks, step dependencies or cron expressions. It does not publish declarations, start runs or replace daemon validation. Validate the emitted KDL with Fractal before publication.

## Generated source and checks

`index.ts` is generated by the Rust client code generator. Do not edit it by hand. From the repository root:

```sh
cargo run -p fractal-client-codegen
cargo run -p fractal-client-codegen -- --check
pnpm --filter @fractal/declarations test
pnpm --filter @fractal/declarations typecheck
```

The Node contract suite transpiles the generated TypeScript with the workspace’s TypeScript compiler. Its exact golden fixture, `fixtures/daily-review.kdl`, represents the `st validate` fixture `crates/fractal/tests/fixtures/validate-revisions.kdl` declarations with canonical multiline completion formatting. The suite also covers references, quoting, argv, collection ordering, readonly inputs and supported enum variants. `type-tests.ts` uses positive examples and compile-time negative assertions without suppressing diagnostics. Both package checks run in `scripts/ci-typescript-client`.
