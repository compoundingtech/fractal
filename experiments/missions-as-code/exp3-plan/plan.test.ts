import assert from "node:assert/strict"
import { test } from "node:test"
import { fileURLToPath } from "node:url"
import { Effect } from "effect"
import { gate, mission, scope } from "../sdk/mission.ts"
import type { Json } from "../sdk/history.ts"
import shipChange from "../exp1-sdk/ship-change.ts"
import dynamic from "./cases/dynamic.ts"
import helpers from "./cases/helpers.ts"
import opaque from "./cases/opaque.ts"
import { compare } from "./compare.ts"
import { probe } from "./probe.ts"
import { extractFile, staticOps, toGraph, toMermaid, type PlanNode } from "./static.ts"

const file = (rel: string) => fileURLToPath(new URL(rel, import.meta.url))
const ids = (plan: PlanNode) => staticOps(plan).map((o) => o.id)
const find = (node: PlanNode, pred: (n: PlanNode) => boolean): Array<PlanNode> => {
  const out: Array<PlanNode> = []
  const visit = (n: PlanNode) => {
    if (pred(n)) out.push(n)
    if (n.type === "seq" || n.type === "parallel") n.items.forEach(visit)
    if (n.type === "branch") n.arms.forEach((a) => visit(a.body))
    if (n.type === "loop" || n.type === "frame") visit(n.body)
    if (n.type === "wrap") {
      visit(n.inner)
      n.annotations.forEach((a) => a.handler && visit(a.handler))
    }
  }
  visit(node)
  return out
}

const shipInput = { change: "orchard/feature-a", repo: "orchard/app" }

test("static: ship-change's steps, parallel group, loop, branches and handlers", () => {
  const { mission: name, plan, diagnostics } = extractFile(file("../exp1-sdk/ship-change.ts"))
  assert.equal(name, "orchard/ship-change")
  assert.deepEqual(diagnostics, [])
  assert.deepEqual(ids(plan), [
    "build", "docs", "review", "review-overdue", "review-reassigned", "round-${round}/ci", "round-${round}/fix", "report-failure"
  ])
  const parallel = find(plan, (n) => n.type === "parallel")
  assert.deepEqual(parallel.map((p) => ids(p)), [["build", "docs"]])
  const loop = find(plan, (n) => n.type === "loop")[0] as Extract<PlanNode, { type: "loop" }>
  assert.equal(loop.bound, 3)
  const annotations = find(plan, (n) => n.type === "wrap").flatMap((w) => (w as any).annotations.map((a: any) => `${a.type} ${a.detail}`))
  assert.ok(annotations.includes("retry { times: 2, schedule: Schedule.exponential(\"1 minute\") }"))
  assert.ok(annotations.includes("escalate review-deadline after 4 hours"))
  assert.ok(annotations.includes("timer-name build-backoff"))
  assert.ok(annotations.includes("on-failure then fail"))
  assert.ok(annotations.includes("scope round-${round}"))
  // Data flow: review reads the build and docs results; the CI branch reads the gate.
  const review = find(plan, (n) => n.type === "op" && n.id === "review")[0] as any
  assert.deepEqual(review.uses.sort(), ["build", "docs"])
  const branches = find(plan, (n) => n.type === "branch") as Array<any>
  assert.deepEqual(branches.map((b) => [b.condition, b.uses]), [
    ['answer.id === "reassign"', ["review-overdue"]],
    ['ci._tag === "Success"', ["round-${round}/ci"]]
  ])
  const mermaid = toMermaid(toGraph(plan))
  assert.match(mermaid, /==>\|"review-deadline after 4 hours"\|/)
  assert.match(mermaid, /×3/)
})

test("static: follows helpers across modules and substitutes literal arguments", () => {
  const { plan } = extractFile(file("./cases/helpers.ts"))
  assert.deepEqual(ids(plan), ["design", "design-approved", "build-api", "test-api", "build-web", "test-web", "release"])
  const release = find(plan, (n) => n.type === "op" && n.id === "release")[0] as any
  assert.deepEqual(release.uses.sort(), ["build-api", "build-web", "test-api", "test-web"])
})

test("static: data-driven shapes become templates, fan-outs of unknown width, and unbounded loops", () => {
  const { plan } = extractFile(file("./cases/dynamic.ts"))
  const ops = staticOps(plan)
  assert.deepEqual(ops.map((o) => [o.id, o.idForm]), [
    ["plan", "literal"],
    ["deploy-${service}", "template"],
    ["<[\"ship\", lane, input.team].join(\"-\")>", "dynamic"],
    ["watch-${round}/healthy", "template"],
    ["timer", "literal"]
  ])
  assert.deepEqual((find(plan, (n) => n.type === "parallel")[0] as any).over, "input.services")
  assert.equal((find(plan, (n) => n.type === "loop")[0] as any).bound, null)
})

