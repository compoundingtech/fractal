// Probe a program and print what it reached: node exp3-plan/print-probe.ts FILE 'INPUT_JSON' [--mermaid]
import { probe, probeMermaid } from "./probe.ts"

const [file, inputText] = process.argv.slice(2)
if (!file) throw new Error("usage: node exp3-plan/print-probe.ts FILE 'INPUT_JSON' [--mermaid]")
const program = (await import(new URL(file, `file://${process.cwd()}/`).href)).default
const started = performance.now()
const result = probe(program, JSON.parse(inputText ?? "{}"))
const ms = Math.round(performance.now() - started)
if (process.argv.includes("--mermaid")) console.log(probeMermaid(result))
else {
  console.log(`states ${result.states}, complete ${result.complete}, ${ms} ms`)
  console.log("ops:")
  for (const op of result.ops) console.log(`  depth ${String(op.firstDepth).padStart(2)}  ${op.kind.padEnd(5)} ${op.op}${op.assignedTo ? `  (${op.assignedTo})` : ""}`)
  console.log("opened together:", result.together.map((g) => g.join(" + ")).join("; ") || "none")
  console.log("endings:")
  for (const e of result.endings) console.log(`  ${e}`)
  if (result.divergences.length) console.log("divergences:", result.divergences)
}
