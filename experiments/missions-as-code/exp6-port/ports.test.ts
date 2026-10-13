import assert from "node:assert/strict"
import { test } from "node:test"
import { simulate, type Script } from "../sdk/sim.ts"
import change from "./change.ts"
import ledger from "./ledger-changes.ts"
import refresh from "./refresh-then-answer.ts"

const minute = 60_000
const hour = 60 * minute
const done = (after: number, summary = "done", evidence: Array<string> = []) => ({
  after,
  value: { summary, evidence, attempt: 1 }
})
const pass = (after = 10 * minute) => ({ after, value: { answer: "pass", evidence: [] } })
const red = (reason = "orchard-ci failed: unit") => ({ after: 10 * minute, error: { reason } })
const scheduled = (log: Array<string>) =>
  log.filter((l) => l.includes(" schedule ")).map((l) => l.split(" ").slice(3).join(" "))
const status = (trace: ReturnType<typeof simulate>) => trace.decisions.at(-1)!.status

// ---- change (simple) --------------------------------------------------------------------------

test("change: five yields in KDL order", () => {
  const trace = simulate(change, { request: "add a ledger index", run: "orchard/change/1" }, (_op, kind) =>
    kind === "gate" ? pass(2 * hour) : done(hour)
  )
  assert.deepEqual(scheduled(trace.log), ["implement", "verify", "open-pull-request", "approve-merge", "timer", "merge"])
  assert.equal(status(trace).state, "completed")
})

test("change: an unanswered approval fails the run after 24h, like the agentless step's budget", () => {
  const trace = simulate(change, { request: "x", run: "orchard/change/2" }, (_op, kind) =>
    kind === "gate" ? "never" : done(hour)
  )
  assert.ok(trace.log.some((l) => l.endsWith("cancel approve-merge")))
  assert.equal(status(trace).state, "failed")
})

// ---- refresh-then-answer (queue, nested missions, loops, finally) -----------------------------

const refreshScript = (redRounds: Record<number, number>): Script => {
  const seen: Record<number, number> = {}
  return (op, kind, spec) => {
    if (kind === "gate" && op.endsWith("/checks")) {
      const pr = op.startsWith("refresh-210") ? 210 : 233
      seen[pr] = (seen[pr] ?? 0) + 1
      return seen[pr]! <= (redRounds[pr] ?? 0) ? red(`checks failed on ${pr}, round ${seen[pr]}`) : pass(30 * minute)
    }
    if (spec?.message) return done(minute, "sent")
    return done(hour)
  }
}

test("refresh-then-answer: the queue runs job 210 to its final message before 233 starts", () => {
  const trace = simulate(refresh, {}, refreshScript({}))
  const ops = scheduled(trace.log).filter((o) => !o.endsWith("/timer")) // the loop's 8h deadline
  assert.deepEqual(ops.slice(0, 6), [
    "refresh-210/merge-main",
    "refresh-210/green/1/fix",
    "refresh-210/green/1/checks",
    "refresh-210/tell-chief",
    "refresh-210/tell-chief-finished",
    "answer-233/answer"
  ])
  assert.equal(status(trace).state, "completed")
})

test("refresh-then-answer: a red round feeds the next fix the failure", () => {
  const trace = simulate(refresh, {}, refreshScript({ 210: 2 }))
  const fixes = trace.decisions.flatMap((d) => d.commands).filter((c) => c.type === "schedule" && c.op.startsWith("refresh-210/green/") && c.op.endsWith("/fix"))
  assert.deepEqual(fixes.map((c) => c.op), ["refresh-210/green/1/fix", "refresh-210/green/2/fix", "refresh-210/green/3/fix"])
  assert.deepEqual((fixes[2] as any).spec.context, { feedback: "checks failed on 210, round 2" })
  assert.equal(status(trace).state, "completed")
})

test("refresh-then-answer: three red rounds raise attention, run finally, and stop the queue", () => {
  const trace = simulate(refresh, {}, refreshScript({ 210: 3 }))
  const ops = scheduled(trace.log)
  assert.deepEqual(ops.slice(-2), ["refresh-210/green-exhausted", "refresh-210/tell-chief-finished"])
  assert.ok(!ops.some((o) => o.startsWith("answer-233")))
  assert.equal((status(trace) as any).error._tag, "LoopExhausted")
})

// ---- ledger-changes (the intake and queue path the KDL keeps in prose) ------------------------

const ledgerScript = (options: { ciRed?: number; noGo?: number; eject?: number } = {}): Script => {
  let ci = 0
  let intakeCalls = 0
  let merges = 0
  return (op, kind) => {
    if (kind === "gate") return ++ci <= (options.ciRed ?? 0) ? red() : pass()
    if (kind === "wait") return ++merges <= (options.eject ?? 0) ? { after: hour, error: { reason: "ejected: conflict with main" } } : done(hour)
    if (op.includes("/intake") && !op.includes("design")) return ++intakeCalls <= (options.noGo ?? 0) ? done(hour, "NO-GO: missing a concurrency test") : done(hour, "GO", ["doc/intake/go"])
    if (op.includes("design-intake")) return done(hour, "GO")
    if (op.includes("open-pull-request")) return done(hour, "opened", [`orchard/app#${op.includes("reads") ? 301 : 302}`])
    return done(hour)
  }
}

test("ledger-changes: both slices land through CI, intake, queue and merge", () => {
  const trace = simulate(ledger, { run: "orchard/ledger-changes/1" }, ledgerScript())
  assert.deepEqual(status(trace), { state: "completed", value: { shipped: ["orchard/app#301", "orchard/app#302"] } })
  assert.equal(scheduled(trace.log).at(-1), "stop-builder")
})

test("ledger-changes: red CI is fixed, a NO-GO goes round again, an ejection is fixed and requeued", () => {
  const trace = simulate(ledger, { run: "orchard/ledger-changes/2" }, ledgerScript({ ciRed: 1, noGo: 1, eject: 1 }))
  const reads = scheduled(trace.log).filter((o) => o.startsWith("land-reads/"))
  assert.deepEqual(reads, [
    "land-reads/open-pull-request",
    "land-reads/pass-1/ci-1/ci",
    "land-reads/pass-1/ci-1/fix-ci",
    "land-reads/pass-1/ci-2/ci",
    "land-reads/pass-1/intake",
    "land-reads/pass-1/address-intake",
    "land-reads/pass-2/ci-1/ci",
    "land-reads/pass-2/intake",
    "land-reads/pass-2/queue",
    "land-reads/pass-2/merged",
    "land-reads/pass-2/fix-ejection",
    "land-reads/pass-3/ci-1/ci",
    "land-reads/pass-3/intake",
    "land-reads/pass-3/queue",
    "land-reads/pass-3/merged"
  ])
  assert.equal(status(trace).state, "completed")
})

test("ledger-changes: CI that never goes green fails the run and still stops the builder", () => {
  const trace = simulate(ledger, { run: "orchard/ledger-changes/3" }, ledgerScript({ ciRed: 99 }))
  assert.equal((status(trace) as any).error._tag, "CiNeverGreen")
  assert.equal(scheduled(trace.log).at(-1), "stop-builder")
})
