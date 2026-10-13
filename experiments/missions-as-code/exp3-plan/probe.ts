// Probe plan extraction: run the real program through decide() against made-up histories and
// record what it asks for. Each open op can succeed or fail, each ask can take each of its named
// answers, and each timer can fire, in any order. A breadth-first search over those outcomes,
// bounded by states and depth, collects every op it reaches, which ops open together, what
// outcome led to each op, and how the run can end.
//
// It sees through helpers, computed IDs and data-driven fan-out, because it runs the code. It
// only sees what the sample input and the bound reach, and an op's success value is invented,
// so a branch on the content of a result (rather than on success or failure) follows only the
// value the probe made up.

import { decide, type Program } from "../sdk/engine.ts"
import type { Decision, HistoryEvent, Json, OpKind } from "../sdk/history.ts"

export interface ProbeOptions {
  readonly maxStates?: number
  readonly maxDepth?: number
  // Values a successful op returns, by kind. A function can vary them by op.
  readonly success?: (op: string, kind: OpKind, spec: any) => Array<{ label: string; value: Json }>
  readonly failures?: boolean
}

export interface ProbeOp {
  readonly op: string
  readonly kind: OpKind
  readonly title?: string
  readonly assignedTo?: string
  readonly firstDepth: number // resolutions before it first opened
}

export interface ProbeEdge {
  readonly from: string // an op ID or "start"
  readonly outcome: string // ok, fail, fire, answer:<id>, or "" from start
  readonly to: string // an op ID, "cancel <op>", or "end <status>"
}

export interface ProbeResult {
  readonly ops: Array<ProbeOp>
  readonly edges: Array<ProbeEdge>
  // Ops that opened in the same decision (parallel groups), deduplicated.
  readonly together: Array<Array<string>>
  readonly endings: Array<string>
  readonly states: number
  readonly complete: boolean // false when the bound stopped the search
  readonly divergences: Array<string>
}

const defaultSuccess = (op: string, kind: OpKind, spec: any): Array<{ label: string; value: Json }> => {
  switch (kind) {
    case "step":
      return [{ label: "ok", value: { summary: `${op} done`, evidence: [], attempt: 1 } }]
    case "gate":
      return [{ label: "ok", value: { answer: "pass", evidence: [] } }]
    case "ask": {
      const answers: Array<{ id: string }> = spec?.answers ?? []
      return answers.length
        ? answers.map((a) => ({ label: `answer:${a.id}`, value: { id: a.id } }))
        : [{ label: "ok", value: { id: "ok", text: "probe" } }]
    }
    default:
      return [{ label: "ok", value: null }]
  }
}

const statusText = (d: Decision): string =>
  d.status.state === "running" ? "running" : `${d.status.state} ${JSON.stringify((d.status as any).value ?? (d.status as any).error)}`

