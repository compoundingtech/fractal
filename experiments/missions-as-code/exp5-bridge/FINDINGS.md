# Experiment 5: our own Rust bridge

Question: can a mission program run inside an isolated JS runtime embedded in Rust, as a pure
function from the run's history to its next actions, with no I/O but the bridge? What does it
cost to start, in memory and per step, is it deterministic, and does it fit the daemon's rules
(one write thread, instant reads, nothing slow in the reconciler)?

**Yes, with QuickJS, if the daemon keeps runs live and the bridge has its own thread.** The same
bundle runs unchanged in QuickJS (rquickjs), Boa and V8 (deno_core). Every decision is
byte-identical to Node's, a runaway program is stopped at its budget, and the engine stays usable
after the stop.

What decides the design is per-step cost:

- **Replaying a long run's whole history for each decision is too slow** to do inline.
- **A live session that is fed only the new events costs tens of microseconds.**

So the bridge belongs on a worker thread that owns live sessions. The reconciler only tells that
worker what changed, and the worker's commands come back through the writer like any other write.

Nothing here comes from or copies another bridge. The bridge is
[src/engines.rs](src/engines.rs), the measurements are [src/main.rs](src/main.rs), and the script
side is [js/entry.ts](js/entry.ts).

## What runs

```
bundle.js (126 KB minified)     Effect 4 + the SDK + engine.ts + programs, one IIFE
  __decide(program, history)    full replay: a fresh Session over the whole history
  __open(program, history)      a live session, replayed once, kept in the engine
  __apply(session, events)      feed new events; returns status and commands only
  __decision(session)           the full decision, with the open list
  __close(session)
```

**The host side.**

- The host registers no functions.
- Each call passes strings in and gets a string back.
- QuickJS gets `Context::full`, which has ECMAScript intrinsics only.
- Boa gets `Context::default`, likewise with no host functions.
- V8 gets deno_core's default `JsRuntime`. That is **not** I/O-free: see "Where it falls short".

**The script side.** `entry.ts` closes what a program could use to see the world:

- `Date.now()` and `new Date()` throw.
- `Math.random` is replaced by a seeded sequence. It cannot simply throw, because Effect uses it
  for object identity hashes (`effect/Hash`).
- Effect's own Clock, Random and Scheduler are replaced inside the engine, as in experiment 1.

A program that reads the clock fails its run with a defect that says so (`reads_clock` in the
results).

**The fixtures.** `js/fixtures.ts` writes eight histories with the decision Node made for each:

- `ship-happy` and `ship-messy`: experiment 1's ship-change, its happy path and every failure path;
- `long-N`: N sequential steps;
- `wide-N`: N parallel steps.

`build.sh` bundles everything and checks the bundle in a bare `node:vm` context.

## Results

`cargo run --release --features quickjs,boa,v8 -- <engine> fixtures`, written to
[results/](results). [js/node-bench.mjs](js/node-bench.mjs) is the Node baseline:
`node:vm` running the same bundle in a bare context.

The host was a shared 16-core build machine **while it ran other builds: load average 36 to 42** for the final
runs. The p50s below are from those runs. Treat p99s and maxima as noise: they include
preemption, and an earlier QuickJS run at a lower load had half the full-replay times (long-1000
48 ms, ship-happy 0.36 ms, start 8.9 ms).

| | QuickJS | Boa | V8 (deno_core) | Node `vm` |
|---|---|---|---|---|
| start: new engine with the bundle loaded, p50 | 20 ms (8.9 quieter) | 195 ms | 41 ms (19 quieter) | 17 ms |
| RSS per loaded engine | 1.7 MB | 4.7 MB | 4.3 MB | |
| JS heap after load | 1.1 MB | (not exposed) | 2.1 MB | |
| **full replay** p50, ship-happy (11 events) | 0.65 ms | 2.6 ms | 0.10 ms | 0.32 ms |
| ship-messy (27 events) | 1.4 ms | 5.5 ms | 0.18 ms | 0.73 ms |
| long-1000 (2001 events) | 95 ms | 502 ms | 15 ms | 49 ms |
| wide-1000 (2001 events) | 86 ms | 332 ms | 6.4 ms | 35 ms |
| per event, full replay | 30–60 µs | 125–250 µs | 2–10 µs | 17–29 µs |
| **live session**, apply p50, long-1000 | 72 µs | 277 µs | 7 µs | 44 µs |
| apply p50, long-100 | 38 µs | 268 µs | 8 µs | 44 µs |
| whole long-1000 run, applied event by event | 263 ms | 793 ms | 23 ms | 62 ms |
| whole wide-1000 run | 17 ms | 165 ms | 5.2 ms | 9.3 ms |
| a live 2000-event session holds | 1.6 MB heap, 2.8 MB RSS | 4.4 MB RSS | 1.3 MB heap, 1.8 MB RSS | |
| matches Node, byte for byte, all 8 fixtures | yes | yes | yes | (the reference) |
| same answer 20–300 times, warm; fresh engine = warm engine | yes | yes | yes | |
| live session's final decision = full replay | yes | yes | yes | yes |
| `for (;;) {}` after a step, 50 ms budget | stopped at 57 ms | stopped at 58 ms (loop limit) | stopped at 54 ms | |
| engine still decides correctly after the stop | yes | yes | yes | |

