// Experiment 4: what happens to in-flight runs when a program changes under them.
//
// Runs of v1 are stopped at four points. Each v2 is checked against each stopped run; a run that
// can move continues under v2 to the end, and one that cannot keeps replaying under v1.
// `node exp4-hmr/matrix.ts` prints the table in RESULTS.md.

import { join } from "node:path"
import type { HistoryEvent } from "../sdk/history.ts"
import { simulate, type Script } from "../sdk/sim.ts"
import { bundle, type Version } from "./bundle.ts"
import { checkUpgrade, type UpgradeReport } from "./upgrade.ts"

const minute = 60_000
const input = { change: "orchard/feature-a", repo: "orchard/app" }
const ok = (op: string) => ({ after: 20 * minute, value: { summary: op, evidence: [], attempt: 1 } })
// CI fails twice, passes on the third round.
const script: Script = (op, kind) => {
  if (kind === "gate") return /round-[12]\//.test(op) ? { after: 10 * minute, error: { reason: "red" } } : { after: 10 * minute, value: { answer: "pass", evidence: [] } }
  return ok(op)
}

export const checkpoints: Array<{ name: string; open: (ops: Array<string>) => boolean }> = [
  { name: "building", open: (ops) => ops.includes("build") },
  { name: "in review", open: (ops) => ops.includes("review") },
  { name: "fixing round 1", open: (ops) => ops.includes("round-1/fix") },
  { name: "last CI round", open: (ops) => ops.includes("round-3/ci") }
]

export const versionNames = ["v2-text", "v2-insert", "v2-patched", "v2-remove", "v2-rounds"]

export interface Cell {
  readonly checkpoint: string
  readonly version: string
  readonly report: UpgradeReport
  // How the run ended after the decision: under v2 when it moved, under v1 when it stayed.
  readonly ranUnder: string
  readonly outcome: string
  readonly opsAfter: Array<string>
}

export const stopAt = (v1: Version, name: string): Array<HistoryEvent> => {
  const full = simulate(v1.program, input, script, { decide: v1.decide })
  const checkpoint = checkpoints.find((c) => c.name === name)!
  // The shortest prefix after which the program waits on the checkpoint's op, with every command
  // of that decision already journaled: a driver is only ever stopped between decisions' effects
  // or replays to that point.
  for (let i = 1; i <= full.history.length; i++) {
    const prefix = full.history.slice(0, i)
    const decision = v1.decide(v1.program, prefix)
    if (decision.commands.length > 0) continue
    if (checkpoint.open(decision.open.map((o) => o.op))) return prefix
  }
  throw new Error(`v1 never reached ${name}`)
}

export const run = async (): Promise<{ versions: Record<string, Version>; cells: Array<Cell> }> => {
  const dir = join(import.meta.dirname, "versions")
  const versions: Record<string, Version> = {}
  for (const name of ["v1", ...versionNames]) versions[name] = await bundle(join(dir, `${name}.ts`))
  const v1 = versions.v1!
  const cells: Array<Cell> = []
  for (const checkpoint of checkpoints) {
    const prefix = stopAt(v1, checkpoint.name)
    for (const name of versionNames) {
      const to = versions[name]!
      const report = checkUpgrade(to, prefix)
      const moves = report.verdict === "compatible" || report.verdict === "compatible-with-changes"
      const target = moves ? to : v1
      const accepted = report.changes.map((c) => c.op)
      const history: Array<HistoryEvent> = moves
        ? [...prefix, { type: "upgraded", at: prefix.at(-1)!.at, version: to.hash, ...(accepted.length ? { accepted } : {}) }]
        : prefix
      const trace = simulate(target.program, input, script, { history, decide: target.decide })
      const status = trace.decisions.at(-1)!.status
      cells.push({
        checkpoint: checkpoint.name,
        version: name,
        report,
        ranUnder: moves ? name : "v1",
        outcome: status.state === "completed" ? `shipped, ${(status.value as any).rounds} rounds` : status.state,
        opsAfter: trace.history.slice(history.length).flatMap((e) => (e.type === "scheduled" && e.kind !== "timer" ? [e.kind === "marker" ? `(${e.op})` : e.op] : []))
      })
    }
  }
  return { versions, cells }
}

if (import.meta.main) {
  const { versions, cells } = await run()
  console.log("| run stopped while | change | verdict | the run then | ops it opened after the change |")
  console.log("|---|---|---|---|---|")
  for (const c of cells) {
    console.log(`| ${c.checkpoint} | ${c.version} | **${c.report.verdict}**<br>${c.report.reason} | ran under ${c.ranUnder}: ${c.outcome} | ${c.opsAfter.join(", ") || "none"} |`)
  }
  console.log()
  for (const [name, v] of Object.entries(versions)) console.log(`- ${name}: sha256 ${v.hash.slice(0, 16)}…`)
}
