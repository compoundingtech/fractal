// An in-memory world for driving a program without st: it enacts commands, answers ops from a
// script, and advances virtual time. The real driver (driver/) does the same against st.

import { decide, type Program } from "./engine.ts"
import type { Decision, HistoryEvent, Json, OpError, OpKind } from "./history.ts"

export type Outcome =
  | { readonly after: number; readonly value: Json }
  | { readonly after: number; readonly error: OpError }
  | "never"

// Answers an op by its ID and spec. Timers answer themselves at their fire time.
export type Script = (op: string, kind: OpKind, spec: any) => Outcome

export interface Trace {
  readonly history: Array<HistoryEvent>
  readonly decisions: Array<Decision>
  readonly log: Array<string>
}

export const simulate = (
  program: Program,
  input: Json,
  script: Script,
  options: {
    start?: number
    maxRounds?: number
    // Resume an existing run (a crash, or a new version) instead of starting one.
    history?: ReadonlyArray<HistoryEvent>
    decide?: (program: Program, history: ReadonlyArray<HistoryEvent>) => Decision
  } = {}
): Trace => {
  const start = options.history?.[0]?.at ?? options.start ?? 1_700_000_000_000
  const decideWith = options.decide ?? decide
  const history: Array<HistoryEvent> = options.history
    ? [...options.history]
    : [{ type: "started", at: start, program: program.name, version: program.version, input }]
  const decisions: Array<Decision> = []
  const log: Array<string> = []
  const pending = new Map<string, { at: number; event: HistoryEvent }>()
  let now = history.at(-1)!.at
  const finished = new Set(history.flatMap((e) => (e.type === "completed" || e.type === "failed" || e.type === "cancelled" ? [e.op] : [])))
  for (let round = 0; round < (options.maxRounds ?? 500); round++) {
    const decision = decideWith(program, history)
    // On a resumed run, ops already scheduled get their outcome from the script now.
    for (const open of decision.open) {
      if (!open.scheduled || pending.has(open.op) || finished.has(open.op)) continue
      const outcome: Outcome =
        open.kind === "timer" ? { after: Math.max(0, (open.spec as any).fireAt - now), value: null } : script(open.op, open.kind, open.spec)
      if (outcome === "never") continue
      const at = now + outcome.after
      pending.set(open.op, {
        at,
        event: "error" in outcome ? { type: "failed", at, op: open.op, error: outcome.error } : { type: "completed", at, op: open.op, value: outcome.value }
      })
    }
    decisions.push(decision)
    if (decision.divergence) {
      log.push(`diverged at ${decision.divergence.at}: ${decision.divergence.op}: ${decision.divergence.reason}`)
      break
    }
    for (const command of decision.commands) {
      if (command.type === "schedule") {
        history.push({ type: "scheduled", at: now, op: command.op, kind: command.kind, hash: command.hash })
        log.push(`${t(now - start)} schedule ${command.kind} ${command.op}`)
        if (command.kind === "marker") {
          finished.add(command.op)
          continue
        }
        if (command.kind === "timer") {
          const fireAt = (command.spec as any).fireAt as number
          pending.set(command.op, { at: fireAt, event: { type: "completed", at: fireAt, op: command.op, value: null } })
          continue
        }
        const outcome = script(command.op, command.kind, command.spec)
        if (outcome === "never") continue
        const at = now + outcome.after
        pending.set(command.op, {
          at,
          event: "error" in outcome
            ? { type: "failed", at, op: command.op, error: outcome.error }
            : { type: "completed", at, op: command.op, value: outcome.value }
        })
      } else {
        pending.delete(command.op)
        finished.add(command.op)
        history.push({ type: "cancelled", at: now, op: command.op })
        log.push(`${t(now - start)} cancel ${command.op}`)
      }
    }
    if (decision.status.state !== "running") {
      log.push(`${t(now - start)} ${decision.status.state} ${JSON.stringify((decision.status as any).value ?? (decision.status as any).error)}`)
      break
    }
    if (decision.commands.length > 0) continue
    // Deliver the earliest pending result; ties go in ID order so the run is reproducible.
    const next = [...pending.entries()].sort((a, b) => a[1].at - b[1].at || (a[0] < b[0] ? -1 : 1))[0]
    if (!next) {
      log.push(`${t(now - start)} stuck: nothing pending`)
      break
    }
    pending.delete(next[0])
    finished.add(next[0])
    now = next[1].at
    history.push(next[1].event)
    log.push(`${t(now - start)} ${next[1].event.type} ${next[0]}`)
  }
  return { history, decisions, log }
}

const t = (ms: number) => {
  const m = Math.floor(ms / 60000)
  return `+${String(Math.floor(m / 60)).padStart(2, "0")}:${String(m % 60).padStart(2, "0")}`
}