Notes on the table:

- **Node `vm` is slower than deno_core's V8**, which is the same engine. The likely cause is
  that a `vm` context reaches its globals through interceptors. Node is the reference for
  correctness here, not a competitor.
- **The incremental bench feeds events as a driver appends them:** the scheduled events of one
  decision in one call, each result in its own call.
  - Fed one event at a time, each `scheduled` event of the 1000-wide batch made the session list
    the 999, 998, ... ops still to schedule. That is an O(n²) artifact of the bench, not of a
    driver.
  - It was also why `__apply` first took 3 ms. `Session.step` (in [sdk/engine.ts](../sdk/engine.ts))
    now looks only at the ops the new events touched. A test checks that it agrees with a full
    decision at every event of every fixture.
- **QuickJS's apply cost grows with history** (38 µs at 100 steps, 72 µs at 1000), where V8's
  and Node's stay flat. I did not pin down why. Its full cycle collection scans every live
  object, so a bigger live heap makes each pass longer. A larger first GC threshold made no
  difference I could tell apart from the host's noise.

## Determinism

Determinism comes from the design, and the bridge confirms it:

- the synchronous scheduler gives one interleaving;
- "now" is the time of the event being applied;
- the run's Random is seeded from the run.

All three engines produced exactly Node's bytes for all eight fixtures. That covers parallel
ops, cancellation by a race, retries with backoff, and a three-round CI loop. They repeated
those bytes on every call, and a fresh engine agreed with a warm one.

What the engines could still differ on is pinned by the language: JSON key order, float
formatting and `Array.prototype.sort` stability are all specified. A real bridge pins one bundle
per program version (experiment 4), so a program never meets a different Effect.

One weakness remains: **`Math.random` is reseeded per `__decide` and per `__open`, not per
session.** Two live sessions in one engine share the sequence, so an `__open` reseeds it under the
other session. The sequence only feeds Effect's identity hashes, so no output here depended on it.
But any `HashMap` or `HashSet` of plain objects would iterate in an order that depends on which
sessions ran before. The fix is to give Effect a per-session counter for identity hashes instead
of `Math.random`.

## Isolation

| | Time budget | Memory limit | Host surface by default |
|---|---|---|---|
| QuickJS | **yes**: the interrupt handler checks a deadline; the script throws `interrupted` | **yes**: `set_memory_limit`, and a stack limit | none |
| V8 | **yes**: a watchdog thread calls `terminate_execution`, then the caller cancels it | heap limits exist (not set here) | **deno_core adds `Deno.core` ops and a printing `console`** |
| Boa | **no**: only a loop-iteration limit, which stopped `for (;;) {}` but cannot stop a slow loop that finishes, or slow code without loops | no | none |

## Against the daemon's rules

**One write thread.** `decide` never needs the writer.

- **Input:** the run's journal, read from a read connection.
- **Output:** commands, which become claims through the normal write path, one job each. The
  writer's cost per op is the cost of any claim write, measured in experiment 2.
- **The bridge runs on its own thread**, never on the writer's or an async worker's. Both
  engines tie an engine to its thread anyway:
  - rquickjs without its `parallel` feature;
  - V8, whose isolates are entered on creation and must be dropped in reverse order on their
    thread. The bench aborted inside V8 until it did that.

**Instant reads.** Clients never call the bridge. They read the journal and the op runs
(experiment 2), which are ordinary claims. A plan preview (experiment 3) is computed once at
publish and stored, not computed on read.

**Nothing slow in the reconciler.** A full replay is too slow to run inline:

