# Missions as code: six experiments

Can an st mission be a program, an Effect.gen where each yield is a step, gate, ask or wait,
instead of a KDL graph? These experiments build enough to answer that with running code and
measurements. All names in them are invented. Nothing here is wired into st; it sits beside it
and talks to st only through its CLI.

**The short answer:**

- **As an engine:** yes. A small SDK, a pure replay core and a driver that lives outside the
  daemon run real missions through st and survive being killed.
- **As a replacement for KDL:** no. A program earns its place only where KDL cannot hold the
  control flow, such as a change's loop through CI, intake and the queue. There it fits as one
  programmed part inside a declarative mission.
- **What it costs:**
  - a weaker preview before the run (experiment 3);
  - weaker visibility while it runs (experiment 2);
  - naming and versioning discipline (experiments 1 and 4);
  - an engine in the daemon only if the cost per step stays small, which it does when runs are
    kept live (experiment 5).

| # | Experiment | Code | Result |
|---|---|---|---|
| 1 | An Effect mission SDK | [sdk/](sdk), [exp1-sdk/](exp1-sdk) | Works; about 300 lines, because Effect already has retries, timeouts, races, loops and parallelism. [FINDINGS](exp1-sdk/FINDINGS.md) |
| 2 | Durable execution as an st exec | [exp2-driver/](exp2-driver) | Works through st in a lab instance; survives SIGKILL and a crash between acting and recording. The clients see the ops as separate runs, without nesting, a status line or timers. [FINDINGS](exp2-driver/FINDINGS.md) |
| 3 | Seeing the plan before it runs | [exp3-plan/](exp3-plan) | An outline, yes; an exact plan, no. [FINDINGS](exp3-plan/FINDINGS.md) |
| 4 | Changing a running mission | [exp4-hmr/](exp4-hmr) | Works live: a run moves only if the new version replays its history, otherwise it keeps its own. Versions are content-addressed documents. [FINDINGS](exp4-hmr/FINDINGS.md) |
| 5 | Our own Rust bridge | [exp5-bridge/](exp5-bridge) | See [FINDINGS](exp5-bridge/FINDINGS.md). |
| 6 | Two real missions ported | [exp6-port/](exp6-port) | Simple missions: keep KDL. The change path: the program wins, because KDL holds it only as prose. [COMPARISON](exp6-port/COMPARISON.md) |

## Running it

```sh
cd experiments/missions-as-code
npm install                                   # Effect 4.0.4, esbuild, TypeScript
npx tsc -p .                                  # typecheck everything
node --test sdk/*.test.ts exp1-sdk/*.test.ts exp3-plan/*.test.ts exp4-hmr/*.test.ts exp6-port/*.test.ts
node exp4-hmr/matrix.ts                       # the hot reload matrix
exp5-bridge/build.sh                          # bundle + fixtures, checked in a bare V8 context
(cd exp5-bridge && cargo run --release --features quickjs -- quickjs fixtures)
```

Experiments 2 and 4 run against a throwaway st instance; their READMEs say how
(`exp2-driver/lab-st`, `lab/kill-test.sh`, `exp4-hmr/lab-demo.sh`).

## The design in one paragraph

A program's run is an append-only history: started, scheduled, completed, failed, cancelled,
upgraded. `decide(program, history)` is pure. It replays the history through the program on a
synchronous scheduler, with Effect's Clock, Random and Scheduler replaced so time and order come
from the history, and returns the ops the program waits on plus what to do next. A driver
enacts that: each op becomes an ordinary st mission run with a deterministic ID, and each event
becomes a custom claim. The run's state lives in the graph, never in the driver. A version is a
content-addressed bundle that carries its own engine, so every version replays forever.
