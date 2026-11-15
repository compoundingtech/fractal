import { describe, expect, it } from '@effect/vitest'
import { Data, Effect, Schedule, Schema } from 'effect'

import {
  AskCancelled,
  ChildFailed,
  Fractal,
  type ProgramExit,
  type ProgramServices,
} from './effect.ts'
import { fakeHost, journalOf, start } from './program-host.fixture.ts'
import type { AgentRef } from './subject-ref.ts'

class StepFailed extends Data.TaggedError('StepFailed')<{
  readonly step: string
  readonly reason: string
}> {}
class Refused extends Schema.TaggedError<Refused>()('Refused', { reason: Schema.String }) {}

const reportTo: AgentRef = {
  _tag: 'SubjectRef',
  kind: 'agent',
  url: 'file:///tree/factory/agent.ts',
}
const meta = { url: 'file:///tree/factory/test/mission.ts' }
const Change = Schema.Struct({ change: Schema.String })

describe('program replay', () => {
  const effects: string[] = []
  const land = Fractal.mission(
    {
      meta,
      id: 'factory/test',
      goal: 'Merge the change.',
      reportTo,
      input: Change,
      output: Schema.Struct({ merged: Schema.String, errors: Schema.Number }),
    },
    ({ change }) =>
      Effect.gen(function* () {
        const checked = yield* Fractal.step(
          { id: 'check', success: Schema.Struct({ errors: Schema.Array(Schema.String) }) },
          Effect.sync(() => {
            effects.push('check')
            return { errors: [] }
          }),
        )
        const merged = yield* Fractal.step(
          { id: 'merge', success: Schema.String },
          Effect.sync(() => {
            effects.push('merge')
            return `merged ${change}`
          }),
        )
        return { merged, errors: checked.errors.length }
      }),
  )

  it.effect('returns journaled values without running their effects again', () =>
    Effect.gen(function* () {
      const first = fakeHost()
      expect(yield* start(land, first)).toEqual({
        _tag: 'Success',
        value: { merged: 'merged abc', errors: 0 },
      })
      expect(first.recorded).toEqual([
        { key: 'check', status: 'completed', value: { errors: [] } },
        { key: 'merge', status: 'completed', value: 'merged abc' },
      ])
      expect(effects.splice(0)).toEqual(['check', 'merge'])

      const replay = fakeHost({ journal: journalOf(first.recorded) })
      expect(yield* start(land, replay)).toEqual({
        _tag: 'Success',
        value: { merged: 'merged abc', errors: 0 },
      })
      expect(effects).toEqual([])
      expect(replay.recorded).toEqual([])
      expect(replay.started).toEqual([])

      const resume = fakeHost({ journal: journalOf(first.recorded.slice(0, 1)) })
      yield* start(land, resume)
      expect(effects.splice(0)).toEqual(['merge'])
      expect(resume.started).toEqual(['merge'])
    }),
  )

  it.effect('stops with a defect when the daemon refuses a journal write', () =>
    Effect.gen(function* () {
      const exit = yield* start(land, fakeHost({ nack: 'idempotency-conflict' }))
      expect(exit).toMatchObject({ _tag: 'Failure', error: { _tag: 'Defect' } })
      expect(JSON.stringify(exit)).toContain('idempotency-conflict')
      // The program does not continue past a result that is not durable.
      expect(effects.splice(0)).toEqual(['check'])
    }),
  )

  it.effect('ends the attempt on a refused journal write even when the body catches defects', () =>
    Effect.gen(function* () {
      const ran: string[] = []
      const catching = Fractal.mission(
        { meta, id: 'factory/test', goal: 'Merge.', reportTo, input: Change },
        () =>
          Effect.gen(function* () {
            yield* Effect.exit(
              Fractal.step(
                { id: 'check' },
                Effect.sync(() => ran.push('check')),
              ),
            )
            yield* Fractal.step(
              { id: 'merge' },
              Effect.sync(() => ran.push('merge')),
            )
          }),
      )
      const exit = yield* start(catching, fakeHost({ nack: 'idempotency-conflict' }))
      expect(exit).toEqual({
        _tag: 'Failure',
        error: {
          _tag: 'Defect',
          message: 'the daemon refused the journal entry for `check`: idempotency-conflict',
        },
      })
      expect(ran).toEqual(['check'])
    }),
  )
})

