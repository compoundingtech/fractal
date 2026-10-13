// Plays a demo program out in virtual time and writes the beats block it means, in the plain
// demo format. Each path through the program is one file: pass the person's answers in order.
//
//   node exp6-port/demo/compile.ts exp6-port/demo/release-day.ts
//   node exp6-port/demo/compile.ts exp6-port/demo/an-alert-needs-an-answer.ts now

import { resolve } from "node:path"
import { pathToFileURL } from "node:url"
import type { Program } from "../../sdk/engine.ts"
import { simulate } from "../../sdk/sim.ts"
import type { Beat } from "./demo.ts"

const KEY_MS = 2000 // when the presenter presses the advance key, in virtual time
const ANSWER_MS = 5000 // when the person clicks an answer

const q = (s: string) => `"${s.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`
const block = (s: string, indent: string) => `"""\n${s.split("\n").map((l) => (l ? indent + l : "")).join("\n")}\n${indent}"""`
const str = (s: string, indent: string) => (s.includes("\n") ? block(s, indent) : q(s))
const seconds = (ms: number) => `${Number((ms / 1000).toFixed(3))}s`

export const compile = (program: Program, answers: ReadonlyArray<string> = []): string => {
  let asked = 0
  const trace = simulate(program, null, (_op, _kind, spec: Beat) => {
    if (spec.action === "key") return { after: KEY_MS, value: null }
    if (spec.action === "ask") return { after: ANSWER_MS, value: { id: answers[asked++] ?? spec.answers![0]!.id } }
    return { after: 0, value: null }
  })
  // Pair each scheduled beat (its time, from the history) with its spec (from the decisions).
  const specs = trace.decisions.flatMap((d) => d.commands.flatMap((c) => (c.type === "schedule" && c.kind === "beat" ? [c.spec as unknown as Beat] : [])))
  const times = trace.history.flatMap((e) => (e.type === "scheduled" && e.kind === "beat" ? [e.at] : []))
  const done = new Map(trace.history.flatMap((e) => (e.type === "completed" ? [[e.op, e.at] as const] : [])))
  const ops = trace.history.flatMap((e) => (e.type === "scheduled" && e.kind === "beat" ? [e.op] : []))
  const lines: Array<string> = []
  let previousDone: number | undefined
  let waitedForKey = true // the first beat waits for the presenter
  let afterAsk = false
  specs.forEach((b, i) => {
    const at = times[i]!
    if (b.action === "key") {
      waitedForKey = true
      previousDone = done.get(ops[i]!)
      return
    }
    const parts = [b.action]
    if (b.target !== undefined) parts.push(str(b.target, "    "))
    if (afterAsk && !waitedForKey) {
      // The program waits for the person's answer; a beats file cannot, so the presenter's key
      // stands in for it.
      lines.push("  // waits for the advance key: press it once the alert is answered")
      waitedForKey = true
    }
    afterAsk = false
    if (!waitedForKey && previousDone !== undefined) parts.push(`after="${seconds(at - previousDone)}"`)
    for (const p of b.positional ?? []) parts.push(q(p))
    for (const [k, v] of Object.entries(b.attrs ?? {})) {
      if (v === undefined) continue
      parts.push(v === true ? `${k}=#true` : `${k}=${str(String(v), "    ")}`)
    }
    if (b.text !== undefined) parts.push(str(b.text, "    "))
    let line = `  ${parts.join(" ")}`
    if (b.answers) {
      line += " {\n" + b.answers.map((a) =>
        `    answer ${q(a.id)} ${q(a.label)}${a.outcome ? ` outcome=${q(a.outcome)}` : ""}${a.then ? ` then=${q(a.then)}` : ""}`).join("\n") + "\n  }"
    }
    lines.push(line)
    afterAsk = b.action === "ask"
    waitedForKey = false
    previousDone = done.get(ops[i]!) ?? at
  })
  return `beats {\n${lines.join("\n")}\n}\n`
}

if (import.meta.main) {
  const [file, ...answers] = process.argv.slice(2)
  const program = (await import(pathToFileURL(resolve(file!)).href)).default as Program
  process.stdout.write(compile(program, answers))
}
