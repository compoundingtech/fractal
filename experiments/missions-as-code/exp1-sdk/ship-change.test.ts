import assert from "node:assert/strict"
import { test } from "node:test"
import { decide } from "../sdk/engine.ts"
import { simulate, type Script } from "../sdk/sim.ts"
import shipChange from "./ship-change.ts"

const minute = 60_000
const hour = 60 * minute
const input = { change: "orchard/feature-a", repo: "orchard/app" }

const ok = (after: number, summary = "done") => ({ after, value: { summary, evidence: [`doc/${summary}`], attempt: 1 } })
const happy: Script = (op, kind) => {
  if (kind === "gate") return { after: 10 * minute, value: { answer: "pass", evidence: [] } }
  return ok(20 * minute, op)
}

const scheduled = (log: Array<string>) => log.filter((l) => l.includes(" schedule ")).map((l) => l.split(" ").slice(2).join(" "))

test("parallel steps open together and the happy path completes", () => {
  const trace = simulate(shipChange, input, happy)
  assert.deepEqual(trace.decisions[0]!.commands.map((c) => c.op), ["build", "docs"])
  assert.deepEqual(trace.decisions.at(-1)!.status, { state: "completed", value: { shipped: true, rounds: 1, review: "review" } })
  assert.deepEqual(scheduled(trace.log), ["step build", "step docs", "step review", "timer timer", "gate round-1/ci"])
  // The escalation timer was cancelled once the review answered.
  assert.ok(trace.log.some((l) => l.endsWith("cancel timer")))
})

test("a failing build retries with durable backoff", () => {
  const trace = simulate(shipChange, input, (op, kind, spec) =>
    op === "build" || op === "build~2" ? { after: 5 * minute, error: { reason: "flaky linker" } } : happy(op, kind, spec)
  )
  assert.deepEqual(scheduled(trace.log).slice(0, 6), [
    "step build", "step docs", "timer timer", "step build~2", "timer timer~2", "step build~3"
  ])
  // Backoff is 1m then 2m, measured on the durable clock.
  const timers = trace.history.filter((e) => e.type === "scheduled" && e.kind === "timer")
  assert.equal(timers.length, 3) // two backoffs and the review's escalation deadline
  assert.equal(trace.decisions.at(-1)!.status.state, "completed")
})

test("an overdue review escalates to a person and the answer reroutes the work", () => {
  const trace = simulate(shipChange, input, (op, kind, spec) => {
    if (op === "review") return "never"
    if (op === "review-overdue") return { after: 30 * minute, value: { id: "reassign" } }
    return happy(op, kind, spec)
  })
  const log = trace.log.join("\n")
  assert.match(log, /\+04:20 completed timer\n\+04:20 cancel review\n\+04:20 schedule ask review-overdue/)
  assert.ok(scheduled(trace.log).includes("step review-reassigned"))
  assert.equal(trace.decisions.at(-1)!.status.state, "completed")
})

test("the CI loop fixes and checks again, round by round", () => {
  let ciCalls = 0
  const trace = simulate(shipChange, input, (op, kind, spec) => {
    if (kind === "gate") return ++ciCalls < 3 ? { after: 10 * minute, error: { reason: "orchard-ci failed" } } : happy(op, kind, spec)
    return happy(op, kind, spec)
  })
  assert.deepEqual(scheduled(trace.log).slice(-5), ["gate round-1/ci", "step round-1/fix", "gate round-2/ci", "step round-2/fix", "gate round-3/ci"])
  assert.deepEqual((trace.decisions.at(-1)!.status as any).value.rounds, 3)
})

test("on failure the mission files a report and still fails", () => {
  const trace = simulate(shipChange, input, (op, kind, spec) =>
    kind === "gate" ? { after: 10 * minute, error: { reason: "red" } } : happy(op, kind, spec)
  )
  assert.equal(scheduled(trace.log).at(-1), "step report-failure")
  assert.deepEqual(trace.decisions.at(-1)!.status, { state: "failed", error: { _tag: "CiNeverPassed", rounds: 3 } })
})

test("replay is deterministic: every prefix decides the same way twice", () => {
  const trace = simulate(shipChange, input, (op, kind, spec) =>
    op === "build" ? { after: 5 * minute, error: { reason: "flaky" } } : op === "review" ? { after: 5 * hour, value: { summary: "late" } } : happy(op, kind, spec)
  )
  for (let i = 1; i <= trace.history.length; i++) {
    const prefix = trace.history.slice(0, i)
    assert.deepEqual(decide(shipChange, prefix), decide(shipChange, prefix))
  }
})

test("a result for an op the program never asked for is a divergence, not a guess", () => {
  const trace = simulate(shipChange, input, happy)
  const renamed = { ...shipChange, run: (i: any) => shipChange.run({ ...i }) }
  const edited = trace.history.map((e) => ("op" in e && e.op === "docs" ? { ...e, op: "documentation" } : e))
  const decision = decide(renamed, edited)
  assert.equal(decision.divergence?.op, "documentation")
  assert.deepEqual(decision.commands, [])
})