export const probe = (program: Program, input: Json, options: ProbeOptions = {}): ProbeResult => {
  const maxStates = options.maxStates ?? 5000
  const maxDepth = options.maxDepth ?? 40
  const success = options.success ?? defaultSuccess
  const failures = options.failures ?? true
  const start = 1_700_000_000_000
  const ops = new Map<string, ProbeOp>()
  const edges = new Map<string, ProbeEdge>()
  const together = new Map<string, Array<string>>()
  const endings = new Set<string>()
  const divergences = new Set<string>()
  const seen = new Set<string>()
  let complete = true
  let states = 0

  const addEdge = (e: ProbeEdge) => edges.set(`${e.from}|${e.outcome}|${e.to}`, e)

  // Settle a history: decide, enact schedule and cancel commands, record what opened.
  const settle = (history: Array<HistoryEvent>, now: number, cause: { from: string; outcome: string }, depth: number) => {
    const decision = decide(program, history)
    if (decision.divergence) {
      divergences.add(`${decision.divergence.op}: ${decision.divergence.reason}`)
      return { decision, history }
    }
    const next = [...history]
    const opened: Array<string> = []
    for (const c of decision.commands) {
      if (c.type === "schedule") {
        next.push({ type: "scheduled", at: now, op: c.op, kind: c.kind, hash: c.hash })
        opened.push(c.op)
        const spec = c.spec as any
        if (!ops.has(c.op))
          ops.set(c.op, {
            op: c.op,
            kind: c.kind,
            ...(spec?.title ? { title: spec.title } : c.kind === "timer" ? { title: `${spec.ms / 60000}m` } : {}),
            ...(spec?.assignedTo ? { assignedTo: spec.assignedTo } : spec?.for ? { assignedTo: spec.for } : {}),
            firstDepth: depth
          })
        addEdge({ from: cause.from, outcome: cause.outcome, to: c.op })
      } else {
        next.push({ type: "cancelled", at: now, op: c.op })
        addEdge({ from: cause.from, outcome: cause.outcome, to: `cancel ${c.op}` })
      }
    }
    if (opened.length > 1) together.set(opened.join(","), opened)
    if (decision.status.state !== "running") {
      endings.add(statusText(decision))
      addEdge({ from: cause.from, outcome: cause.outcome, to: `end ${decision.status.state}` })
    }
    return { decision, history: next }
  }

  type State = { history: Array<HistoryEvent>; decision: Decision; now: number; depth: number; key: Array<string> }
  const first = settle(
    [{ type: "started", at: start, program: program.name, version: program.version, input }],
    start,
    { from: "start", outcome: "" },
    0
  )
  const queue: Array<State> = [{ ...first, now: start, depth: 0, key: [] }]
  while (queue.length > 0) {
    const state = queue.shift()!
    if (state.decision.status.state !== "running" || state.decision.divergence) continue
    if (state.depth >= maxDepth) {
      complete = false
      continue
    }
    for (const open of state.decision.open) {
      const outcomes: Array<{ label: string; event: (at: number) => HistoryEvent; at: number }> = []
      if (open.kind === "timer") {
        const fireAt = Math.max(state.now, (open.spec as any).fireAt as number)
        outcomes.push({ label: "fire", at: fireAt, event: (at) => ({ type: "completed", at, op: open.op, value: null }) })
      } else {
        const at = state.now + 60_000
        for (const s of success(open.op, open.kind, open.spec))
          outcomes.push({ label: s.label, at, event: (t) => ({ type: "completed", at: t, op: open.op, value: s.value }) })
        if (failures)
          outcomes.push({ label: "fail", at, event: (t) => ({ type: "failed", at: t, op: open.op, error: { reason: "probe" } }) })
      }
      for (const outcome of outcomes) {
        // States that resolved the same ops the same way are the same state, whatever the order.
        const key = [...state.key, `${open.op}=${outcome.label}`].sort()
        const keyText = key.join("|")
        if (seen.has(keyText)) continue
        seen.add(keyText)
        if (++states > maxStates) {
          complete = false
          queue.length = 0
          break
        }
        const history = [...state.history, outcome.event(outcome.at)]
        const settled = settle(history, outcome.at, { from: open.op, outcome: outcome.label }, state.depth + 1)
        queue.push({ history: settled.history, decision: settled.decision, now: outcome.at, depth: state.depth + 1, key })
      }
    }
  }
  return {
    ops: [...ops.values()],
    edges: [...edges.values()],
    together: [...together.values()],
    endings: [...endings].sort(),
    states,
    complete,
    divergences: [...divergences]
  }
}

export const probeMermaid = (result: ProbeResult): string => {
  const ids = new Map<string, string>()
  const node = (name: string) => {
    if (!ids.has(name)) ids.set(name, `n${ids.size}`)
    return ids.get(name)!
  }
  const lines = ["flowchart TD"]
  const esc = (s: string) => s.replace(/"/g, "'")
  lines.push(`  ${node("start")}(("start"))`)
  for (const op of result.ops)
    lines.push(`  ${node(op.op)}["${esc(`${op.kind} ${op.op}${op.title && op.kind === "timer" ? ` ${op.title}` : ""}`)}"]`)
  for (const e of result.edges) {
    if (e.to.startsWith("end ") || e.to.startsWith("cancel ")) lines.push(`  ${node(e.to)}(["${esc(e.to)}"])`)
    lines.push(`  ${node(e.from)} -->${e.outcome ? `|"${esc(e.outcome)}"|` : ""} ${node(e.to)}`)
  }
  return [...new Set(lines)].join("\n")
}
