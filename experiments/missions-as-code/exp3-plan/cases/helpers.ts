// Hard case 1: the plan lives in helper functions in another module.

import { Effect } from "effect"
import { mission, step } from "../../sdk/mission.ts"
import { buildAndTest, reviewed } from "./lib.ts"

export default mission("orchard/helpers", (_input: { change: string }) =>
  Effect.gen(function* () {
    yield* reviewed("design", "agent/orchard/architect")
    const [api, web] = yield* Effect.all([buildAndTest("api"), buildAndTest("web")], { concurrency: "unbounded" })
    yield* step("release", { title: "Release", assignedTo: "agent/orchard/builder", context: { api: api.summary, web: web.summary } })
    return "released"
  })
)
