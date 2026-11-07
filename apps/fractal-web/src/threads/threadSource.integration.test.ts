import { FractalClient } from '@fractal/client'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import { expect, it, vi } from 'vitest'
import fixture from '../data/subjectReadPort.gateway.fixtures.json'
import { gatewayThreads } from './threadSource.ts'

const base = fixture.capabilities
const id = 'thread/thread-one'
const agent = 'agent/workshop/atlas'
const summary = {
  id, agent_id: agent, title: 'PR #42', title_revision: 'claim/title', revision: 'claim/latest', state: 'open',
}

it('issues no thread reads or edits when the daemon has not advertised the feature', async () => {
  const fetchImpl = vi.fn(async (_input: RequestInfo | URL) => Response.json(base))
  const client = new FractalClient({ baseUrl: 'http://example.test', fetchImpl })
  const registry = AtomRegistry.make()
  const source = gatewayThreads(client, registry)
  const list = source.list({ agent_id: agent })
  const stop = registry.mount(list)
  try {
    await vi.waitFor(() => expect(fetchImpl).toHaveBeenCalledTimes(1))
    expect(registry.get(list)).toEqual({ _tag: 'Waiting' })
    expect(registry.get(source.access)).toMatchObject({ read: false, actions: new Set() })
    expect(String(fetchImpl.mock.calls[0]?.[0])).toContain('/capabilities')
  } finally {
    stop()
    registry.dispose()
  }
})

it('reads the exact agent, paginates by opaque cursor and sends the observed metadata fence', async () => {
  const requests: Array<{ readonly url: URL; readonly body: unknown }> = []
  const fetchImpl = async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = new URL(String(input))
    const body: unknown = typeof init?.body === 'string' ? JSON.parse(init.body) : undefined
    requests.push({ url, body })
    if (url.pathname.endsWith('/capabilities')) return Response.json({
      ...base,
      value: { ...base.value, transport: 'local-web', capabilities: [
        ...base.value.capabilities,
        { id: 'threads-v1', version: 1, state: 'granted' },
        { id: 'thread.close', version: 0, state: 'granted' },
      ] },
    })
    if (url.pathname.endsWith('/threads')) return Response.json({ ...base, value: { items: [summary], next_cursor: 'older/+chunk' } })
    if (url.pathname.endsWith('/timeline')) return Response.json({ ...base, value: { items: [], next_cursor: null } })
    if (url.pathname.endsWith('/actions')) return Response.json({
      ...base,
      value: {
        kind: 'action-result', action_id: (body as { id: string }).id, operation_id: 'operation/one', status: 'completed',
        affected_ids: [id], metadata_revision: 'claim/closed', snapshot_id: base.snapshot.id,
      },
    })
    return Response.json({ ...base, value: summary })
  }
  const client = new FractalClient({ baseUrl: 'http://example.test', fetchImpl })
  const registry = AtomRegistry.make()
  const source = gatewayThreads(client, registry)
  const list = source.list({ agent_id: agent, cursor: 'older/+chunk' })
  const detail = source.detail(id)
  const timeline = source.timeline(id, 'timeline/older')
  const stops = [registry.mount(list), registry.mount(detail), registry.mount(timeline)]
  try {
    await vi.waitFor(() => {
      expect(registry.get(list)._tag).toBe('Observed')
      expect(registry.get(detail)._tag).toBe('Observed')
      expect(registry.get(timeline)._tag).toBe('Observed')
    })
    const observation = registry.get(detail)
    if (observation._tag !== 'Observed') throw new Error('expected thread observation')
    expect(await source.changeState(observation.value, 'closed')).toBeUndefined()
    expect(requests.find(({ url }) => url.pathname.endsWith('/threads'))?.url.searchParams.get('agent_id')).toBe(agent)
    expect(requests.find(({ url }) => url.pathname.endsWith('/threads'))?.url.searchParams.get('cursor')).toBe('older/+chunk')
    expect(requests.find(({ url }) => url.pathname.endsWith('/timeline'))?.url.pathname).toBe('/v1/client/threads/thread-one/timeline')
    expect(requests.find(({ url }) => url.pathname.endsWith('/timeline'))?.url.searchParams.get('cursor')).toBe('timeline/older')
    expect(requests.find(({ url }) => url.pathname.endsWith('/actions'))?.body).toMatchObject({
      type: 'thread.close',
      fence: { snapshot_id: base.snapshot.id, subject_revisions: { [id]: summary.revision } },
      parameters: { target_id: id },
    })
  } finally {
    for (const stop of stops) stop()
    registry.dispose()
  }
})

