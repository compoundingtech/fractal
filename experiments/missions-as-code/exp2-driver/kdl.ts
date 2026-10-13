// Renders one program op as the KDL mission a person would write for it today.
//
// Every op becomes a one-step mission run, so web, tui and ios show it like any other run:
// a step is a step with its seat, a gate is an agentless step holding that gate, an ask is a
// step assigned to the person, and a wait is an agentless step holding the condition.
// Timers have no st form with an exact wake; the driver keeps them (see FINDINGS.md).

import { hashJson } from "../sdk/hash.ts"
import type { OpKind } from "../sdk/history.ts"

// Lab mode replaces what the lab cannot provide: seats that need model logins and checks that
// need GitHub. Each becomes an agentless exec worker whose exit code is the outcome.
export interface LabOptions {
  readonly worker: string // command; receives the op ID as its first argument
}

export interface Rendered {
  readonly mission: string
  readonly kdl: string
  // The subject whose claims carry the worker's summary and evidence.
  readonly step: string
}

const q = (text: string): string =>
  `"${text.replace(/\\/g, "\\\\").replace(/"/g, '\\"').replace(/\n/g, "\\n").replace(/\$\{/g, "$${")}"`

const list = (value: unknown): Array<string> =>
  value === undefined ? [] : Array.isArray(value) ? value.map(String) : [String(value)]

const gateBody = (spec: any, lab: LabOptions | undefined, op: string): { attrs: string; body: Array<string> } => {
  if (spec.human) {
    const body = [`reviewer ${q(spec.human.reviewer)}`]
    if (spec.human.question) body.push(`question ${q(spec.human.question)}`)
    for (const r of list(spec.human.review)) body.push(`review ${q(r)}`)
    return { attrs: ` type="human"`, body }
  }
  if (lab && (spec.ciPassed || spec.merged)) {
    return { attrs: "", body: [`field "exit_code" "exec/$\{ST_MISSION_RUN}/worker" "is" 0`] }
  }
  if (spec.exec) {
    const body = [`exec ${q(spec.exec)}`, `host ${q(spec.host ?? "local")}`, `workspace ${spec.workspace ? q(spec.workspace) : '"${ST_WORKSPACE}"'}`]
    if (spec.timeLimit) body.push(`time-limit ${q(spec.timeLimit)}`)
    return { attrs: "", body }
  }
  if (spec.ciPassed) {
    const at = spec.commit ? ` commit=${q(spec.commit)}` : spec.branch ? ` branch=${q(spec.branch)}` : ""
    return { attrs: "", body: [`ci-passed ${q(spec.ciPassed)} repo=${q(spec.repo)}${at}`] }
  }
  if (spec.merged) return { attrs: "", body: [`merged ${q(spec.merged)}`] }
  if (spec.document) return { attrs: "", body: [`document ${q(spec.document)}`] }
  if (spec.exists) return { attrs: "", body: [`exists ${q(spec.exists)}`] }
  if (spec.field) {
    const [path, subject, operator, value] = spec.field
    return { attrs: "", body: [`field ${q(path)} ${q(subject)} ${q(operator)} ${typeof value === "string" ? q(value) : JSON.stringify(value)}`] }
  }
  throw new Error(`op ${op}: no st form for gate ${JSON.stringify(spec)}`)
}

const workerExec = (lab: LabOptions, op: string): Array<string> => [
  `exec "worker" {`,
  `  host "local"`,
  `  workspace "\${ST_WORKSPACE}"`,
  `  command ${q(`${lab.worker} ${op}`)}`,
  `  restart "never"`,
  `}`
]

const indent = (lines: Array<string>, by: string) => lines.map((l) => by + l)

export const render = (
  program: string,
  op: string,
  kind: OpKind,
  spec: any,
  lab?: LabOptions
): Rendered => {
  const step: Array<string> = []
  const attrs: Array<string> = []
  let goal = "Run one op of a mission program."
  if (kind === "step") {
    goal = list(spec.goal)[0] ?? spec.title ?? goal
    if (spec.title) step.push(`title ${q(spec.title)}`)
    for (const g of list(spec.goal).slice(0, 3)) step.push(`goal ${q(g)}`)
    for (const c of list(spec.constraint)) step.push(`constraint ${q(c)}`)
    if (spec.context !== undefined) step.push(`constraint ${q(`Program context: ${JSON.stringify(spec.context)}`)}`)
    if (lab) {
      step.push("agentless", ...workerExec(lab, op))
      step.push(`gate "the worker succeeded" {`, `  field "exit_code" "exec/$\{ST_MISSION_RUN}/worker" "is" 0`, `}`)
    } else {
      if (spec.assignedTo) step.push(`assigned-to ${q(spec.assignedTo)}`)
      for (const a of list(spec.availableTo)) step.push(`available-to ${q(a)}`)
      if (spec.freshContext) step.push("fresh-context")
      if (spec.timeout) attrs.push(` timeout=${q(spec.timeout)}`)
    }
    for (const d of list(spec.documents)) step.push(`document ${q(d)}`)
    if (spec.retry) step.push(`retry { attempts ${spec.retry.attempts}${spec.retry.backoff ? `; backoff ${q(spec.retry.backoff)}` : ""} }`)
    for (const g of spec.gates ?? []) {
      const { attrs: ga, body } = gateBody(g, lab, op)
      step.push(`gate ${q(g.name)}${ga} {`, ...indent(body, "  "), `}`)
    }
  } else if (kind === "gate" || kind === "wait") {
    goal = kind === "gate" ? `Check ${op} for a mission program.` : `Wait for ${op} for a mission program.`
    step.push(`title ${q(`${kind === "gate" ? "Check" : "Wait for"} ${op}`)}`, "agentless")
    if (spec.run) {
      step.push(`after-run ${q(spec.run)}`)
    } else {
      if (lab && (spec.ciPassed || spec.merged)) step.push(...workerExec(lab, op))
      const { attrs: ga, body } = gateBody(spec, lab, op)
      step.push(`gate ${q(op)}${ga} {`, ...indent(body, "  "), `}`)
    }
  } else if (kind === "ask") {
    goal = spec.question
    step.push(`title ${q(spec.title)}`, `goal ${q(spec.question)}`, `assigned-to ${q(spec.for)}`)
    if (spec.answers?.length) {
      const choices = spec.answers.map((a: any) => `${a.id} (${a.label})`).join(", ")
      step.push(`constraint ${q(`Answer with the first word of your response: ${choices}.`)}`)
    }
  } else {
    throw new Error(`op ${op}: a ${kind} has no mission form`)
  }
  const body = [
    `version 2`,
    ``,
    `mission ${q("MISSION")} state="ready" {`,
    `  concurrent-runs`,
    `  goal ${q(goal)}`,
    `  step "work"${attrs.join("")} {`,
    ...indent(step, "    "),
    `  }`,
    `}`,
    ``
  ].join("\n")
  // One mission per distinct op shape: identical specs share a published revision.
  const mission = `mac/${program}/${kind}/${hashJson(body)}`
  return { mission, kdl: body.replace(`"MISSION"`, q(mission)), step: "work" }
}
