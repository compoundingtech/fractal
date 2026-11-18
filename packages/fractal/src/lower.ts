import { Effect, Schema } from 'effect'

import type { MissionIntent } from './define.ts'
import { MissionSchema, missionWire, type MissionInput, type AgentReference } from './mod.ts'
import * as Native from './program.ts'
import type { SubjectDefinition } from './subject-ref.ts'

/** Compilation fails closed before a bundle can be published. */
export class GraphCompileRefusal extends Schema.TaggedError<GraphCompileRefusal>()('GraphCompileRefusal', {
  step: Schema.String,
  construct: Schema.String,
  reason: Schema.String,
}) {}

export class GraphGateFailed extends Schema.TaggedError<GraphGateFailed>()('GraphGateFailed', {
  step: Schema.String,
  gate: Schema.String,
  reason: Schema.String,
}) {}

/** The published child mission's explicit instruction-input protocol. */
export interface WorkerBinding {
  readonly mission: string
  readonly goalInput: string
}

export interface LowerOptions {
  /** Path-derived mission id, supplied by the authoring tree. */
  readonly id: string
  /** The program's execution host; remote gate commands are not supported. */
  readonly host: string
  /** Explicit routing from graph agent id to a published seat-turn mission. */
  readonly workers: Readonly<Record<string, WorkerBinding>>
  /** Resolve imported agent references using the same tree as the graph renderer. */
  readonly resolveAgent?: (value: unknown) => AgentReference
}

const Product = Schema.Record(Schema.String, Schema.Unknown)
const Products = Schema.Record(Schema.String, Product)
const refuse = (step: string, construct: string, reason = 'Not supported by W1 graph lowering'): never => {
  throw new GraphCompileRefusal({ step, construct, reason })
}

