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
  const list = source.list(agent)
  const stop = registry.mount(list)
  try {
    await vi.waitFor(() => expect(fetchImpl).toHaveBeenCalledTimes(1))
    expect(registry.get(list)).toEqual({ _tag: 'Waiting' })
    expect(registry.get(source.access)).toEqual({ read: false, edit: false })
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
      value: { ...base.value, capabilities: [
        ...base.value.capabilities,
        { id: 'threads-v1', version: 1, state: 'granted' },
        { id: 'control.messages', version: 1, state: 'granted' },
      ] },
    })
    if (url.pathname.endsWith('/threads')) return Response.json({ ...base, value: { items: [summary], next_cursor: 'older/+chunk' } })
    if (url.pathname.endsWith('/timeline')) return Response.json({ ...base, value: { items: [], next_cursor: null } })
    if (url.pathname.endsWith('/actions')) return Response.json({
      ...base,
      value: {
        kind: 'action-result', action_id: 'action/thread-close', operation_id: 'operation/one', status: 'completed',
        affected_ids: [id], metadata_revision: 'claim/closed', snapshot_id: base.snapshot.id,
      },
    })
    return Response.json({ ...base, value: summary })
  }
  const client = new FractalClient({ baseUrl: 'http://example.test', fetchImpl })
  const registry = AtomRegistry.make()
  const source = gatewayThreads(client, registry)
  const list = source.list(agent, 'older/+chunk')
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
