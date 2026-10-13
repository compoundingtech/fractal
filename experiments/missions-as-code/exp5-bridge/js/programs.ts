// Programs the bridge can run, by name. A real bridge would load one content-addressed bundle
// per program version; one bundle with a registry keeps this experiment small.

import { Effect } from "effect"
import { mission, scope, step } from "../../sdk/mission.ts"
import shipChange from "../../exp1-sdk/ship-change.ts"

// A long sequential program: n steps, for measuring how replay cost grows with history.
export const longRun = mission("orchard/long-run", (input: { steps: number }) =>
  Effect.gen(function* () {
    let last = ""
    for (let i = 0; i < input.steps; i++) {
      const r = yield* step("work", { title: `Item ${i}`, assignedTo: "agent/orchard/builder", context: { last } }).pipe(scope(`item-${i}`))
      last = r.summary
    }
    return { last }
  })
)

// A wide program: n parallel steps at once.
export const wideRun = mission("orchard/wide-run", (input: { steps: number }) =>
  Effect.gen(function* () {
    const results = yield* Effect.forEach(
      Array.from({ length: input.steps }, (_, i) => i),
      (i) => step(`shard-${i}`, { title: `Shard ${i}`, assignedTo: "agent/orchard/builder" }),
      { concurrency: "unbounded" }
    )
    return { count: results.length }
  })
)



// Hostile programs: the bridge must stop these without stalling its caller.
export const spin = mission("orchard/spin", (_: unknown) =>
  Effect.gen(function* () {
    yield* step("first", { title: "First", assignedTo: "agent/orchard/builder" })
    for (;;) {} // a program bug: never yields again
  })
)
export const readsClock = mission("orchard/reads-clock", (_: unknown) =>
  Effect.gen(function* () {
    const id = `stamp-${Date.now()}` // nondeterministic: a replay would ask for a different op
    return yield* step(id, { title: "Stamped", assignedTo: "agent/orchard/builder" })
  })
)
export const programs = { [shipChange.name]: shipChange, [longRun.name]: longRun, [wideRun.name]: wideRun, [spin.name]: spin, [readsClock.name]: readsClock }