/** Compile a resolved graph definition. Export the result as the module's default bundle entry. */
export const lower = (
  graph: SubjectDefinition<'mission', MissionIntent, never>,
  options: LowerOptions,
) => {
  if (typeof graph.intent === 'function') refuse(options.id, 'context factory', 'Resolve the mission factory before lowering')
  const intent = graph.intent as MissionIntent
  if ('id' in intent && intent.id !== options.id) refuse(options.id, 'mission id', 'The bound id must equal the graph mission id')
  // Inspect authoring-only constructs before the graph schema can erase or reject them.
  const inspect = (value: unknown, step: string): void => {
    if (typeof value !== 'object' || value === null) return
    if (Array.isArray(value)) {
      for (const item of value) inspect(item, step)
      return
    }
    if ('id' in value && typeof value.id === 'string') step = value.id
    for (const [key, item] of Object.entries(value)) {
      if (item === undefined) continue
      if (['waitFor', 'watch', 'observer', 'observers', 'subscriptions', 'on', 'timing', 'review', 'waitUntil'].includes(key)) refuse(step, key === 'timing' ? 'Timing' : key === 'review' ? 'Review' : key)
      if (['steps', 'finally', 'round', 'gates', 'schedule'].includes(key)) inspect(item, step)
    }
  }
  inspect(intent, options.id)
  const wire = missionWire({ ...intent, id: options.id } as MissionInput<string>, (value, step) => {
    if (typeof value !== 'object' || value === null || !('_tag' in value) || value._tag !== 'SubjectRef') return value
    if (options.resolveAgent === undefined) return refuse(step, 'agent binding', 'Imported agent references need the authoring tree resolver')
    return options.resolveAgent(value)
  })
  const mission = Schema.decodeUnknownSync(MissionSchema)(wire)
  for (const key of ['finally', 'completion', 'gates', 'constraints', 'lanes', 'reportStalls', 'reportCompleted', 'observers', 'subscriptions'] as const) {
    if (mission[key] !== undefined) refuse(options.id, key)
  }
  const steps = mission.steps.map((step) => {
    if (step.maxRounds !== undefined) return refuse(step.id, 'round')
    for (const key of ['usesMission', 'producesMission', 'exec', 'retry', 'timeout', 'program', 'documents', 'resources', 'docs', 'observers', 'subscriptions'] as const) {
      if (step[key] !== undefined) refuse(step.id, key === 'usesMission' ? 'waitFor' : key)
    }
    for (const dependency of step.dependsOn ?? []) {
      if (dependency.state !== 'completed') refuse(step.id, 'dependsOn.state')
    }
    const agent = step.assignedTo ?? mission.assignedTo
    if (step.agentless !== true && (agent === undefined || !('id' in agent))) refuse(step.id, 'assignedTo')
    const worker = step.agentless === true || agent === undefined || !('id' in agent) ? undefined : options.workers[agent.id]
    if (step.agentless !== true && worker === undefined) refuse(step.id, 'worker binding', 'No seat-turn mission bound to the assigned agent')
    if (worker !== undefined && mission.inputs?.some((input) => input.name === worker.goalInput)) refuse(step.id, 'worker input binding', `Instruction input ${worker.goalInput} collides with a declared graph input; bind an unused child input key`)
    for (const product of step.produces ?? []) {
      if (!product.subject.startsWith('resource/mission-run/${ST_MISSION_RUN}/')) refuse(step.id, 'product.resource subject')
    }
    for (const gate of step.gates ?? []) {
      if (gate.kind === 'exec') {
        if (gate.host !== options.host) refuse(step.id, 'gate.exec.host', `Gate host ${gate.host} differs from program host ${options.host}`)
        if (gate.timeLimit !== undefined) refuse(step.id, 'gate.exec.timeLimit')
      } else if (gate.kind === 'human') {
        if (gate.mode === 'feedback') refuse(step.id, 'gate.human.feedback')
        if (gate.review !== undefined) refuse(step.id, 'Review')
      } else if (gate.kind === 'field') {
        if (gate.operator !== 'is') refuse(step.id, 'gate.field operator')
        if (!mission.steps.some((producer) => producer.maxRounds === undefined && producer.produces?.some((product) => product.subject === gate.subject))) refuse(step.id, 'gate.fieldIs subject', 'Field checks require a product of this graph')
      } else refuse(step.id, `gate.${gate.kind}`)
    }
    if (step.agentless === true && step.produces !== undefined) refuse(step.id, 'agentless product')
    return { ...step, worker }
  })
  const readySets: (typeof steps)[] = []
  const scheduled = new Set<string>()
  while (scheduled.size < steps.length) {
    const ready = steps.filter((step) => !scheduled.has(step.id) && (step.dependsOn ?? []).every((dependency) => scheduled.has(dependency.step)))
    if (ready.length === 0) refuse(steps.find((step) => !scheduled.has(step.id))!.id, 'dependsOn cycle')
    readySets.push(ready)
    for (const step of ready) scheduled.add(step.id)
  }
  const ancestors = (id: string): Set<string> => {
    const result = new Set<string>([id])
    const visit = (id: string): void => {
      for (const dependency of steps.find((step) => step.id === id)!.dependsOn ?? []) {
        if (!result.has(dependency.step)) {
          result.add(dependency.step)
          visit(dependency.step)
        }
      }
    }
    visit(id)
    return result
  }
  for (const step of steps) {
    const predecessors = ancestors(step.id)
    for (const gate of step.gates ?? []) {
      if (gate.kind === 'field' && !steps.some((producer) => predecessors.has(producer.id) && producer.produces?.some((product) => product.subject === gate.subject))) refuse(step.id, 'gate.fieldIs dependsOn', 'The product producer must be this step or a dependency ancestor')
    }
  }
  for (const input of mission.inputs ?? []) {
    if (input.kind !== 'text') refuse(options.id, `input.${input.kind}`)
  }
  const input = Schema.Struct(Object.fromEntries((mission.inputs ?? []).map((field) => [field.name, Schema.String])))
  const plan = steps.map((step) => ({ id: step.id, dependsOn: (step.dependsOn ?? []).map((dependency) => dependency.step), gates: step.gates ?? [] }))
  const program = Native.mission({
    meta: { url: graph.url }, id: options.id, goal: mission.goal.join('\n'), reportTo: mission.reportTo,
    input,
    ...(mission.budget === undefined ? {} : { budget: mission.budget }),
    ...(mission.concurrentRuns === undefined ? {} : { concurrentRuns: mission.concurrentRuns }),
    ...(mission.authority === undefined ? {} : { authority: mission.authority }),
    ...(mission.resources === undefined ? {} : { resources: mission.resources }),
    ...(mission.docs === undefined ? {} : { docs: mission.docs }),
    ...(mission.stalledAfter === undefined ? {} : { stalledAfter: mission.stalledAfter }),
    ...(mission.timeout === undefined ? {} : { timeout: mission.timeout }),
  }, (values) => Effect.gen(function* () {
    const env = yield* Native.env
    const products = new Map<string, Readonly<Record<string, unknown>>>()
    const expand = (text: string): string => text.replace(/\$\{(input\.([^}]+)|ST_MISSION_RUN)\}/gu, (_, name: string, field: string) => name === 'ST_MISSION_RUN' ? env.ST_MISSION_RUN!.replace(/^mission-run\//u, '') : values[field]!)
    const execute = (step: typeof steps[number]) => Effect.gen(function* () {
      const fail = (gate: string, reason: string) => Effect.fail(new GraphGateFailed({ step: step.id, gate, reason }))
      const declaredProducts = step.produces ?? []
      if (step.worker !== undefined) {
        const paths = declaredProducts.map((product) => expand(product.subject.slice('resource/mission-run/${ST_MISSION_RUN}/'.length)))
        const worker = Native.Worker.layer({ mission: step.worker.mission, input: { ...values, [step.worker.goalInput]: expand((step.goal ?? []).join('\n')) }, ...(paths.length === 0 ? {} : { product: paths }) })
        if (paths.length === 0) yield* Native.step({ id: step.id }).pipe(Effect.provide(worker))
        else {
          const values = yield* Native.step({ id: step.id, success: Products }).pipe(Effect.provide(worker))
          yield* Native.step({ id: `${step.id}/product` }, Effect.gen(function* () {
            for (const [index, product] of declaredProducts.entries()) {
              const value = values[paths[index]!]!
              for (const [field, expected] of Object.entries(product.fields)) {
                if (value[field] !== (typeof expected === 'string' ? expand(expected) : expected)) return yield* fail('product', `Product field ${field} does not match its declared constraint`)
              }
            }
          }))
          for (const [index, product] of declaredProducts.entries()) products.set(expand(product.subject), values[paths[index]!]!)
        }
      }
      const gates = step.gates ?? []
      for (const [index, gate] of gates.entries()) {
        const id = step.worker === undefined && gates.length === 1 ? step.id : `${step.id}/gate/${index}`
        if (gate.kind === 'human') {
          const answer = yield* Native.ask({
            id, for: gate.reviewer, title: expand(gate.question ?? ((step.goal ?? []).join('\n') || gate.name)),
            request: {
              version: 1, type: 'decision', question: expand(gate.question ?? gate.name),
              why_person: 'The graph mission requires your approval before this step can complete.',
              answers: [
                { id: 'approve', label: 'Approve', outcome: 'accept', consequence: 'The step may complete and dependent steps may run.' },
                { id: 'reject', label: 'Reject', outcome: 'decline', consequence: 'The mission fails without starting dependent steps.' },
              ],
            },
            default: 'reject',
          })
          if (answer.resolved_by !== gate.reviewer.subject || answer.answer?.outcome !== 'accept' || answer.answer.id !== 'approve') return yield* fail(gate.name, 'Human approval was not granted')
        } else if (gate.kind === 'exec') {
          yield* Native.step({ id }, Effect.gen(function* () {
            const result = yield* Native.command(['sh', '-c', expand(gate.command)], { cwd: expand(gate.workspace), env: Object.fromEntries(Object.entries(gate.env ?? {}).map(([key, value]) => [key, expand(value)])) })
            if (result.code !== 0) return yield* fail(gate.name, `Command exited ${result.code}`)
          }))
        } else if (gate.kind === 'field') {
          yield* Native.step({ id }, Effect.gen(function* () {
            let value: unknown = products.get(expand(gate.subject))
            for (const field of gate.path.split('.')) value = typeof value === 'object' && value !== null && !Array.isArray(value) && Object.hasOwn(value, field) ? Reflect.get(value, field) : undefined
            if (value !== (typeof gate.value === 'string' ? expand(gate.value) : gate.value)) return yield* fail(gate.name, `Field ${gate.path} does not match`)
          }))
        }
      }
      if (step.worker === undefined && gates.length !== 1) yield* Native.step({ id: step.id }, Effect.void)
    })
    for (const ready of readySets) yield* Effect.all(ready.map(execute), { concurrency: 'unbounded' })
  }))
  return Object.assign(program, { graph: plan })
}