describe('typed failures', () => {
  const effects: string[] = []
  const publish = Fractal.mission(
    { meta, id: 'factory/test', goal: 'Publish.', reportTo, input: Change, output: Schema.String },
    () =>
      Effect.gen(function* () {
        const reverted = yield* Fractal.step(
          { id: 'publish' },
          Effect.suspend(() => {
            effects.push('publish')
            return Effect.fail(new StepFailed({ step: 'publish', reason: 'admission refused' }))
          }),
        ).pipe(Effect.catchTag('StepFailed', (e) => Effect.succeed(`reverted: ${e.reason}`)))
        const refused = yield* Fractal.step(
          { id: 'verify', error: Refused },
          Effect.suspend(() => {
            effects.push('verify')
            return Effect.fail(new Refused({ reason: 'stale' }))
          }),
        ).pipe(
          Effect.catchTag('Refused', (e) =>
            Effect.succeed(e instanceof Refused ? `rebuilt ${e.reason}` : 'plain'),
          ),
        )
        return `${reverted}; ${refused}`
      }),
  )

  it.effect('are journaled with their tag and fields and match catchTag on replay', () =>
    Effect.gen(function* () {
      const first = fakeHost()
      const exit = { _tag: 'Success', value: 'reverted: admission refused; rebuilt stale' }
      expect(yield* start(publish, first)).toEqual(exit)
      expect(first.recorded).toEqual([
        {
          key: 'publish',
          status: 'failed',
          error: { _tag: 'StepFailed', step: 'publish', reason: 'admission refused' },
        },
        { key: 'verify', status: 'failed', error: { _tag: 'Refused', reason: 'stale' } },
      ])
      expect(effects.splice(0)).toEqual(['publish', 'verify'])

      expect(yield* start(publish, fakeHost({ journal: journalOf(first.recorded) }))).toEqual(exit)
      expect(effects).toEqual([])
    }),
  )

  it.effect('fail the program with their tag and fields; defects are not journaled', () =>
    Effect.gen(function* () {
      const failing = Fractal.mission(
        { meta, id: 'factory/test', goal: 'Fail.', reportTo, input: Change },
        () =>
          Fractal.step(
            { id: 'check' },
            Effect.fail(new StepFailed({ step: 'check', reason: 'red' })),
          ),
      )
      expect(yield* start(failing, fakeHost())).toEqual({
        _tag: 'Failure',
        error: { _tag: 'StepFailed', step: 'check', reason: 'red' },
      })

      const buggy = Fractal.mission(
        { meta, id: 'factory/test', goal: 'Fail.', reportTo, input: Change },
        () => Fractal.step({ id: 'check' }, Effect.die(new Error('bug'))),
      )
      const host = fakeHost()
      const exit = yield* start(buggy, host)
      expect(exit).toMatchObject({ _tag: 'Failure', error: { _tag: 'Defect' } })
      expect(host.recorded).toEqual([])
      expect(host.started).toEqual(['check'])
    }),
  )

  it.effect('fail a program that declares an error class when the step declares it too', () =>
    Effect.gen(function* () {
      const refused = Fractal.mission(
        { meta, id: 'factory/test', goal: 'Refuse.', reportTo, input: Change, error: Refused },
        () =>
          Fractal.step(
            { id: 'verify', error: Refused },
            Effect.fail(new Refused({ reason: 'stale' })),
          ),
      )
      expect(yield* start(refused, fakeHost())).toEqual({
        _tag: 'Failure',
        error: { _tag: 'Refused', reason: 'stale' },
      })
      // Without `error` the step fails with the plain journaled shape, which is not the class.
      // @ts-expect-error a plain `{ _tag, reason }` is not a `Refused` instance
      const plain: Effect.Effect<void, Refused, ProgramServices> = Fractal.step(
        { id: 'verify' },
        Effect.fail(new Refused({ reason: 'stale' })),
      )
      expect(plain).toBeDefined()
    }),
  )
})

