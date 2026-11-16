import { describe, expect, it } from '@effect/vitest'
import { Effect, Schema } from 'effect'

import { Fractal, type ProgramExit, type ResourceRequest } from './effect.ts'
import { fakeHost, journalOf, start } from './program-host.fixture.ts'

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

  it.effect('resumes the second activity membership after the completed first activity is skipped', () => Effect.gen(function* () {
    const initialRequests: ResourceRequest[] = []
    const program = Fractal.mission(intent, () => Effect.gen(function* () {
      for (const id of ['first', 'second']) {
        yield* Fractal.step({ id }, Effect.scoped(Fractal.resources.lane('checks', { host: 'executor' })))
      }
    }))
    const initial = fakeHost({ resource: (request, signal) => {
      initialRequests.push(request)
      if (request.key === 'second/resource.lane/checks') {
        const { promise, resolve } = Promise.withResolvers<ProgramExit>()
        signal?.addEventListener('abort', () => resolve({
          _tag: 'Failure', error: { _tag: 'ResourceError', code: 'cancelled', message: 'Request withdrawn.' },
        }), { once: true })
        initial.cancel()
        return promise
      }
      return Promise.resolve({ _tag: 'Success', value: request.operation === 'resource.lane.acquire' ? membership : null })
    } })
    expect(yield* start(program, initial)).toMatchObject({ _tag: 'Failure', error: { _tag: 'Interrupted' } })
    expect(initial.recorded.map((entry) => entry.key)).toEqual(['first'])
    const replayRequests: ResourceRequest[] = []
    const replay = fakeHost({
      journal: journalOf(initial.recorded),
      resource: async (request) => {
        replayRequests.push(request)
        if (request.key === 'first/resource.lane/checks') {
          return { _tag: 'Failure', error: { _tag: 'ResourceError', code: 'retired', message: 'The first activity retired.' } }
        }
        return { _tag: 'Success', value: request.operation === 'resource.lane.acquire' ? membership : null }
      },
    })
    expect((yield* start(program, replay))._tag).toBe('Success')
    expect(initialRequests.filter((request) => request.operation === 'resource.lane.acquire').map((request) => request.key)).toEqual([
      'first/resource.lane/checks', 'second/resource.lane/checks',
    ])
    expect(replayRequests.map((request) => request.key)).toEqual([
      'second/resource.lane/checks', 'second/resource.lane/checks/release',
    ])
    expect(replay.started).toEqual(['second'])
  }))

  it.effect('lets concurrent durable activities contend for the same physical lane at the host boundary', () => Effect.gen(function* () {
    const program = Fractal.mission(intent, () => Effect.all(
      ['first', 'second'].map((id) => Fractal.step({ id }, Effect.scoped(Effect.gen(function* () {
        const lane = yield* Fractal.resources.lane('checks', { host: 'executor', maxMembers: 1 })
        yield* lane.exec('check', ['check'])
      })))),
      { concurrency: 'unbounded' },
    ).pipe(Effect.asVoid))
    const attemptKeys: string[][] = []
    for (let attempt = 0; attempt < 2; attempt++) {
      const requests: ResourceRequest[] = []
      const bothRequested = Promise.withResolvers<void>()
      const released = Promise.withResolvers<void>()
      let acquisitions = 0
      let occupant: string | undefined
      const host = fakeHost({ resource: async (request) => {
        requests.push(request)
        switch (request.operation) {
          case 'resource.lane.acquire': {
            acquisitions++
            if (acquisitions === 2) {
              bothRequested.resolve()
              await released.promise
            }
            expect(occupant).toBeUndefined()
            occupant = request.key
            return { _tag: 'Success', value: { ...membership, membershipId: request.key } }
          }
          case 'resource.lane.exec':
            await bothRequested.promise
            return { _tag: 'Success', value: { code: 0, signal: null, stdout: '', stderr: '' } }
          case 'resource.lane.release':
            expect(request.payload).toEqual({ membership: { ...membership, membershipId: occupant } })
            occupant = undefined
            released.resolve()
            return { _tag: 'Success', value: null }
          default: throw new Error('unexpected child run')
        }
      } })
      expect((yield* start(program, host))._tag).toBe('Success')
      const acquisitionsMade = requests.filter((request) => request.operation === 'resource.lane.acquire')
      expect(acquisitionsMade.map((request) => request.payload)).toEqual([
        { name: 'checks', host: 'executor', maxMembers: 1, coverage: 'logical' },
        { name: 'checks', host: 'executor', maxMembers: 1, coverage: 'logical' },
      ])
      attemptKeys.push(acquisitionsMade.map((request) => request.key).sort())
      expect(occupant).toBeUndefined()
    }
    expect(attemptKeys).toEqual([
      ['first/resource.lane/checks', 'second/resource.lane/checks'],
      ['first/resource.lane/checks', 'second/resource.lane/checks'],
    ])
  }))

  it.effect('preserves successive acquisitions within one durable activity', () => Effect.gen(function* () {
    const keys: string[] = []
    const program = Fractal.mission(intent, () => Fractal.step({ id: 'checks' }, Effect.gen(function* () {
      yield* Effect.scoped(Fractal.resources.lane('checks', { host: 'executor' }))
      yield* Effect.scoped(Fractal.resources.lane('checks', { host: 'executor' }))
    })))
    const host = fakeHost({ resource: async (request) => {
      keys.push(request.key)
      return { _tag: 'Success', value: request.operation === 'resource.lane.acquire' ? membership : null }
    } })
    expect((yield* start(program, host))._tag).toBe('Success')
    expect(keys).toEqual([
      'checks/resource.lane/checks', 'checks/resource.lane/checks/release',
      'checks/resource.lane/checks#2', 'checks/resource.lane/checks#2/release',
    ])
  }))

  it.effect('still rejects duplicate lane identities across fibers inside one durable activity', () => Effect.gen(function* () {
    const program = Fractal.mission(intent, () => Fractal.step({ id: 'checks' }, Effect.all([
      Effect.scoped(Fractal.resources.lane('checks', { host: 'executor' })),
      Effect.scoped(Fractal.resources.lane('checks', { host: 'executor' })),
    ], { concurrency: 'unbounded' }).pipe(Effect.asVoid)))
    const host = fakeHost({ resource: async (request) => ({
      _tag: 'Success', value: request.operation === 'resource.lane.acquire' ? membership : null,
    }) })
    const exit = yield* start(program, host)
    expect(exit).toMatchObject({ _tag: 'Failure', error: { _tag: 'Defect' } })
    expect(JSON.stringify(exit)).toContain('steps in concurrent branches share the id `checks/resource.lane/checks`')
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

it.effect('rejects cross-fiber inherited command reuse in either arrival order, including replay', () => Effect.gen(function* () {
  for (const scope of ['program', 'checks']) {
    let reversed = false
    const program = Fractal.mission(intent, () => Effect.gen(function* () {
      yield* Fractal.step({ id: 'completed' }, Effect.void)
      const { promise, resolve } = Promise.withResolvers<void>()
      const branches = Effect.asVoid(Effect.all(
        ['first', 'second'].map((id) => Effect.gen(function* () {
          // The losing branch arrives only after the other command has completed. Ownership
          // must survive completion, not merely prevent overlapping native requests.
          if (id === (reversed ? 'first' : 'second')) yield* Effect.promise(() => promise)
          yield* Fractal.command([id])
          resolve()
        })),
        { concurrency: 'unbounded' },
      ))
      yield* scope === 'program' ? branches : Fractal.step({ id: scope }, branches)
    }))
    const liveRequests: ResourceRequest[] = []
    const live = fakeHost({
      resourceMembership: membership,
      resource: async (request) => {
        liveRequests.push(request)
        return { _tag: 'Success', value: { code: 0, signal: null, stdout: '', stderr: '' } }
      },
    })
    const liveExit = yield* start(program, live)
    expect(liveExit).toMatchObject({ _tag: 'Failure', error: { _tag: 'Defect' } })
    expect(JSON.stringify(liveExit)).toContain(`steps in concurrent branches share the id \`resource.command/${scope}\``)
    expect(liveRequests).toEqual([{
      key: `resource.command/${scope}`, operation: 'resource.lane.exec',
      payload: { membership, argv: ['first'], options: {} },
    }])
    reversed = true
    const replayRequests: ResourceRequest[] = []
    const replay = fakeHost({
      resourceMembership: membership,
      journal: journalOf(live.recorded),
      resource: async (request) => {
        replayRequests.push(request)
        return { _tag: 'Success', value: { code: 0, signal: null, stdout: '', stderr: '' } }
      },
    })
    const replayExit = yield* start(program, replay)
    expect(replayExit).toMatchObject({ _tag: 'Failure', error: { _tag: 'Defect' } })
    expect(JSON.stringify(replayExit)).toContain(`steps in concurrent branches share the id \`resource.command/${scope}\``)
    expect(replayRequests).toEqual([{
      key: `resource.command/${scope}`, operation: 'resource.lane.exec',
      payload: { membership, argv: ['second'], options: {} },
    }])
    expect(replay.started).not.toContain('completed')
  }
}))

it.effect('keeps same-fiber inherited command occurrence keys stable when an unfinished activity replays', () => Effect.gen(function* () {
  const program = Fractal.mission(intent, () => Fractal.step({ id: 'checks' }, Effect.gen(function* () {
    yield* Fractal.command(['first'])
    yield* Fractal.command(['second'], { cwd: '/source', env: { CHECK: 'yes' } })
    yield* Fractal.command(['first'])
  })))
  const attempts: ResourceRequest[][] = []
  for (let attempt = 0; attempt < 2; attempt++) {
    const requests: ResourceRequest[] = []
    const host = fakeHost({
      resourceMembership: membership,
      // Native commands may have completed without the enclosing activity being journaled.
      resource: async (request) => {
        requests.push(request)
        return { _tag: 'Success', value: { code: 0, signal: null, stdout: '', stderr: '' } }
      },
    })
    expect((yield* start(program, host))._tag).toBe('Success')
    attempts.push(requests)
  }
  expect(attempts[0]).toEqual([
    { key: 'resource.command/checks', operation: 'resource.lane.exec', payload: { membership, argv: ['first'], options: {} } },
    { key: 'resource.command/checks#2', operation: 'resource.lane.exec', payload: { membership, argv: ['second'], options: { cwd: '/source', env: { CHECK: 'yes' } } } },
    { key: 'resource.command/checks#3', operation: 'resource.lane.exec', payload: { membership, argv: ['first'], options: {} } },
  ])
  expect(attempts[1]).toEqual(attempts[0])
}))

it.effect('allows concurrent inherited commands in distinct durable activities', () => Effect.gen(function* () {
  const requests: ResourceRequest[] = []
  const program = Fractal.mission(intent, () => Effect.asVoid(Effect.all(
    ['first', 'second'].map((id) => Fractal.step({ id }, Fractal.command([id]))),
    { concurrency: 'unbounded' },
  )))
  const host = fakeHost({
    resourceMembership: membership,
    resource: async (request) => {
      requests.push(request)
      return { _tag: 'Success', value: { code: 0, signal: null, stdout: '', stderr: '' } }
    },
  })
  expect((yield* start(program, host))._tag).toBe('Success')
  expect(requests).toHaveLength(2)
  for (const id of ['first', 'second'])
    expect(requests).toContainEqual({
      key: `resource.command/${id}`, operation: 'resource.lane.exec',
      payload: { membership, argv: [id], options: {} },
    })
}))

it.effect('preserves concurrent non-lane commands through ordinary host spawn', () => Effect.gen(function* () {
  const spawned: string[][] = []
  const program = Fractal.mission(intent, () => Effect.asVoid(Effect.all(
    [Fractal.command(['first']), Fractal.command(['second'])],
    { concurrency: 'unbounded' },
  )))
  const host = fakeHost({
    spawn: async (argv) => {
      spawned.push([...argv])
      return { code: 0, signal: null, stdout: '', stderr: '' }
    },
  })
  expect((yield* start(program, host))._tag).toBe('Success')
  expect(spawned).toHaveLength(2)
  expect(spawned).toContainEqual(['first'])
  expect(spawned).toContainEqual(['second'])
}))
