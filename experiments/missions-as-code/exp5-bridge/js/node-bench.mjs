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
// Incremental, as the Rust bench does it: open with the first event, then one event per call.
const incremental = []
for (const name of ["ship-messy", "long-100", "long-1000", "wide-1000"]) {
  const fx = JSON.parse(readFileSync(new URL(`${name}.json`, dir), "utf8"))
  const id = context.__open(fx.program, JSON.stringify(fx.history.slice(0, 1)))
  const samples = []
  // As a driver appends them: one decision's scheduled events together, each result alone.
  const batches = []
  for (const event of fx.history.slice(1)) {
    const last = batches.at(-1)
    if (event.type === "scheduled" && last?.[0].type === "scheduled") last.push(event)
    else batches.push([event])
  }
  for (const batch of batches) {
    const one = JSON.stringify(batch)
    const t = performance.now()
    context.__apply(id, one)
    samples.push(performance.now() - t)
  }
  const { stats: _, ...expected } = fx.decision
  const matches = context.__decision(id) === JSON.stringify(expected)
  context.__close(id)
  const total = samples.reduce((a, b) => a + b, 0)
  samples.sort((a, b) => a - b)
  const at = (p) => +(samples[Math.round((samples.length - 1) * p)] * 1e3).toFixed(1)
  incremental.push({ fixture: name, events: fx.history.length, calls: batches.length, matches_full_replay: matches, apply_p50_us: at(0.5), apply_p99_us: at(0.99), apply_max_us: at(1), whole_run_ms: +total.toFixed(1) })
}
console.log(JSON.stringify({ engine: `node ${process.version} (vm context)`, start_ms: +startMs.toFixed(1), cases: rows, incremental }, null, 2))