describe('step keys', () => {
  it.effect('repeat as id#n under Effect.retry and replay in that order', () =>
    Effect.gen(function* () {
      let runs = 0
      const verify = Fractal.mission(
        {
          meta,
          id: 'factory/test',
          goal: 'Verify.',
          reportTo,
          input: Change,
          output: Schema.Number,
        },
        () =>
          Fractal.step(
            { id: 'verify', success: Schema.Number },
            Effect.sync(() => ++runs),
          ).pipe(
            Effect.flatMap((n) =>
              n < 3
                ? Effect.fail(new StepFailed({ step: 'verify', reason: 'stale' }))
                : Effect.succeed(n),
            ),
            Effect.retry(Schedule.recurs(5)),
          ),
      )
      const first = fakeHost()
      expect(yield* start(verify, first)).toEqual({ _tag: 'Success', value: 3 })
      expect(first.recorded.map((entry) => [entry.key, entry.value])).toEqual([
        ['verify', 1],
        ['verify#2', 2],
        ['verify#3', 3],
      ])
      expect(yield* start(verify, fakeHost({ journal: journalOf(first.recorded) }))).toEqual({
        _tag: 'Success',
        value: 3,
      })
      expect(runs).toBe(3)
    }),
  )

  it.effect('refuse two concurrent steps with one id as a defect', () =>
    Effect.gen(function* () {
      const twice = Fractal.mission(
        { meta, id: 'factory/test', goal: 'Check twice.', reportTo, input: Change },
        () =>
          Effect.asVoid(
            Effect.all(
              [
                Fractal.step({ id: 'check' }, Effect.never),
                Fractal.step({ id: 'check' }, Effect.never),
              ],
              { concurrency: 'unbounded' },
            ),
          ),
      )
      const host = fakeHost()
      const exit = yield* start(twice, host)
      expect(exit).toMatchObject({ _tag: 'Failure', error: { _tag: 'Defect' } })
      expect(JSON.stringify(exit)).toContain('steps in concurrent branches share the id `check`')
      expect(host.recorded).toEqual([])
    }),
  )

  it.effect('refuse one id in two concurrent branches even when the uses do not overlap', () =>
    Effect.gen(function* () {
      // Live, the fast branch would take `x` and the slow one `x#2`; on replay `a` returns at once
      // and the branches would swap keys and values.
      const branches = Fractal.mission(
        { meta, id: 'factory/test', goal: 'Branch.', reportTo, input: Change },
        () =>
          Effect.asVoid(
            Effect.all(
              [
                Fractal.step({ id: 'a' }, Effect.yieldNow).pipe(
                  Effect.andThen(Fractal.step({ id: 'x' }, Effect.void)),
                ),
                Fractal.step({ id: 'x' }, Effect.void),
              ],
              { concurrency: 'unbounded' },
            ),
          ),
      )
      const exit = yield* start(branches, fakeHost())
      expect(exit).toMatchObject({ _tag: 'Failure', error: { _tag: 'Defect' } })
      expect(JSON.stringify(exit)).toContain('steps in concurrent branches share the id `x`')
    }),
  )
})

