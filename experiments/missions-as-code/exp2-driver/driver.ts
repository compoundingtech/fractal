// Experiment 2: durable execution outside the daemon.
//
// The driver runs as a declared st exec inside a host mission run. It owns no state of its own:
// - The journal is a list of custom claims on custom/mac/journal/<run>, one per history event,
//   written with idempotency key <run>/<seq>. A second driver that disagrees about event <seq>
//   is refused, so the journal is also the fence.
// - Each op becomes a child mission run with a deterministic ID, <run>/op/<op>, so a restart
//   that enacts the same command again gets the same run back instead of a second one.
// - On start it reads the journal, replays it through decide(), and carries on.
//
// Usage (st runs this; see host.ts):
//   node driver.ts --program FILE [--run ID] [--actor ACTOR] [--workspace DIR]
//                  [--lab-worker CMD] [--time-scale N] [--poll-ms N] [--crash-after-enact OP]

import { existsSync, mkdirSync, writeFileSync } from "node:fs"
import { join, resolve } from "node:path"
import { pathToFileURL } from "node:url"
import { decide, type Program } from "../sdk/engine.ts"
import type { HistoryEvent, Json, OpKind } from "../sdk/history.ts"
import { render, type LabOptions } from "./kdl.ts"
import { St, StError } from "./st.ts"

const argv = process.argv.slice(2)
const arg = (name: string) => {
  const i = argv.indexOf(`--${name}`)
  return i >= 0 ? argv[i + 1] : undefined
}

const run = arg("run") ?? process.env.ST_MISSION_RUN
const programFile = arg("program")
if (!run || !programFile) throw new Error("--program and --run (or ST_MISSION_RUN) are required")
const actor = arg("actor") ?? process.env.ST_REQUESTER ?? "person/operator"
const workspace = arg("workspace") ?? process.env.ST_WORKSPACE ?? process.cwd()
const scale = Number(arg("time-scale") ?? 1)
const pollMs = Number(arg("poll-ms") ?? 1000)
const crashAfter = arg("crash-after-enact")
const lab: LabOptions | undefined = arg("lab-worker") ? { worker: arg("lab-worker")! } : undefined

