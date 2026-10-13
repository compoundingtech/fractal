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
// Versions (experiment 4): with --program pointing at a version bundle (exp4-hmr/bundle.ts), the
// bundle is stored as an immutable document and the journal names it, doc/mac/programs/..@SHA.
// A reload request (custom/mac/reload/<run>) names another version; the driver replays the
// history through it and moves the run, holds the request, or refuses it. A restarted driver
// loads the run's current version from the document store, so every version stays replayable.
//
// Usage (st runs this; see host.ts):
//   node driver.ts --program FILE [--run ID] [--actor ACTOR] [--workspace DIR]
//                  [--lab-worker CMD] [--time-scale N] [--poll-ms N] [--crash-after-enact OP]

import { existsSync, mkdirSync, writeFileSync } from "node:fs"
import { join, resolve } from "node:path"
import { pathToFileURL } from "node:url"
import { createHash } from "node:crypto"
import { readFileSync } from "node:fs"
import { decide as devDecide, type DecideOptions, type Program } from "../sdk/engine.ts"
import type { Decision } from "../sdk/history.ts"
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
  if (kind === "timer" || kind === "marker") return
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

// ---- versions ---------------------------------------------------------------------------------

interface Loaded {
  readonly version: string
  readonly program: Program
  readonly decide: (program: Program, history: ReadonlyArray<HistoryEvent>, options?: DecideOptions) => Decision
}

// A .mjs bundle carries its own engine; a .ts file is a development program on the shared SDK.
const loadFile = async (file: string, version?: string): Promise<Loaded> => {
  const mod = await import(pathToFileURL(resolve(file)).href)
  if (mod.decide) return { version: version ?? file, program: mod.program, decide: mod.decide }
  return { version: version ?? mod.default.version, program: mod.default, decide: devDecide }
}

const storeVersion = (file: string, name: string): string => {
  const put = st.json(["documents", "put", resolve(file), "--as", `doc/mac/programs/${name.replace(/\//g, "-")}`])
  return `${put.name}@${put.hash}`
}

const fetchVersion = async (ref: string): Promise<Loaded> => {
  const file = join(scratch, `${ref.split("@")[1]}.mjs`)
  if (!existsSync(file)) st.run(["documents", "get", ref, "--output", file])
  const bytes = readFileSync(file)
  if (createHash("sha256").update(bytes).digest("hex") !== ref.split("@")[1]) throw new Error(`version ${ref} does not match its hash`)
  return loadFile(file, ref)
}

const currentVersion = (history: ReadonlyArray<HistoryEvent>): string => {
  for (let i = history.length - 1; i >= 0; i--) {
    const e = history[i]!
    if (e.type === "upgraded") return e.version
    if (e.type === "started") return e.version
  }
  throw new Error("no started event")
}

const reloadSubject = `custom/mac/reload/${run}`
let lastReloadCheck = 0
const handledReloads = new Set<string>()

// The newest reload request this driver has not settled, if any.
const pendingReload = (): { id: string; version: string } | undefined => {
  const claims = st.jsonLines(["subject", "history", reloadSubject, "--limit", "200"])
  for (const c of claims) if (c.kind === "custom.mac.reload.answered") handledReloads.add(c.body.fields.request)
  const request = claims.filter((c) => c.kind === "custom.mac.reload.requested").at(-1)
  if (!request || handledReloads.has(request.id)) return undefined
  return { id: request.id, version: request.body.fields.version }
}

const answerReload = (request: { id: string; version: string }, outcome: string, reason: string) => {
  handledReloads.add(request.id)
  st.run(["claim", reloadSubject, "custom.mac.reload.answered", "--actor", actor,
    "--field", `request=${request.id}`, "--field", `version=${request.version}`,
    "--field", `outcome=${outcome}`, "--field", `reason=${reason}`, "--idempotency-key", `${run}/reload/${request.id}`])
  log(`reload to ${request.version}: ${outcome}: ${reason}`)
}

// The same rules as exp4-hmr/upgrade.ts, inline so the driver has no other dependency.
const tryReload = async (history: Array<HistoryEvent>, loaded: Loaded): Promise<Loaded> => {
  const request = pendingReload()
  if (!request || request.version === loaded.version) return loaded
  const next = await fetchVersion(request.version)
  const check = next.decide(next.program, history, { onChangedSpec: "report" })
  if (check.divergence) {
    answerReload(request, "refused", `event ${check.divergence.at}, op ${check.divergence.op}: ${check.divergence.reason}`)
    return loaded
  }
  const open = (check.changes ?? []).filter((c) => !c.settled)
  if (open.length > 0) {
    log(`reload to ${request.version} waits: ${open.map((c) => c.op).join(", ")} still open under the old definition`)
    return loaded
  }
  const accepted = (check.changes ?? []).map((c) => c.op)
  append(history, { type: "upgraded", at: now(), version: request.version, ...(accepted.length ? { accepted } : {}) })
  answerReload(request, "moved", accepted.length ? `finished ops defined differently: ${accepted.join(", ")}` : "the history replays exactly")
  return next
}

// ---- the loop ---------------------------------------------------------------------------------

const main = async () => {
  // --program doc/mac/programs/NAME@SHA names a stored version; a path is for development.
  const given = programFile.startsWith("doc/") ? await fetchVersion(programFile) : await loadFile(programFile)
  const history = readJournal()
  let loaded: Loaded
  if (history.length === 0) {
    const show = st.json(["missions", "show", `mission-run/${run}`])
    const input = JSON.parse(show.inputs?.["program-input"]?.value ?? "{}")
    const version = programFile.startsWith("doc/")
      ? programFile
      : programFile.endsWith(".mjs")
        ? storeVersion(programFile, given.program.name)
        : given.version
    loaded = { ...given, version }
    append(history, { type: "started", at: Date.now(), program: given.program.name, version, input })
    log(`started ${given.program.name} ${version} as ${run}`)
  } else {
    const version = currentVersion(history)
    loaded = version.startsWith("doc/") ? await fetchVersion(version) : given
    log(`resumed ${loaded.program.name} ${version} as ${run}: replaying ${history.length} journal events`)
  }
  const last = history.at(-1)!.at
  clockBase = { real: Date.now(), program: Math.max(last, scale === 1 ? Date.now() : last) }

  for (;;) {
    loaded = await tryReload(history, loaded)
    const decision = loaded.decide(loaded.program, history)
    if (decision.divergence) {
      log(`divergence at event ${decision.divergence.at}: ${decision.divergence.op}: ${decision.divergence.reason}`)
      st.run(["claim", resultSubject, "custom.mac.result", "--actor", actor, "--field", "state=diverged", "--field", `divergence=${JSON.stringify(decision.divergence)}`])
      // Hold, as a broken gate does: the run waits for a person to revise the program.
      for (;;) sleep(60_000)
    }
    for (const command of decision.commands) {
      if (command.type === "schedule") {
        enact(loaded.program, command.op, command.kind, command.spec)
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
      // A reload request is a graph write like any other; look for one every few seconds.
      if (Date.now() - lastReloadCheck > 5000) {
        lastReloadCheck = Date.now()
        if (pendingReload()) break
      }
    }
  }
}

main().catch((error) => {
  console.error(error)
  process.exit(1)
})