describe('asks, child runs and worker turns', () => {
  const Verdict = Schema.Struct({ verdict: Schema.Literals(['GO', 'NO-GO']) })
  const review = Fractal.mission(
    {
      meta,
      id: 'factory/test',
      goal: 'Review.',
      reportTo,
      input: Change,
      output: Schema.Array(Schema.String),
    },
    ({ change }) =>
      Effect.gen(function* () {
        const ack = yield* Fractal.ask({
          id: 'owner-ack',
          for: Fractal.person('person/owner'),
          title: 'Accept the change',
          request: { version: 1, type: 'decision' },
          default: 'reject',
        })
        const child = yield* Fractal.run('factory/publish', { change }, { id: 'publish' })
        const failed = yield* Fractal.run('factory/publish', { change }, { id: 'publish' }).pipe(
          Effect.catchTag('ChildFailed', (e) =>
            Effect.succeed(e instanceof ChildFailed ? `failed ${e.reason}` : 'plain'),
          ),
        )
        const verdict = yield* Fractal.step({ id: 'review', success: Verdict }).pipe(
          Effect.provide(
            Fractal.Worker.layer({
              mission: 'factory/step/review',
              input: { change },
              product: 'review/verdict',
            }),
          ),
        )
        return [
          ack.answer?.id ?? 'none',
          ack.resolved_by,
          child.run,
          String(failed),
          verdict.verdict,
        ]
      }),
  )
  const output = ['approve', 'person/owner', 'mission-run/child-1', 'failed cancelled', 'GO']

  it.effect('wait for the daemon, then replay from its journal', () =>
    Effect.gen(function* () {
      const requests: unknown[] = []
      const resolutions: Record<string, ProgramExit> = {
        'owner-ack': {
          _tag: 'Success',
          value: {
            answer: { type: 'decision', outcome: 'accept', id: 'approve' },
            resolved_by: 'person/owner',
          },
        },
        publish: { _tag: 'Success', value: { run: 'mission-run/child-1', status: 'completed' } },
        'publish#2': {
          _tag: 'Failure',
          error: {
            _tag: 'ChildFailed',
            run: 'mission-run/child-2',
            status: 'cancelled',
            reason: 'cancelled',
          },
        },
        review: {
          _tag: 'Success',
          value: {
            run: 'mission-run/child-3',
            status: 'completed',
            products: {
              'resource/mission-run/child-3/review/verdict': { verdict: 'GO', head: 'h1' },
            },
          },
        },
      }
      const host = fakeHost({
        ask: async (request) => {
          requests.push(request)
          return resolutions[request.key]!
        },
        child: async (request) => {
          requests.push(request)
          return resolutions[request.key]!
        },
      })
      expect(yield* start(review, host)).toEqual({ _tag: 'Success', value: output })
      expect(requests).toEqual([
        {
          key: 'owner-ack',
          for: 'person/owner',
          title: 'Accept the change',
          request: { version: 1, type: 'decision' },
          default: 'reject',
        },
        { key: 'publish', mission: 'factory/publish', input: { change: 'abc' } },
        { key: 'publish#2', mission: 'factory/publish', input: { change: 'abc' } },
        { key: 'review', mission: 'factory/step/review', input: { change: 'abc' } },
      ])
      expect(host.recorded).toEqual([])

      const replay = fakeHost({ journal: Object.entries(resolutions) })
      expect(yield* start(review, replay)).toEqual({ _tag: 'Success', value: output })
    }),
  )

  it.effect('returns execution evidence separately from a worker product, live and on replay', () =>
    Effect.gen(function* () {
      const execution = {
        id: 'claim/invocation',
        step_subject: 'step-run/child/review',
        attempt: 1,
        agent: 'agent/child/reviewer',
        incarnation_id: 'runtime-1',
        declared_model: 'requested-model',
        observed_models: ['actual-model'],
        fallback: null,
        started_at_unix_ms: 10,
        ended_at_unix_ms: 30,
        duration_ms: 20,
        status: 'completed',
        trace_id: null,
        exit_evidence: null,
        usage: null,
      }
      const worker = Fractal.mission(
        { meta, id: 'factory/test', goal: 'Read.', reportTo, input: Change,
          output: Schema.Struct({ value: Verdict, executions: Schema.optionalKey(Schema.Unknown) }) },
        () => Fractal.step({ id: 'read', success: Verdict, execution: true }).pipe(
          Effect.provide(Fractal.Worker.layer({ mission: 'worker', product: 'verdict' })),
        ),
      )
      const exit: ProgramExit = { _tag: 'Success', value: {
        run: 'mission-run/child', status: 'completed', executions: [execution],
        products: { 'resource/mission-run/child/verdict': { verdict: 'GO' } },
      } }
      const host = fakeHost({ child: async () => exit })
      const expected = { _tag: 'Success', value: {
        value: { verdict: 'GO' }, executions: [execution],
      } }
      expect(yield* start(worker, host)).toEqual(expected)
      expect(yield* start(worker, fakeHost({ journal: [['read', exit]] }))).toEqual(expected)
      const legacy: ProgramExit = { _tag: 'Success', value: {
        run: 'mission-run/child', status: 'completed',
        products: { 'resource/mission-run/child/verdict': { verdict: 'GO' } },
      } }
      expect(yield* start(worker, fakeHost({ journal: [['read', legacy]] }))).toEqual({
        _tag: 'Success', value: { value: { verdict: 'GO' } },
      })
    }),
  )

  it.effect('fail typed when the daemon withdraws an ask', () =>
    Effect.gen(function* () {
      const asking = Fractal.mission(
        { meta, id: 'factory/test', goal: 'Ask.', reportTo, input: Change, output: Schema.String },
        () =>
          Fractal.ask({
            id: 'ack',
            for: Fractal.person('person/owner'),
            title: 'Ack',
            request: null,
          }).pipe(
            Effect.as('answered'),
            Effect.catchTag('AskCancelled', (e) =>
              Effect.succeed(e instanceof AskCancelled ? e.ask : ''),
            ),
          ),
      )
      const journal = [
        ['ack', { _tag: 'Failure', error: { _tag: 'AskCancelled', ask: 'step-run/g/ask-1' } }],
      ] as const
      expect(yield* start(asking, fakeHost({ journal }))).toEqual({
        _tag: 'Success',
        value: 'step-run/g/ask-1',
      })
    }),
  )
})

