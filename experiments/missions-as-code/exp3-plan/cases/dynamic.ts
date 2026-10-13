// Hard case 2: the shape depends on input data and on results.
//   - fan-out over input.services (count unknown until the run starts)
//   - an op ID computed at runtime from a step's result
//   - an unbounded while loop whose exit depends on a gate

import { Effect } from "effect"
import { gate, mission, scope, step } from "../../sdk/mission.ts"

const laneFor = (summary: string) => (summary.includes("hotfix") ? "fast" : "slow")

export default mission("orchard/dynamic", (input: { services: Array<string>; team: string }) =>
  Effect.gen(function* () {
    const plan = yield* step("plan", { title: "Plan the rollout", assignedTo: "agent/orchard/planner" })
    yield* Effect.forEach(
      input.services,
      (service) => step(`deploy-${service}`, { title: `Deploy ${service}`, assignedTo: "agent/orchard/deployer" }),
      { concurrency: 2 }
    )
    const lane = laneFor(plan.summary)
    yield* step(["ship", lane, input.team].join("-"), { title: "Ship", assignedTo: "agent/orchard/deployer" })
    let round = 0
    while (true) {
      round++
      const green = yield* gate("healthy", { exec: "./check-health.sh" }).pipe(Effect.result, scope(`watch-${round}`))
      if (green._tag === "Success" || round >= 4) break
      yield* Effect.sleep("10 minutes")
    }
    return { lane, rounds: round }
  })
)
