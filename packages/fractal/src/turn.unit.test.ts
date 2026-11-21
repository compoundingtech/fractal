import { describe, expect, it } from '@effect/vitest'
import { Effect, Schema } from 'effect'

import { Fractal, type WorkerReport, type ProgramExit, type StepExecution, type ResourceError, type ChildFailed, type ProgramServices } from './effect.ts'
import { fakeHost, start } from './program-host.fixture.ts'

const meta = { url: 'file:///tree/fractal/seat-turn/mission.ts' }
const reportTo = { id: 'example/owner' }
const receipt: StepExecution = {
  id: 'execution/claimed', step_subject: 'step-run/turn/work', attempt: 1,
  agent: 'agent/example/worker', incarnation_id: 'incarnation/real', declared_model: 'declared',
  observed_models: ['observed'], fallback: false, started_at_unix_ms: 1, ended_at_unix_ms: 2,
  duration_ms: 1, status: 'completed', trace_id: null, exit_evidence: 'work.completed', usage: null,
}
const report: WorkerReport = {
  run: 'mission-run/episode', step: 'step-run/episode/work', summary: 'Verified.', evidence: ['resource/mission-run/episode/verdict'],
  products: { verdict: { kind: 'custom.example.verdict', accepted: true } }, executions: [receipt],
}
const options = { id: 'turn', agent: 'agent/example/worker', instructions: 'Inspect; do not run $(commands).', context: '{"scope":"literal"}' }
const mission = (body: () => Effect.Effect<WorkerReport, ResourceError | ChildFailed, ProgramServices>) => Fractal.mission({
  meta, id: 'fractal/seat-turn', reportTo, goal: 'Inspect.', input: Schema.Struct({}), output: Fractal.WorkerReport,
}, body)

