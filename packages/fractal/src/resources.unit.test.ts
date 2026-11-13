import { describe, expect, it } from '@effect/vitest'
import { Effect, Schema } from 'effect'

import { Fractal, type ProgramExit, type ResourceRequest } from './effect.ts'
import { fakeHost, start } from './program-host.fixture.ts'

const membership = { membershipId: 'membership/check', authorityEpoch: 'epoch/1', generation: 'generation/1', coverage: 'logical' } as const
const meta = { url: 'file:///tree/check/mission.ts' }
const reportTo = { _tag: 'SubjectRef', kind: 'agent', url: 'file:///tree/agent.ts' } as const
const intent = { meta, id: 'checks', goal: 'Check source.', reportTo, input: Schema.Struct({ change: Schema.String }) }

// This is a protocol test, not a replacement host accountant. Native parity tests own fit,
// queue clocks, process containment, cancellation races and confirmed physical retirement.
describe('scoped native resource lanes', () => {
  it.effect('acquires and executes through the native boundary, then awaits release', () => Effect.gen(function* () {
    const operations: ResourceRequest[] = []
    const program = Fractal.mission(intent, () => Effect.scoped(Effect.gen(function* () {
      const lane = yield* Fractal.resources.lane('checks', { host: 'executor', admissionTimeout: '30m' })
      yield* lane.exec('check', ['tools/check'], { timeout: '2h', cwd: '/source' })
      yield* lane.run('child', 'checks/child', { change: 'abc' })
    })))
    const host = fakeHost({ resource: async (request) => {
      operations.push(request)
      switch (request.operation) {
        case 'resource.lane.acquire': return { _tag: 'Success', value: membership }
        case 'resource.lane.exec': return { _tag: 'Success', value: { code: 0, signal: null, stdout: '', stderr: '' } }
        case 'resource.lane.run': return { _tag: 'Success', value: { run: 'mission-run/child', status: 'completed' } }
        case 'resource.lane.release': return { _tag: 'Success', value: null }
      }
    } })
    const exit = yield* start(program, host)
    expect(exit._tag).toBe('Success')
    expect(operations.map((request) => request.operation)).toEqual([
      'resource.lane.acquire', 'resource.lane.exec', 'resource.lane.run', 'resource.lane.release',
    ])
    expect(operations[0]?.payload).toEqual({ name: 'checks', host: 'executor', coverage: 'logical', admissionTimeout: '1800000ms' })
    expect(operations[1]?.payload).toEqual({ membership, argv: ['tools/check'], options: { timeout: '7200000ms', cwd: '/source' } })
    expect(operations[2]?.payload).toEqual({ membership, mission: 'checks/child', input: { change: 'abc' } })
    expect(operations[3]?.payload).toEqual({ membership })
  }))

  it.effect('revalidates acquisition on replay instead of trusting a journaled grant', () => Effect.gen(function* () {
    const acquisitions: string[] = []
    const program = Fractal.mission(intent, () => Effect.scoped(Effect.gen(function* () {
      yield* Fractal.resources.lane('checks', { host: 'executor', coverage: 'enforced' })
    })))
    const host = fakeHost({
      journal: [['resource.lane/checks', { _tag: 'Success', value: membership }]],
      resource: async (request) => {
        acquisitions.push(request.operation)
        return { _tag: 'Failure', error: { _tag: 'ResourceError', code: 'stale-membership', message: 'The execution generation is retired.' } }
      },
    })
    expect(yield* start(program, host)).toEqual({ _tag: 'Failure', error: {
      _tag: 'ResourceError', code: 'stale-membership', message: 'The execution generation is retired.',
    } })
    expect(acquisitions).toEqual(['resource.lane.acquire'])
  }))

  it.effect('releases the exact membership after payload failure', () => Effect.gen(function* () {
    const operations: ResourceRequest[] = []
    const program = Fractal.mission(intent, () => Effect.scoped(Effect.gen(function* () {
      const lane = yield* Fractal.resources.lane('checks', { host: 'executor' })
      yield* lane.exec('check', ['false'])
    })))
    const host = fakeHost({ resource: async (request) => {
      operations.push(request)
      if (request.operation === 'resource.lane.acquire') return { _tag: 'Success', value: membership }
      if (request.operation === 'resource.lane.release') return { _tag: 'Success', value: null }
      return { _tag: 'Failure', error: { _tag: 'ResourceError', code: 'execution-refused', message: 'The generation was fenced.' } }
    } })
    expect((yield* start(program, host))._tag).toBe('Failure')
    expect(operations.at(-1)).toMatchObject({ operation: 'resource.lane.release', payload: { membership } })
  }))
})

it.effect('withdraws an interrupted queued acquisition through its host request signal', () => Effect.gen(function* () {
  let withdrawn = false
  const host = fakeHost({ resource: (_request, signal) => {
    const { promise, resolve } = Promise.withResolvers<ProgramExit>()
    signal?.addEventListener('abort', () => {
      withdrawn = true
      resolve({ _tag: 'Failure', error: { _tag: 'ResourceError', code: 'cancelled', message: 'Request withdrawn.' } })
    }, { once: true })
    host.cancel()
    return promise
  } })
  const program = Fractal.mission(intent, () => Effect.scoped(Effect.gen(function* () {
    yield* Fractal.resources.lane('checks', { host: 'executor' })
  })))
  expect(yield* start(program, host)).toMatchObject({ _tag: 'Failure', error: { _tag: 'Interrupted' } })
  expect(withdrawn).toBe(true)
  expect(host.recorded).toEqual([])
}))

it.effect('inherits native membership in child commands and keeps command keys stable across skipped steps', () => Effect.gen(function* () {
  const requests: ResourceRequest[] = []
  const program = Fractal.mission(intent, () => Effect.gen(function* () {
    yield* Fractal.step({ id: 'first' }, Fractal.command(['first']))
    yield* Fractal.step({ id: 'second' }, Fractal.command(['second']))
  }))
  const host = fakeHost({
    resourceMembership: membership,
    journal: [['first', { _tag: 'Success', value: undefined }]],
    spawn: async () => { throw new Error('inherited commands must not use detached host spawn') },
    resource: async (request) => {
      requests.push(request)
      return { _tag: 'Success', value: { code: 0, signal: null, stdout: '', stderr: '' } }
    },
  })
  expect((yield* start(program, host))._tag).toBe('Success')
  expect(requests).toEqual([{
    key: 'resource.command/second',
    operation: 'resource.lane.exec',
    payload: { membership, argv: ['second'], options: {} },
  }])
}))