it('starts with the exact ask episode and retries the same first message after an uncertain response', async () => {
  const bodies: Array<Record<string, unknown>> = []
  const fetchImpl = async (input: RequestInfo | URL, init?: RequestInit) => {
    if (String(input).endsWith('/capabilities')) return Response.json({
      ...base,
      value: { ...base.value, transport: 'local-web', capabilities: [
        ...base.value.capabilities,
        { id: 'threads-v1', version: 1, state: 'granted' },
        { id: 'thread.start', version: 0, state: 'granted' },
      ] },
    })
    const body = JSON.parse(String(init?.body))
    bodies.push(body)
    if (bodies.length === 1) throw new TypeError('connection interrupted')
    if (bodies.length === 3) return Response.json({
      api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', request_id: 'request/refused',
      code: 'forbidden', message: 'Thread key is temporarily unavailable', retryable: false, details: {},
    }, { status: 403 })
    return Response.json({ ...base, value: {
      kind: 'action-result', action_id: bodies.length === 2 ? 'action/unrelated' : body.id, operation_id: 'operation/start', status: 'completed',
      affected_ids: [body.parameters.target_id, 'message/first'], snapshot_id: base.snapshot.id,
    } })
  }
  const client = new FractalClient({ baseUrl: 'http://example.test', fetchImpl })
  const registry = AtomRegistry.make()
  const source = gatewayThreads(client, registry)
  const stop = registry.mount(source.access)
  const input = { agent_id: agent, origin_ask: 'step-run/review', origin_ask_episode: 'claim/ask', title: 'PR #42', content: 'Can we run the worker test?' }
  try {
    await vi.waitFor(() => expect(registry.get(source.access).actions.has('thread.start')).toBe(true))
    expect((await source.start('ask/review', input))._tag).toBe('Refused')
    expect((await source.start('ask/review', { ...input, content: 'Different message' }))._tag).toBe('Refused')
    expect((await source.start('ask/review', input))._tag).toBe('Refused')
    expect((await source.start('ask/review', input))._tag).toBe('Refused')
    expect((await source.start('ask/review', input))._tag).toBe('Sent')
    expect(bodies).toHaveLength(4)
    for (const retry of bodies.slice(1)) expect(retry).toEqual(bodies[0])
    expect(bodies[1]).toMatchObject({
      type: 'thread.start',
      parameters: { ...input, target_id: expect.stringMatching(/^thread\//) },
    })
    expect(bodies[1]?.parameters).not.toHaveProperty('in_reply_to')
  } finally {
    stop()
    registry.dispose()
  }
})

it('offers metadata edits but omits the unsigned composer on paired web transport', async () => {
  const client = new FractalClient({ baseUrl: 'http://example.test', fetchImpl: async () => Response.json({
    ...base,
    value: { ...base.value, capabilities: [
      ...base.value.capabilities,
      { id: 'threads-v1', version: 1, state: 'granted' },
      ...['thread.start', 'thread.reply', 'thread.close'].map(id => ({ id, version: 0, state: 'granted' })),
    ] },
  }) })
  const registry = AtomRegistry.make()
  const source = gatewayThreads(client, registry)
  const stop = registry.mount(source.access)
  try {
    await vi.waitFor(() => expect(registry.get(source.access)).toMatchObject({
      read: true,
      actions: new Set(['thread.close']),
    }))
  } finally {
    stop()
    registry.dispose()
  }
})