describe('inputs and outputs', () => {
  const counted = Fractal.mission(
    {
      meta,
      id: 'factory/test',
      goal: 'Count.',
      reportTo,
      input: Schema.Struct({
        count: Schema.NumberFromString,
        reviewer: Schema.Literals(['llm', 'scripted']),
      }),
      output: Schema.Struct({ next: Schema.NumberFromString }),
    },
    ({ count }) => Effect.succeed({ next: count + 1 }),
  )

  it.effect('decode the payload and encode the success with the output schema', () =>
    Effect.gen(function* () {
      expect(
        yield* start(counted, fakeHost({ payload: { count: '41', reviewer: 'llm' } })),
      ).toEqual({
        _tag: 'Success',
        value: { next: '42' },
      })
    }),
  )

  it.effect('refuse a payload that does not decode', () =>
    Effect.gen(function* () {
      for (const payload of [
        { count: '1', reviewer: 'nobody' },
        { count: '1' },
        { count: '1', reviewer: 'llm', extra: 'x' },
      ])
        expect(yield* start(counted, fakeHost({ payload }))).toMatchObject({
          _tag: 'Failure',
          error: { _tag: 'InvalidInput' },
        })
    }),
  )

  it('refuse input fields that are optional or do not encode to a string', () => {
    const body = () => Effect.void
    expect(() =>
      Fractal.mission(
        {
          meta,
          id: 'factory/test',
          goal: 'g',
          reportTo,
          input: Schema.Struct({ change: Schema.optionalKey(Schema.String) }),
        },
        body,
      ),
    ).toThrow('Program input change must be required and encode to a string')
    expect(() =>
      Fractal.mission(
        {
          meta,
          id: 'factory/test',
          goal: 'g',
          reportTo,
          // @ts-expect-error a number does not encode to a native text input
          input: Schema.Struct({ count: Schema.Number }),
        },
        body,
      ),
    ).toThrow('Program input count must be required and encode to a string')
  })
})

describe('cancellation', () => {
  it.effect('interrupts the running step through the host signal and journals nothing', () =>
    Effect.gen(function* () {
      let interrupted = false
      const host = fakeHost()
      // The step's effect is running when the daemon cancels the attempt.
      const waiting = Fractal.mission(
        { meta, id: 'factory/test', goal: 'Wait.', reportTo, input: Change },
        () =>
          Fractal.step(
            { id: 'merge' },
            Effect.sync(() => host.cancel()).pipe(
              Effect.andThen(Effect.never),
              Effect.onInterrupt(() => Effect.sync(() => (interrupted = true))),
            ),
          ),
      )
      const exit = yield* start(waiting, host)
      expect(exit).toMatchObject({ _tag: 'Failure', error: { _tag: 'Interrupted' } })
      expect(interrupted).toBe(true)
      expect(host.started).toEqual(['merge'])
      expect(host.recorded).toEqual([])
    }),
  )
})

describe('commands', () => {
  it.effect('run through the host and return its result', () =>
    Effect.gen(function* () {
      const echo = Fractal.mission(
        { meta, id: 'factory/test', goal: 'Echo.', reportTo, input: Change, output: Schema.String },
        ({ change }) =>
          Fractal.step(
            { id: 'echo', success: Schema.String },
            Effect.map(
              Fractal.command(['echo', change], { priority: 'background' }),
              (out) => out.stdout,
            ),
          ),
      )
      expect(yield* start(echo, fakeHost())).toEqual({ _tag: 'Success', value: 'echo abc' })
    }),
  )
})