const st = new St({ bin: process.env.ST3_BIN ?? "st", actor })
const journal = `custom/mac/journal/${run}`
const resultSubject = `custom/mac/result/${run}`
const childRun = (op: string) => `${run}/op/${op}`
const scratch = join(workspace, ".mac", run.replace(/\//g, "_"))
mkdirSync(scratch, { recursive: true })

const log = (line: string) => console.log(`${new Date().toISOString()} ${line}`)
const sleep = (ms: number) => Atomics.wait(new Int32Array(new SharedArrayBuffer(4)), 0, 0, ms)

// ---- time -------------------------------------------------------------------------------------
// The lab compresses program time (4 hours of review deadline in 24 seconds at --time-scale 600).
// History times are program times, so a replay sees the same clock whatever the scale.
let clockBase = { real: Date.now(), program: Date.now() }
const programTime = (real: number) => Math.round(clockBase.program + (real - clockBase.real) * scale)
const now = () => programTime(Date.now())

// ---- journal ----------------------------------------------------------------------------------

const readJournal = (): Array<HistoryEvent> => {
  const events = new Map<number, HistoryEvent>()
  let after: number | undefined
  for (;;) {
    const page = st.jsonLines(["subject", "history", journal, "--limit", "500", ...(after ? ["--after-index", String(after)] : [])])
    for (const claim of page) {
      if (claim.kind !== "custom.mac.journal.event") continue
      const seq = claim.body.fields.seq as number
      if (!events.has(seq)) events.set(seq, claim.body.fields.event as HistoryEvent)
    }
    if (page.length < 500) break
    after = page.at(-1).store_index
  }
  const history: Array<HistoryEvent> = []
  for (let seq = 0; events.has(seq); seq++) history.push(events.get(seq)!)
  if (history.length !== events.size) throw new Error(`journal ${journal} has a gap after event ${history.length}`)
  return history
}

const append = (history: Array<HistoryEvent>, event: HistoryEvent) => {
  const seq = history.length
  st.run([
    "claim", journal, "custom.mac.journal.event",
    "--actor", actor,
    "--field", `seq=${seq}`,
    "--field", `event=${JSON.stringify(event)}`,
    "--idempotency-key", `${run}/${seq}`
  ])
  history.push(event)
}

// ---- enacting commands ------------------------------------------------------------------------

const published = new Map<string, string>() // KDL hash -> published revision

const enact = (program: Program, op: string, kind: OpKind, spec: any) => {
  if (kind === "timer") return
  const rendered = render(program.name, op, kind, spec, lab)
  let revision = published.get(rendered.hash)
  if (!revision) {
    const file = join(scratch, `${rendered.hash}.kdl`)
    writeFileSync(file, rendered.kdl)
    const applied = st.json(["apply", file, "--no-gate-check", ...st.as()])
    revision = applied.published_missions.find((m: any) => m.subject === `mission/${rendered.mission}`).revision as string
    published.set(rendered.hash, revision)
  }
  // Idempotent: the same ID returns the run a crashed driver already started.
  st.run(["missions", "start", rendered.mission, "--revision", revision, "--id", childRun(op), "--workspace", workspace, ...st.as()])
}

const cancel = (op: string, kind: OpKind) => {
  if (kind === "timer") return
  try {
    st.run(["missions", "cancel", `mission-run/${childRun(op)}`, "--reason", `the program stopped waiting on ${op}`, ...st.as()])
  } catch (error) {
    // Already finished: the cancel has nothing left to stop.
    if (!(error instanceof StError)) throw error
  }
}

// ---- observing results ------------------------------------------------------------------------

interface Observation {
  readonly at: number
  readonly event: HistoryEvent
}

const workerReport = (stepSubject: string) => {
  const claims = st.jsonLines(["subject", "history", stepSubject, "--limit", "200"])
  return claims.filter((c) => ["work.completed", "work.submitted", "work.person-done", "work.failed"].includes(c.kind)).at(-1)
}

const observe = (op: string, kind: OpKind, spec: any): Observation | undefined => {
  if (kind === "timer") {
    const fireAt = spec.fireAt as number
    return now() >= fireAt ? { at: fireAt, event: { type: "completed", at: fireAt, op, value: null } } : undefined
  }
  let show: any
  try {
    show = st.json(["missions", "show", `mission-run/${childRun(op)}`])
  } catch {
    return undefined // not visible yet
  }
  if (!["completed", "failed", "cancelled"].includes(show.status)) return undefined
  const step = show.steps?.[0]
  const at = programTime(show.updated_at_unix_ms as number)
  if (show.status !== "completed") {
    const reason = step?.failure_reason ?? step?.blocked_reason ?? show.reason ?? `the ${kind} run ${show.status}`
    return { at, event: { type: "failed", at, op, error: { reason: String(reason), attempt: step?.attempt } } }
  }
  const report = step ? workerReport(step.subject) : undefined
  const summary: string = report?.body?.fields?.summary ?? report?.body?.summary ?? `${op} completed`
  const evidence: Array<string> = report?.body?.evidence ?? [step?.subject ?? `mission-run/${childRun(op)}`]
  let value: Json
  if (kind === "step") value = { summary, evidence, attempt: step?.attempt ?? 1 }
  else if (kind === "ask") {
    const [first, ...rest] = summary.trim().split(/\s+/)
    const ids = (spec.answers ?? []).map((a: any) => a.id)
    value = ids.includes(first) ? { id: first!, text: rest.join(" ") } : { id: "custom", text: summary }
  } else if (kind === "gate") value = { answer: "pass", evidence }
  else value = { evidence }
  return { at, event: { type: "completed", at, op, value } }
}

// ---- the loop ---------------------------------------------------------------------------------

const main = async () => {
  const program: Program = (await import(pathToFileURL(resolve(programFile)).href)).default
  const history = readJournal()
  if (history.length === 0) {
    const show = st.json(["missions", "show", `mission-run/${run}`])
    const input = JSON.parse(show.inputs?.["program-input"]?.value ?? "{}")
    append(history, { type: "started", at: Date.now(), program: program.name, version: program.version, input })
    log(`started ${program.name} as ${run}`)
  } else {
    log(`resumed ${program.name} as ${run}: replaying ${history.length} journal events`)
  }
  const last = history.at(-1)!.at
  clockBase = { real: Date.now(), program: Math.max(last, scale === 1 ? Date.now() : last) }

  for (;;) {
    const decision = decide(program, history)
    if (decision.divergence) {
      log(`divergence at event ${decision.divergence.at}: ${decision.divergence.op}: ${decision.divergence.reason}`)
      st.run(["claim", resultSubject, "custom.mac.result", "--actor", actor, "--field", "state=diverged", "--field", `divergence=${JSON.stringify(decision.divergence)}`])
      // Hold, as a broken gate does: the run waits for a person to revise the program.
      for (;;) sleep(60_000)
    }
    for (const command of decision.commands) {
      if (command.type === "schedule") {
        enact(program, command.op, command.kind, command.spec)
        if (crashAfter === command.op && !existsSync(join(scratch, "crashed"))) {
          writeFileSync(join(scratch, "crashed"), "")
          log(`fault injection: exiting after enacting ${command.op}, before the journal records it`)
          process.exit(137)
        }
        append(history, { type: "scheduled", at: now(), op: command.op, kind: command.kind, hash: command.hash })
        log(`schedule ${command.kind} ${command.op}`)
      } else {
        const open = history.find((e) => e.type === "scheduled" && e.op === command.op) as any
        cancel(command.op, open?.kind ?? "step")
        append(history, { type: "cancelled", at: now(), op: command.op })
        log(`cancel ${command.op}`)
      }
    }
    if (decision.status.state !== "running") {
      const status = decision.status
      st.run([
        "claim", resultSubject, "custom.mac.result", "--actor", actor,
        "--field", `state=${status.state}`,
        "--field", `value=${JSON.stringify("value" in status ? status.value : status.error)}`,
        "--field", `events=${history.length}`,
        "--idempotency-key", `${run}/result`
      ])
      log(`${status.state}: ${JSON.stringify("value" in status ? status.value : status.error)} (${st.calls} st calls, ${st.callMs} ms in st)`)
      process.exit(status.state === "completed" ? 0 : 2)
    }
    if (decision.commands.length > 0) continue
    // Wait for the first result among the open ops, and record exactly one, in the order seen.
    for (;;) {
      const seen = decision.open
        .map((o) => observe(o.op, o.kind, o.spec))
        .filter((o): o is Observation => o !== undefined)
        .sort((a, b) => a.at - b.at)
      if (seen.length > 0) {
        const event = { ...seen[0]!.event, at: Math.max(seen[0]!.at, history.at(-1)!.at) } as HistoryEvent
        append(history, event)
        log(`${event.type} ${(event as any).op}`)
        break
      }
      sleep(pollMs)
    }
  }
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