- 15 ms (V8) to 95 ms (QuickJS) for a 2000-event run, and 500 ms in Boa;
- and that cost is paid on every decision, so a run's total grows with the square of its length.

A live session's apply is cheap at p50 (QuickJS 14 to 72 µs), but its tail is not bounded by
anything but the budget. So:

1. **The reconciler only enqueues.** "Run R has events after seq N" goes to the bridge worker,
   and the reconciler never waits on the bridge.
2. **The worker owns the engines and live sessions.** It applies the new events and submits the
   resulting commands as a write job.
3. **Every call has a budget**, 50 ms here. A call that runs past it faults that run (a mission
   fault, as for a step out of time) and leaves the engine and every other session in it working,
   as measured.
4. **Commands are a delta.** `__apply` returns status and commands, not the open list. The full
   decision is read only when a person or the driver needs it.
5. **Sessions are evicted when idle.** A run waiting days on a person needs no session. One is
   rebuilt by full replay when the run's next event arrives, which is a cold start for that run:
   about 25 to 60 µs per event in QuickJS, on the worker.
6. **One engine per program version per worker**, each about 1.7 MB in QuickJS.

**Memory.** A live session holds its history and the program's fibers: about 1.3 to 1.6 MB for a
2000-event run, less for short ones. A hundred long, busy runs is about 160 MB, which is why idle
sessions are evicted.

**Restart.** After a daemon restart, sessions are rebuilt lazily as their runs receive events, so
the restart itself costs nothing. Rebuilding every live run at once would be N × replay cost, on
the worker and off the writer.

## Engines compared

- **QuickJS (rquickjs 0.14): the choice.**
  - Small: a 2.3 MB static library, 1.7 MB per engine.
  - Starts in 9 to 20 ms with the 126 KB bundle.
  - A real time budget and memory limit, and nothing exposed by default.
  - Slower per step than V8, by 6 to 13×, which only matters for full replays, and those leave
    the hot path.
- **V8 (deno_core 0.412): fastest, and heavy.**
  - Fastest by far: 7 µs per apply, 15 ms for a 2000-event full replay.
  - Its static library is 187 MB before linking; the bench binary with all three engines is
    86 MB.
  - Isolates have thread rules that crash the process when broken.
  - deno_core's default runtime is not I/O-free.
  - The startup snapshot that would cut its start time crashes (below).
  - Worth it only if full replays of long runs must stay on a hot path.
- **Boa (0.22): no.**
  - Four to seven times slower than QuickJS.
  - The slowest start (195 ms).
  - No time budget, memory limit or heap statistics.

## Where it falls short

1. **The V8 startup snapshot does not work with Effect.**
   - A snapshot taken after running any Effect program aborts V8 on restore:
     `Check failed: LinearSearch(*desc->GetKey(), descriptor_number) == InternalIndex::NotFound()`.
   - [snapshot-repro.sh](snapshot-repro.sh) reduces it to `Effect.runSync(Effect.succeed(1))`.
   - Fixed hash seeds and `--predictable` change nothing.
   - Small bundles with registered symbols, classes, maps and weak maps restore fine.
   - I did not find the cause, so V8's start time with a snapshot is unmeasured.
2. **deno_core is not a sandbox as configured.** Its default runtime gives every script
   `Deno.core` (print, resource reads and writes, timers) and a `console` that prints. A pure
   bridge on V8 would delete those before loading, or use the `v8` crate without deno_core.
3. **`Math.random` is shared across sessions in one engine** (see Determinism).
4. **Boa cannot enforce a time budget.** Its loop limit happens to stop `for (;;) {}` at about the
   same time; a slow loop that finishes is never stopped.
5. **These are one program's numbers on a loaded host.** The bundle registers every program.
   - A real bridge loads one content-addressed bundle per version (experiment 4).
   - Load and start costs are per version, not per program.
6. **The bridge is not wired to the driver.** Experiment 2's driver still decides in Node. The
   bridge's interface is the same `decide` and `Session` with strings at the edge, so the swap is
   mechanical, but it was not done.

## Verdict

An embedded engine as a pure function from history to actions works, and QuickJS is the engine
for it. It is deterministic, small, quick to start, and it can be stopped.

What makes it fit the daemon is not the engine but the shape around it:

- live sessions;
- a bridge thread that the reconciler only notifies;
- a budget per call that faults the run, not the daemon;
- commands written back through the one writer.

Without live sessions, a long run's cost per decision grows with its history, and that is too
slow for the daemon.
