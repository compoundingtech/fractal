// Runs the bundle in a bare V8 context (node:vm) with no host globals, and checks it matches
// the decisions Node made in-process.
import vm from "node:vm"
import { readFileSync, readdirSync } from "node:fs"
const dir = new URL("../fixtures/", import.meta.url)
const bundle = readFileSync(new URL("bundle.js", dir), "utf8")
const context = vm.createContext({})
vm.runInContext(bundle, context)
for (const file of readdirSync(dir).filter((f) => f.endsWith(".json"))) {
  const fx = JSON.parse(readFileSync(new URL(file, dir), "utf8"))
  const got = context.__decide(fx.program, JSON.stringify(fx.history))
  const pre = context.__decide(fx.program, JSON.stringify(fx.history.slice(0, -1)))
  console.log(file, got === JSON.stringify(fx.decision) && pre === JSON.stringify(fx.prefixDecision) ? "match" : "MISMATCH")
}
