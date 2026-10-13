// Hard case 3: shapes a reader of the source cannot pin down.
//   - a generic wrapper that takes an effect and adds an audit step
//   - Effect.all over a collection mapped from input
//   - an op chosen from a table at runtime
//   - an ID derived by a function of the input

import { Effect } from "effect"
import { mission, step } from "../../sdk/mission.ts"

const withAudit = <A, E>(name: string, work: Effect.Effect<A, E>) =>
  work.pipe(Effect.tap(() => step(`audit-${name}`, { title: `Audit ${name}`, assignedTo: "agent/orchard/auditor" })))

const suites = {
  quick: () => step("quick-check", { title: "Quick check", assignedTo: "agent/orchard/builder" }),
  full: () => step("full-suite", { title: "Full suite", assignedTo: "agent/orchard/builder" })
}

const ticket = (change: string) => `ticket-${change.split("/").at(-1)}`

export default mission("orchard/opaque", (input: { regions: Array<string>; mode: "quick" | "full"; change: string }) =>
  Effect.gen(function* () {
    yield* withAudit("migrate", step("migrate", { title: "Migrate", assignedTo: "agent/orchard/builder" }))
    yield* Effect.all(
      input.regions.map((region) => step(`canary-${region}`, { title: `Canary ${region}`, assignedTo: "agent/orchard/deployer" })),
      { concurrency: "unbounded" }
    )
    yield* suites[input.mode]()
    yield* step(ticket(input.change), { title: "Close the ticket", assignedTo: "agent/orchard/builder" })
    return "done"
  })
)
