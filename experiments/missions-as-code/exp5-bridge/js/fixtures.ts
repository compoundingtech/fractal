// Writes history fixtures for the bridge benchmarks: realistic ship-change runs and synthetic
// long and wide runs. Each fixture records the decision Node made, so every engine can be
// compared with it byte for byte.
import { mkdirSync, writeFileSync } from "node:fs"
import { decide } from "../../sdk/engine.ts"
import { simulate } from "../../sdk/sim.ts"
import { programs } from "./programs.ts"

const out = new URL("../fixtures/", import.meta.url)
mkdirSync(out, { recursive: true })
const minute = 60_000
const done = (op: string) => ({ after: 7 * minute, value: { summary: op, evidence: [`doc/${op}`], attempt: 1 } })

const cases: Array<[string, string, any, (op: string, kind: string) => any]> = [
  ["ship-happy", "orchard/ship-change", { change: "orchard/a", repo: "orchard/app" }, (op, kind) => kind === "gate" ? { after: minute, value: { answer: "pass", evidence: [] } } : done(op)],
  ["ship-messy", "orchard/ship-change", { change: "orchard/b", repo: "orchard/app" }, (() => { let ci = 0; return (op: string, kind: string) =>
    op === "build" ? { after: minute, error: { reason: "flaky" } } :
    op === "review" ? "never" :
    op === "review-overdue" ? { after: minute, value: { id: "reassign" } } :
    kind === "gate" ? (++ci < 3 ? { after: minute, error: { reason: "red" } } : { after: minute, value: { answer: "pass", evidence: [] } }) : done(op) })()],
  ...[10, 100, 1000].map((n) => [`long-${n}`, "orchard/long-run", { steps: n }, (op: string) => done(op)] as any),
  ...[10, 100, 1000].map((n) => [`wide-${n}`, "orchard/wide-run", { steps: n }, (op: string) => done(op)] as any)
]

for (const [name, program, input, script] of cases) {
  const trace = simulate((programs as any)[program], input, script as any, { maxRounds: 5000 })
  // Every prefix that ends where the driver would call decide: after each result.
  const full = trace.history
  const final = decide((programs as any)[program], full)
  const prefix = full.slice(0, -1)
  const prefixDecision = decide((programs as any)[program], prefix)
  writeFileSync(new URL(`${name}.json`, out), JSON.stringify({ program, history: full, decision: final, prefixDecision }))
  console.log(`${name}: ${full.length} events, status ${final.status.state}`)
}