describe('native assigned turns', () => {
  it.effect('uses the real keyed resource protocol and preserves episode identity and receipts', () => Effect.gen(function* () {
    const requests: unknown[] = []
    const program = mission(() => Fractal.turn(options))
    const host = fakeHost({ payload: {}, resource: async (request) => {
      requests.push(request)
      return { _tag: 'Success', value: report }
    } })
    expect(yield* start(program, host)).toEqual({ _tag: 'Success', value: report })
    expect(requests).toEqual([{ key: 'turn', operation: 'agent.turn', payload: {
      agent: options.agent, instructions: options.instructions, context: options.context,
    } }])
    const replay = fakeHost({ payload: {}, journal: [['turn', { _tag: 'Success', value: report }]] })
    expect(yield* start(program, replay)).toEqual({ _tag: 'Success', value: report })
    expect(replay.recorded).toEqual([])
  }))

  it.effect('uses occurrence keys on repeated turns without inventing execution identities', () => Effect.gen(function* () {
    const keys: string[] = []
    const program = mission(() => Effect.gen(function* () {
      yield* Fractal.turn(options)
      return yield* Fractal.turn(options)
    }))
    expect(yield* start(program, fakeHost({ payload: {}, resource: async ({ key }) => {
      keys.push(key)
      return { _tag: 'Success', value: report }
    } }))).toEqual({ _tag: 'Success', value: report })
    expect(keys).toEqual(['turn', 'turn#2'])
  }))

  it.effect('preserves native refusal and terminal failure rather than fabricating reports', () => Effect.gen(function* () {
    for (const error of [
      { _tag: 'ResourceError', code: 'resource-operation-refused', message: 'Seat unavailable.' },
      { _tag: 'ChildFailed', run: report.run, status: 'failed', reason: 'Worker failed.', executions: [receipt] },
    ]) {
      const exit: ProgramExit = { _tag: 'Failure', error }
      const program = mission(() => Fractal.turn(options))
      expect(yield* start(program, fakeHost({ payload: {}, resource: async () => exit }))).toEqual(exit)
      expect(yield* start(program, fakeHost({ payload: {}, journal: [['turn', exit]] }))).toEqual(exit)
    }
  }))

  it.effect('decodes the publishable standard protocol without shell or input-name collisions', () => Effect.gen(function* () {
    const inputs = { agent: 'not an assignee', instructions: 'not the goal', context: '$(inert)', goal: 'original goal', nested: '${input.literal}' }
    const products = [{ path: 'review/verdict', kind: 'custom.example.verdict', fields: { kind: 'custom.example.verdict', accepted: true, count: 2 } }]
    const program = Fractal.seatTurn({ meta, reportTo })
    expect(program.intent.id).toBe('fractal/seat-turn')
    expect(program.intent.authority).toEqual({ start: ['native-turn'] })
    expect(Object.keys(program.intent.input.fields)).toEqual(['agent', 'instructions', 'context'])
    for (const freshContext of [undefined, false, true]) {
      const context = { inputs, products, ...(freshContext === undefined ? {} : { freshContext }) }
      const host = fakeHost({ payload: { agent: options.agent, instructions: options.instructions, context: JSON.stringify(context) }, resource: async (request) => {
        expect(request).toEqual({ key: 'turn', operation: 'agent.turn', payload: {
          agent: options.agent, instructions: options.instructions, context: JSON.stringify(inputs), products,
          ...(freshContext === undefined ? {} : { freshContext }),
        } })
        return { _tag: 'Success', value: report }
      } })
      expect(yield* start(program, host)).toEqual({ _tag: 'Success', value: report })
    }
    expect(Fractal.seatTurn({ meta, reportTo, id: 'example/custom-turn' }).intent.id).toBe('example/custom-turn')
  }))

  it.effect('refuses malformed context and non-scalar product constraints before native admission', () => Effect.gen(function* () {
    for (const context of ['not JSON', '{}', JSON.stringify({ inputs: {}, products: [{ path: 'x', kind: 'k', fields: { nested: {} } }] })]) {
      expect(yield* start(Fractal.seatTurn({ meta, reportTo }), fakeHost({ payload: {
        agent: options.agent, instructions: 'Inspect.', context,
      } }))).toMatchObject({ _tag: 'Failure', error: { _tag: 'ResourceError', code: 'resource-operation-refused' } })
    }
  }))

  it.effect('explicit report mode selects relative products and real episode receipts, not child accounting', () => Effect.gen(function* () {
    const program = Fractal.mission({ meta, id: 'fractal/seat-turn', reportTo, goal: 'Inspect.', input: Schema.Struct({}),
      output: Schema.Struct({ value: Schema.Record(Schema.String, Schema.Unknown), executions: Schema.optionalKey(Schema.Array(Fractal.StepExecution)) }),
    }, () => Fractal.step({ id: 'review', success: Schema.Record(Schema.String, Schema.Unknown), execution: true }).pipe(
      Effect.provide(Fractal.Worker.layer({ mission: 'fractal/seat-turn', report: true, product: ['verdict'] })),
    ))
    const host = fakeHost({ payload: {}, child: async () => ({ _tag: 'Success', value: {
      run: 'mission-run/program-child', status: 'completed', result: report, executions: [],
      products: { 'resource/mission-run/program-child/verdict': { fake: true } },
    } }) })
    expect(yield* start(program, host)).toEqual({ _tag: 'Success', value: { value: report.products, executions: [receipt] } })
    for (const result of [undefined, {}, { ...report, products: {} }]) {
      const malformed = fakeHost({ payload: {}, child: async () => ({ _tag: 'Success', value: { run: 'mission-run/child', status: 'completed', result } }) })
      expect(yield* start(program, malformed)).toMatchObject({ _tag: 'Failure', error: { _tag: 'Defect' } })
    }
  }))
})

describe('native child output', () => {
  it.effect('returns actual program output without turning it into a resource claim', () => Effect.gen(function* () {
    const result = { answer: 'actual program output', nested: { value: 3 } }
    const program = Fractal.mission({
      meta, id: 'fractal/seat-turn', reportTo, goal: 'Read child output.',
      input: Schema.Struct({}), output: Schema.Unknown,
    }, () => Fractal.run('example/native-program', {}))
    const done = { run: 'mission-run/child', status: 'completed', result }
    expect(yield* start(program, fakeHost({ payload: {}, child: async () => ({ _tag: 'Success', value: done }) }))).toEqual({
      _tag: 'Success', value: done,
    })
    expect(yield* start(program, fakeHost({ payload: {}, journal: [['example/native-program', { _tag: 'Success', value: done }]] }))).toEqual({
      _tag: 'Success', value: done,
    })
  }))
})
