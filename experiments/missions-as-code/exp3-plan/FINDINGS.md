# Experiment 3: seeing a program's plan before it runs

Question: can we extract a mission program's plan (steps, gates, asks, waits, dependencies, branches, loops, parallel groups, retries, timeouts, escalations) before it runs, the way `st launch preview` shows a KDL mission's static dependency graph? How far does that go?

Short answer: **an outline, yes. A plan you can rely on the way you rely on a KDL graph, no.** Both extractors below give a useful picture of ordinary programs. Neither can promise the picture is complete, and each is defeated by a short list of everyday idioms. The honest product is "the plan as far as we can see, with the unknowns marked", plus the run's history, which is always exact.

## What was built

| File | What it does |
| --- | --- |
| `static.ts` | Reads the source with the TypeScript compiler and type checker. Walks `Effect.gen` bodies, follows helpers into other modules, and outputs a plan tree, a node/edge graph, and Mermaid. |
| `probe.ts` | Runs the real program through `sdk/engine.ts` `decide()` against made-up histories. Explores outcomes breadth-first (each open op succeeds, fails, takes each named answer, or its timer fires), within a state and depth bound. |
| `compare.ts` | Matches the two: static templates such as `round-${round}/ci` against probed IDs such as `round-2/ci`, and `build.2` against `build`. |
| `cases/` | `helpers.ts` + `lib.ts` (helpers across modules), `dynamic.ts` (fan-out over input, an ID from a result, an unbounded `while`), `opaque.ts` (a generic wrapper, `Effect.all(xs.map(...))`, an op picked from a table, an ID from a function). |
| `print.ts`, `print-probe.ts`, `probe-ports.ts` | CLIs. `out/` has sample Mermaid and JSON. |
| `plan.test.ts` | 8 tests, all passing. |

## Static extraction: what it finds

On `exp1-sdk/ship-change.ts` it finds everything the author would draw (`out/ship-change.static.mmd`):
- **Op shapes:** all 8, with their seats: `build`, `docs`, `review`, `review-overdue`, `review-reassigned`, `round-${round}/ci`, `round-${round}/fix`, `report-failure`.
- **Parallel group:** `build + docs`.
- **Annotations:**
  - `retry { times: 2, schedule: exponential("1 minute") }` on build.
  - `escalate after 4 hours` from review to the ask.
  - The ask's two-way branch, with its `fail Stopped` arm.
  - `tapError` → `report-failure` → fail.
- **The CI loop:** with its bound (×3) and the scope template `round-${round}/...`.
- **Which ops read which results:** review reads build and docs; the ask branch reads review-overdue; the CI branch reads `round-${round}/ci`.

It also handles:
- **Helpers across modules.** The type checker resolves helpers, and literal arguments are filled into IDs (`test-${component}` → `test-api`, `test-web`). Effects and callbacks passed into helpers (`withAudit("migrate", step(...))`, `options.round`) are followed, and literal fields of an options object (`job.first.id`, `options.maxRounds`) resolve.
- **Fan-out idioms.** `Effect.forEach(xs, f)` and `Effect.all(xs.map(f))` become "for each xs, N at a time" with the body's template (`deploy-${service}`). The width stays unknown, which is the truth.
- **Calls it can't follow are marked, not dropped.** A call whose type is `Effect<...>` but whose body it can't reach (`suites[input.mode]()`) becomes an opaque node with a note. Without the type check these ops disappeared silently.

Cost: about 0.5 s per file, almost all of it creating the TypeScript program. That is fine at publish time.

## Static extraction: where it breaks

1. **Computed IDs.** `["ship", lane, input.team].join("-")`, `ticket(input.change)`, and a `for (const slice of [...])` loop variable all render as `<expr>`. That is correct, but the reader only learns "some step".
2. **Choice by data.** `suites[input.mode]()` is opaque: there's no way to know `quick-check` or `full-suite` exists.
3. **Loops without a literal bound.** `while (true)` and `for (;; round++)` show as "unbounded"; the real exit is buried in a condition the extractor reads only as text.
4. **Branch conditions are text.** The extractor knows `verdict.summary.startsWith("GO")` reads design-intake. It can't tell whether either arm is reachable.
5. **The long tail of Effect.** Every combinator is a special case. The first real port, `exp6-port/ledger-changes.ts`, needed six more idioms before its plan was right:
   - `shipped.push(yield* land(...))`: a yield inside a call argument.
   - `return Effect.gen(...)` from a plain function.
   - `[...] as const`.
   - Helper returns that continue in the caller (call frames).
   - `opEffect` used directly by local SDK extensions.
   - A cleanup step (`Effect.ensuring`) after a body that only returns.

   Each one was a missing op or a wrong edge until fixed. Expect a steady trickle of these.