test("static: a call it cannot follow is marked opaque, not dropped", () => {
  const { plan, diagnostics } = extractFile(file("./cases/opaque.ts"))
  assert.deepEqual(ids(plan), ["migrate", "audit-migrate", "canary-${region}", "<ticket(input.change)>"])
  assert.deepEqual(find(plan, (n) => n.type === "opaque").map((n: any) => n.text), ["suites[input.mode]()"])
  assert.equal(diagnostics.length, 1)
})

test("probe: ship-change reaches every op, both review paths and every ending", () => {
  const result = probe(shipChange, shipInput)
  assert.equal(result.complete, true)
  const reached = result.ops.map((o) => o.op)
  for (const op of ["build", "build.2", "build.3", "docs", "review", "review-overdue", "review-reassigned", "round-3/fix", "report-failure"])
    assert.ok(reached.includes(op), op)
  assert.deepEqual(result.together[0], ["build", "docs"])
  assert.ok(result.endings.includes('failed {"_tag":"CiNeverPassed","rounds":3}'))
  assert.ok(result.endings.includes('failed {"_tag":"Stopped","reason":"a person stopped the change"}'))
  // The review's deadline is a named timer, so it has one ID on every path. (Counted timers gave
  // it timer, timer.2 or timer.3 depending on how many retry delays ran first.)
  const deadlines = result.together.filter((g) => g[0] === "review").map((g) => g[1])
  assert.deepEqual([...new Set(deadlines)], ["review-deadline"])
})

test("probe: sees through helpers, fan-out over input and computed IDs, for the input it was given", () => {
  assert.deepEqual(
    probe(helpers, { change: "x" }).ops.map((o) => o.op).sort(),
    ["build-api", "build-web", "design", "design-approved", "release", "test-api", "test-api.2", "test-web", "test-web.2"]
  )
  const fanout = probe(dynamic, { services: ["auth", "billing", "search"], team: "lichen" })
  const reached = fanout.ops.map((o) => o.op)
  assert.ok(reached.includes("deploy-search") && reached.includes("ship-slow-lichen"))
  // A branch on a result's content follows only the value the probe invented.
  assert.ok(!reached.some((op) => op.startsWith("ship-fast")))
  const told = probe(dynamic, { services: ["auth"], team: "lichen" }, {
    success: (op, kind): Array<{ label: string; value: Json }> =>
      op === "plan"
        ? [{ label: "ok:normal", value: { summary: "routine", evidence: [], attempt: 1 } }, { label: "ok:hotfix", value: { summary: "hotfix", evidence: [], attempt: 1 } }]
        : [{ label: "ok", value: kind === "gate" ? { answer: "pass", evidence: [] } : { summary: "done", evidence: [], attempt: 1 } }]
  })
  assert.ok(told.ops.some((o) => o.op === "ship-fast-lichen"))
  const opaqueOps = probe(opaque, { regions: ["north", "south"], mode: "full", change: "orchard/feature-b" }).ops.map((o) => o.op)
  assert.ok(opaqueOps.includes("full-suite") && !opaqueOps.includes("quick-check"))
})

test("probe: an unbounded loop exhausts the bound and says the search is incomplete", () => {
  const forever = mission("orchard/forever", (_: {}) =>
    Effect.gen(function* () {
      for (let round = 1; ; round++) {
        const r = yield* gate("green", { exec: "./check.sh" }).pipe(Effect.result, scope(`try-${round}`))
        if (r._tag === "Success") return round
      }
    })
  )
  const result = probe(forever, {}, { maxDepth: 6 })
  assert.equal(result.complete, false)
  // Rounds 1 to 7 opened: depth 0 opens the first, each resolution opens the next.
  assert.equal(result.ops.length, 7)
})

test("compare: each extractor's blind spots", () => {
  const ship = compare(extractFile(file("../exp1-sdk/ship-change.ts")).plan, probe(shipChange, shipInput))
  assert.deepEqual(ship.staticOnly, [])
  assert.deepEqual(ship.probeOnly, [])
  const opaqueCmp = compare(
    extractFile(file("./cases/opaque.ts")).plan,
    probe(opaque, { regions: ["north"], mode: "quick", change: "orchard/feature-b" })
  )
  // Static could not name the table entry or the computed ticket ID; the probe saw one of each.
  assert.deepEqual(opaqueCmp.probeOnly, ["quick-check", "ticket-feature-b"])
  assert.deepEqual(opaqueCmp.dynamic, ["<ticket(input.change)>"])
  const dyn = compare(extractFile(file("./cases/dynamic.ts")).plan, probe(dynamic, { services: ["auth"], team: "lichen" }))
  assert.deepEqual(dyn.probeOnly, ["ship-slow-lichen"])
})
