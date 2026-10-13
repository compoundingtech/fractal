// Print a program's static plan as JSON and Mermaid: node exp3-plan/print.ts FILE [--json]
import { extractFile, toGraph, toMermaid } from "./static.ts"

const file = process.argv[2]
if (!file) throw new Error("usage: node exp3-plan/print.ts FILE [--json]")
const extraction = extractFile(file)
if (process.argv.includes("--json")) console.log(JSON.stringify({ ...extraction, graph: toGraph(extraction.plan) }, null, 2))
else {
  console.log(toMermaid(toGraph(extraction.plan)))
  for (const d of extraction.diagnostics) console.error(`note: ${d}`)
}