6. **Approximations.** Failure edges are drawn as "any of N ops above fails", not per op. `Effect.all` cancelling its siblings on failure is not drawn. Early returns from nested loops inside a helper are drawn only approximately.

A static extractor that is right for idiomatic programs is a few hundred lines. One that is right for any program is impossible.

## Probe extraction: what it finds

Because it runs the program, the probe sees through everything static gets stuck on, for the input it is given:
- **ship-change:** 213 states, complete, about 120–230 ms. It reaches every op, including `build.2`, `build.3` and `round-3/fix`. It finds both review paths and all 16 distinct endings: shipped after 1–3 rounds by either reviewer, CiNeverPassed, Stopped, and each step's failure.
- **helpers:** all 9 ops, including `test-api.2`.
- **dynamic:** `deploy-auth`, `deploy-billing` and `deploy-search`, the two that open together (concurrency 2), and `ship-slow-lichen`.
- **opaque:** `audit-migrate`, `canary-north`, `canary-south`, `full-suite` and `ticket-feature-b`.

## Probe extraction: where it breaks

1. **It only knows the given input.** It reports `full-suite` and never mentions `quick-check`.
2. **Branches on a result's content follow made-up values.** This is the decisive failure.
   - On `ledger-changes.ts` with default values the probe reports complete=true after 26 states and has never seen 8 of the 14 op shapes. The whole path to main is missing, because the made-up intake summary never starts with "GO".
   - Trying both "GO" and "NO-GO" for every step reaches every op but hits the 20,000-state limit (incomplete, about 1.5–3 s).
   - `refresh-then-answer.ts` goes from 700 states to the same limit.
3. **State explosion.** Outcomes multiply across parallel ops, retries and loops. Treating states with the same resolved outcomes as one keeps ship-change at 213 states; the ports show how quickly real programs outgrow that.
4. **Unbounded loops** end the search at the depth bound and report complete=false, which at least is honest.
5. **Auto-numbered ops are not stable names.** The review's deadline timer is `timer`, `timer.2` or `timer.3` depending on how many retry delays ran first, and on another path `timer` is a retry delay. A graph keyed by op ID merges different ops. Replaying history across versions hits the same fragility. Timers should be named after what they guard (`review/deadline`), not counted.
6. **It runs the program.** That is safe only while `decide()` stays pure.

## Side by side

| | KDL (`st launch preview`) | Static | Probe |
| --- | --- | --- | --- |
| Complete? | Yes, by construction | No guarantee; unknowns are marked | Only for the given input and made-up values; can say "complete" while missing paths |
| Exact IDs | Yes | Literal and template; computed IDs are `<expr>` | Yes, for the paths it reached |
| Dependencies | `depends-on` | Order, plus which ops read which results | Which ops opened after which outcome |
| Branches | `depends-on failed/terminal`, gates, loops | Conditions as text | Only the arms its values reached |
| Loops | `loop max-rounds` | Bound when literal | Unrolled to the depth bound |
| Fan-out width | Explicit steps | Unknown | For the given input |
| Cost | Parse | ~0.5 s per file | ms to seconds; exponential worst case |

`ledger-changes.kdl` has 6 steps and an exact graph only because the messy path to main (CI, fix, intake, queue, ejection) lives in prose inside step goals. The program writes that path as code: static extraction finds 14 op shapes and the loops connecting them, which is more of the real plan than the KDL graph shows. But it is an outline with templated IDs and text conditions, not a graph a reviewer can approve as exact.

## How far does it go

- **Yes:** a publish-time outline from static extraction, for review, a docs page, or a rough preview in stui and the phone. It shows the seats involved, the steps, what runs in parallel, and where the loops, deadlines, escalations and failure handlers are. Nothing runs to produce it.
- **Yes, with care:** the probe as test coverage ("every op is reachable", "these endings exist"). Its "complete" must never be shown as a guarantee.
- **No:** an exact plan before the run for arbitrary programs. Anything that needs the complete set of ops can't rest on extraction: a reviewer approving the exact steps, an authorization check listing every seat a run may wake, or reasoning over which steps a revision changed for `revisions="human-only"`. It needs either a restricted language (KDL) or a check at the moment each op opens, against rules rather than a list.

KDL's graph is exact because the hard parts are not in it. A program-based mission moves those parts from prose into code: runnable and testable, but the up-front graph becomes a forecast. The run's history is always exact, and web, tui and iOS should treat it as the truth.

Recommendation: ship the static outline with opaque and dynamic nodes shown as such. Require stable names for every op the author cares about, including timers. Offer no "plan complete" badge.
