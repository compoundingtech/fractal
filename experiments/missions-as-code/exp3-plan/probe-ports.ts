// Probe the experiment 6 ports (written by another experiment) and report reach.
// node exp3-plan/probe-ports.ts
import { probe } from "./probe.ts"
import { compare } from "./compare.ts"
import { extractFile } from "./static.ts"
import type { Json, OpKind } from "../sdk/history.ts"

// Every step can come back with either of two summaries, so branches on "GO" are explored.
const twoSummaries = (op: string, kind: OpKind, spec: any): Array<{ label: string; value: Json }> => {
  if (kind === "step") return ["GO", "NO-GO"].map((s) => ({ label: `ok:${s}`, value: { summary: `${s} from ${op}`, evidence: ["orchard/app#7"], attempt: 1 } }))
  if (kind === "gate") return [{ label: "ok", value: { answer: "pass", evidence: [] } }]
  if (kind === "ask") return (spec?.answers ?? [{ id: "ok" }]).map((a: any) => ({ label: `answer:${a.id}`, value: { id: a.id } }))
  return [{ label: "ok", value: null }]
}

const ports: Array<{ file: string; input: Json }> = [
  { file: "../exp6-port/change.ts", input: { request: "orchard/feature-a", run: "orchard/change/1" } },
  { file: "../exp6-port/ledger-changes.ts", input: { run: "orchard/ledger/1" } },
  { file: "../exp6-port/refresh-then-answer.ts", input: {} }
]
for (const port of ports) {
  const program = (await import(new URL(port.file, import.meta.url).href)).default
  const plan = extractFile(new URL(port.file, import.meta.url).pathname).plan
  for (const [label, options] of [["default values", {}], ["GO and NO-GO summaries", { success: twoSummaries, maxStates: 20000 }]] as const) {
    const t = performance.now()
    const result = probe(program, port.input, options)
    const c = compare(plan, result)
    console.log(
      `${port.file.replace("../exp6-port/", "")} [${label}]: ${result.ops.length} ops, ${result.states} states, complete=${result.complete}, ${Math.round(performance.now() - t)} ms; ` +
        `static IDs never reached: ${c.staticOnly.length ? c.staticOnly.join(", ") : "none"}; probe-only: ${c.probeOnly.length ? c.probeOnly.join(", ") : "none"}`
    )
  }
}
