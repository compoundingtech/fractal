// Helpers in another module, the way a team would factor shared mission shapes.

import { Effect } from "effect"
import { gate, step } from "../../sdk/mission.ts"

// A step followed by a person's approval. The IDs come from the caller's argument.
export const reviewed = (name: string, assignee: string) =>
  step(name, { title: `Do ${name}`, assignedTo: assignee }).pipe(
    Effect.flatMap((done) =>
      gate(`${name}-approved`, { human: { reviewer: "person/wren", question: `Approve ${name}? ${done.summary}` } })
    )
  )

// Build and test one component; test retries once.
export function buildAndTest(component: string) {
  return Effect.gen(function* () {
    const built = yield* step(`build-${component}`, { title: `Build ${component}`, assignedTo: "agent/orchard/builder" })
    return yield* step(`test-${component}`, {
      title: `Test ${component}`,
      assignedTo: "agent/orchard/builder",
      context: { build: built.summary }
    }).pipe(Effect.retry({ times: 1 }))
  })
}
