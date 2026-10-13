// Baseline: the same bundle in Node's own V8 (node:vm, bare context), warm, same fixtures.
import vm from "node:vm"
import { readFileSync, readdirSync } from "node:fs"
const dir = new URL("../fixtures/", import.meta.url)
const bundle = readFileSync(new URL("bundle.js", dir), "utf8")
const t0 = performance.now()
const context = vm.createContext({})
vm.runInContext(bundle, context)
const startMs = performance.now() - t0
const rows = []
for (const file of readdirSync(dir).filter((f) => f.endsWith(".json")).sort()) {
  const fx = JSON.parse(readFileSync(new URL(file, dir), "utf8"))
  const prefix = JSON.stringify(fx.history.slice(0, -1))
  const n = fx.history.length > 1000 ? 20 : fx.history.length > 100 ? 100 : 300
  for (let i = 0; i < 5; i++) context.__decide(fx.program, prefix)
  const samples = []
  for (let i = 0; i < n; i++) {
    const t = performance.now()
    context.__decide(fx.program, prefix)
    samples.push(performance.now() - t)
  }
  samples.sort((a, b) => a - b)
  rows.push({ fixture: file.replace(".json", ""), events: fx.history.length, p50_ms: +samples[Math.floor(n / 2)].toFixed(3), p99_ms: +samples[Math.round((n - 1) * 0.99)].toFixed(3) })
}
console.log(JSON.stringify({ engine: `node ${process.version} (vm context)`, start_ms: +startMs.toFixed(1), cases: rows }, null, 2))
