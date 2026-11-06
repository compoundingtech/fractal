import { FractalClient } from '@fractal/client'
import * as Native from '@fractal/client/schema/read'
import { Effect, Stream } from 'effect'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { describe, expect, it, vi } from 'vitest'
import recording from './subjectReadPort.gateway.fixtures.json' with { type: 'json' }
import { relatedRunsFeed } from './relatedRuns.ts'
import { fixtureSubjectReads, gatewaySubjectReads, nativeSubjectFixture, subjectReaderFromAtom } from './subjectReadPort.ts'

const wire = {
  kind: 'agent-related-runs', agent_id: 'agent/example', as_of: 1, missions_cut: 1, work_cut: 1,
  total_rows: 1, truncated: false,
  rows: [{ id: 'run/example', mission_id: 'mission/example', completed_at: null, current_step: null,
    deadline_at: null, eta: null, folded: false, late: false, progress: { done: 1, total: 3 },
    status: 'running', status_chip: 'running', urgency_rank: 2, waiting_on_you: false, waits_on: [] }],
}
const envelope = (asOf = 1) => Response.json({
  ...recording.cases.session.payload, value: { ...wire, as_of: asOf },
})
const setup = (respond: () => Response | Promise<Response>) => {
  const registry = AtomRegistry.make()
  const epoch = Atom.keepAlive(Atom.make(0))
  const interest = Atom.make((get) => { get.addFinalizer(() => { released++ }); return true })
  let released = 0
  let requests = 0
  const options = { baseUrl: 'http://gateway.invalid', fetchImpl: async (input: RequestInfo | URL) => {
    if (String(input).endsWith('/capabilities')) return Response.json(recording.capabilities)
    requests++
    return respond()
  } }
  const feed = relatedRunsFeed({ options, discovery: new FractalClient(options), registry, epoch,
    retain: get => { get(interest) } })
  const reader = gatewaySubjectReads({ options, registry,
    relatedRuns: subjectReaderFromAtom({ feed, registry }) }).relatedRuns
  const atom = reader.feed(wire.agent_id)
  return { registry, epoch, reader, atom, requests: () => requests, released: () => released }
}

describe('related runs demand and pushed invalidation', () => {
  it('reads once while idle, coalesces frame invalidations, and releases retained frame demand', async () => {
    let asOf = 1
    const s = setup(() => envelope(asOf))
    expect(s.requests()).toBe(0)
    const release = s.registry.mount(s.atom)
    try {
      await vi.waitFor(() => expect(s.registry.get(s.atom)).toMatchObject({ _tag: 'Observed', freshness: 'live' }))
      expect(s.requests()).toBe(1)
      vi.useFakeTimers()
      await vi.advanceTimersByTimeAsync(60_000)
      expect(s.requests()).toBe(1)
      vi.useRealTimers()
      asOf = 2
      Atom.batch(() => {
        s.registry.set(s.epoch, 1)
        s.registry.set(s.epoch, 2)
        s.registry.set(s.epoch, 3)
      })
      await vi.waitFor(() => expect(s.registry.get(s.atom)).toMatchObject({ value: { as_of: 2 }, freshness: 'live' }))
      expect(s.requests()).toBe(2)
    } finally { vi.useRealTimers(); release(); s.registry.dispose() }
    expect(s.released()).toBeGreaterThan(0)
  })

  it('keeps last-good rows during refresh and exposes a failed refresh', async () => {
    let answer = () => Promise.resolve(envelope())
    const s = setup(() => answer())
    const release = s.registry.mount(s.atom)
    try {
      await vi.waitFor(() => expect(s.registry.get(s.atom)).toMatchObject({ freshness: 'live' }))
      const pending = Promise.withResolvers<Response>()
      answer = () => pending.promise
      s.registry.set(s.epoch, 1)
      await vi.waitFor(() => expect(s.requests()).toBe(2))
      expect(s.registry.get(s.atom)).toMatchObject({ freshness: 'stale', value: { rows: [{ id: 'run/example' }] } })
      pending.resolve(Response.json({ error: 'failed' }, { status: 500 }))
      await vi.waitFor(() => expect(s.registry.get(s.atom)).toMatchObject({ freshness: 'stale', error: { reason: 'failed' } }))
      expect(await Effect.runPromise(Effect.flip(s.reader.read(wire.agent_id)))).toMatchObject({ reason: 'failed' })
    } finally { release(); s.registry.dispose() }
  })

  it('discards a previous connection response even when its transport ignores abort', async () => {
    const pending: Array<(response: Response) => void> = []
    const s = setup(() => {
      const request = Promise.withResolvers<Response>()
      pending.push(request.resolve)
      return request.promise
    })
    const release = s.registry.mount(s.atom)
    try {
      await vi.waitFor(() => expect(pending).toHaveLength(1))
      s.registry.set(s.epoch, 1)
      await vi.waitFor(() => expect(pending).toHaveLength(2))
      pending[1]!(envelope(2))
      await vi.waitFor(() => expect(s.registry.get(s.atom)).toMatchObject({ value: { as_of: 2 } }))
      pending[0]!(envelope(1))
      const drained = Promise.withResolvers<void>()
      setImmediate(drained.resolve)
      await drained.promise
      expect(s.registry.get(s.atom)).toMatchObject({ freshness: 'live', value: { as_of: 2 } })
    } finally { release(); s.registry.dispose() }
  })

  it.each([404, 501])('shows unsupported capability (%s), never empty successful rows', async status => {
    const s = setup(() => Response.json({ api_version: 'st3.client.v0', error_version: 'st3.client.error.v0',
      code: 'unsupported-capability', message: 'Related runs are unsupported', retryable: false,
      request_id: 'request/unsupported', details: {} }, { status }))
    try {
      expect(await Effect.runPromise(Effect.flip(s.reader.read(wire.agent_id)))).toMatchObject({ reason: 'unsupported' })
      expect(s.registry.get(s.atom)).toMatchObject({ _tag: 'Unavailable', reason: 'unsupported' })
    } finally { s.registry.dispose() }
  })

  it('decodes generated Option fields and makes fixture overrides available through every reader seam', async () => {
    const state = nativeSubjectFixture({ schema: Native.AgentRelatedRuns, wire })
    const reads = fixtureSubjectReads({ fixtures: { relatedRuns: { [wire.agent_id]: state, 'agent/unsupported': state } },
      overrides: { relatedRuns: { 'agent/unsupported': { _tag: 'Unavailable', reason: 'unsupported', detail: 'Not served' } } } })
    const registry = AtomRegistry.make()
    try {
      const value = await Effect.runPromise(reads.relatedRuns.read(wire.agent_id))
      expect(value.rows[0]?.eta).toMatchObject({ _tag: 'None' })
      expect(registry.get(reads.relatedRuns.feed(wire.agent_id))).toEqual(state)
      expect(await Effect.runPromise(Stream.runHead(reads.relatedRuns.changes(wire.agent_id)))).toMatchObject({ _tag: 'Some', value: state })
      expect(await Effect.runPromise(Effect.flip(reads.relatedRuns.read('agent/unsupported')))).toMatchObject({ reason: 'unsupported' })
      expect(registry.get(reads.relatedRuns.feed('agent/missing'))).toMatchObject({ _tag: 'Unavailable', reason: 'unsupported' })
    } finally { registry.dispose() }
  })
})
