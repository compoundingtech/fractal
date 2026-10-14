import assert from "node:assert/strict"
import { test } from "node:test"
import shipChange from "../exp1-sdk/ship-change.ts"
import { decide, Session } from "./engine.ts"
import { simulate, type Outcome } from "./sim.ts"

const minute = 60_000
const strip = ({ stats: _, ...rest }: any) => rest

test("feeding events one at a time decides exactly as replaying the whole history", () => {
  let ci = 0
  const trace = simulate(shipChange, { change: "orchard/b", repo: "orchard/app" }, (op, kind): Outcome =>
    op === "build" ? { after: minute, error: { reason: "flaky" } } :
    op === "review" ? "never" :
    op === "review-overdue" ? { after: minute, value: { id: "reassign" } } :
    kind === "gate" ? (++ci < 3 ? { after: minute, error: { reason: "red" } } : { after: minute, value: { answer: "pass", evidence: [] } }) :
    { after: 5 * minute, value: { summary: op, evidence: [], attempt: 1 } })
  const session = new Session(shipChange, trace.history.slice(0, 1))
  for (let i = 1; i < trace.history.length; i++) {
    const incremental = session.apply([trace.history[i]!])
    assert.deepEqual(strip(incremental), strip(decide(shipChange, trace.history.slice(0, i + 1))), `after event ${i}`)
  }
  assert.equal(session.decision().status.state, "completed")
})

test("step gives the same status and commands as a full decision, for every program in the bridge", async () => {
  const { readFileSync, readdirSync } = await import("node:fs")
  const { programs } = await import("../exp5-bridge/js/programs.ts")
  const dir = new URL("../exp5-bridge/fixtures/", import.meta.url)
  // The fixtures come from exp5-bridge/build.sh; without them this test has nothing to check.
  const files = (() => { try { return readdirSync(dir).filter((f) => f.endsWith(".json") && !f.includes("1000")) } catch { return [] } })()
  for (const file of files) {
    const fx = JSON.parse(readFileSync(new URL(file, dir), "utf8"))
    const program = (programs as any)[fx.program]
    const session = new Session(program, fx.history.slice(0, 1))
    for (let i = 1; i < fx.history.length; i++) {
      const fast = session.step([fx.history[i]])
      const full = decide(program, fx.history.slice(0, i + 1))
      assert.deepEqual(fast, { status: full.status, commands: full.commands }, `${file} after event ${i}`)
    }
  }
})
