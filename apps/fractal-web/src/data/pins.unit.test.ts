import { FractalClient } from '@fractal/client'
import type { CollectionStream, CollectionStreamOptions, Snapshot } from '@fractal/client'
import { describe, expect, it, vi } from 'vitest'
import { followPins, pinsSelection, type PinsState } from './pins.ts'

const subject = pinsSelection('person/example').subject
const snapshot: Snapshot = { id: 'snapshot/test/7/proof', host_id: 'host/test', store_index: 7, projection_version: 'client-projection.v0', created_at: '2026-01-01T00:00:00Z' }
const identity = { id: 'action/pin', idempotency_key: 'pins-edit-000001', fence: { snapshot_id: snapshot.id, subject_revisions: {} } }
const setup = (absent = false, version = 2, createCode?: 'arrangement-exists' | 'arrangement-retired') => {
  let exists = !absent
  let frame: CollectionStreamOptions['onFrame'] | undefined
  let end: CollectionStreamOptions['onEnd']
  const actions: Record<string, unknown>[] = []
  const requests: URL[] = []
  const subscribed: unknown[][] = []
  const states: PinsState[] = []
  const row = (member: string, key: string) => ({ id: member, container: subject, member, position: { bucket: null, key }, revision: 'claim/place' })
  const client = new FractalClient({ baseUrl: 'http://test.invalid', fetchImpl: async (input, init) => {
    const url = new URL(String(input)); requests.push(url)
    let value: unknown
    if (url.pathname.endsWith('/capabilities')) value = { session_actor: 'person/example/session/test', limits: { max_page_items: 200 }, capabilities: [{ id: 'arrangements', version: 2, state: 'granted' }, { id: 'control.arrangements', state: 'granted' }] }
    else if (url.pathname.endsWith('/actions')) {
      const action = JSON.parse(String(init?.body)); actions.push(action)
      if (action.type === 'arrangement.edit') {
        exists = createCode !== 'arrangement-retired'
        if (createCode !== undefined) return Response.json({ api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', code: createCode, message: createCode, retryable: false }, { status: 409 })
      }
      value = { accepted: true }
    } else if (!exists) return Response.json({ api_version: 'st3.client.v0', error_version: 'st3.client.error.v0', code: 'not-found', message: 'absent', retryable: false }, { status: 404 })
    else if (url.pathname.endsWith('/memberships')) value = { kind: 'page', collection: 'ordered-memberships', filters: { person: 'person/example', subject }, items: [url.searchParams.has('cursor') ? row('mission/example', 'a1') : row('agent/example', 'a0')], page: { limit: 200, has_more: !url.searchParams.has('cursor'), next_cursor: url.searchParams.has('cursor') ? null : 'cursor-next-page' } }
    else value = { id: subject, owner: 'person/example', kind: 'arrangement', deleted: false, revision: 'claim/create', updated_at: snapshot.created_at, body: { version, name: { value: 'Pins', revision: 'claim/create' }, folders: {}, ...(version === 1 ? { placements: {} } : {}) } }
    return Response.json({ api_version: 'st3.client.v0', request_id: 'request/test', snapshot, value })
  } })
  const stream: CollectionStream = {
    subscribeArrangements: (...args) => { subscribed.push(args) }, subscribeOrderedMemberships: (...args) => { subscribed.push(args) },
    subscribe: () => {}, subscribeAgent: async () => {}, subscribeAgentsPublication: async () => {}, subscribeGlasses: () => {}, subscribeTerminal: () => {}, subscribeConversation: () => {}, unsubscribe: () => {}, close: () => {},
  }
  client.collectionStream = async (options) => { frame = options.onFrame; end = options.onEnd; return stream }
  return { client, states, actions, requests, subscribed, end: () => end?.(), retire: () => { exists = false }, invalidate: () => frame?.({ kind: 'changes', id: 'pins-members', collection: 'ordered-memberships', snapshot, membership: { container: subject, live_count: 2, changed_index: 9 }, upserts: [], removes: [], order: [], has_more: true }) }
}

describe('principal ordered Pins', () => {
  it('requires reopening after a stream-opening rejection even after refresh', async () => {
    const h = setup(), open = h.client.collectionStream, onState = (state: PinsState) => h.states.push(state)
    h.client.collectionStream = async () => { throw new Error('socket factory failed') }
    const pins = await followPins({ gateway: h.client, onState }); pins.refresh()
    await vi.waitFor(() => expect(h.requests.filter((url) => url.pathname.endsWith('/memberships'))).toHaveLength(2))
    expect(h.states.at(-1)?.phase).toBe('unavailable')
    await expect(pins.edit(identity, [{ op: 'remove', member: 'agent/example' }])).rejects.toThrow('must be synced'); pins.close()
    h.client.collectionStream = open; const reopened = await followPins({ gateway: h.client, onState }); expect(h.states.at(-1)?.phase).toBe('synced'); await reopened.edit(identity, [{ op: 'remove', member: 'agent/example' }]); reopened.close()
  })
  it('keeps a late post-edit HTTP read unavailable after socket end until reopened', async () => {
    const h = setup()
    const onState = (state: PinsState) => h.states.push(state)
    const pins = await followPins({ gateway: h.client, onState })
    const started = Promise.withResolvers<void>()
    const release = Promise.withResolvers<void>()
    const get = h.client.arrangementsGet.bind(h.client)
    h.client.arrangementsGet = async (...args) => { started.resolve(); await release.promise; return get(...args) }
    const operations = [{ op: 'remove' as const, member: 'agent/example' }]
    const editing = pins.edit(identity, operations)
    await started.promise
    h.end()
    expect(h.states.at(-1)?.phase).toBe('unavailable')
    release.resolve()
    await editing
    expect(h.states.at(-1)?.phase).toBe('unavailable')
    await expect(pins.edit(identity, operations)).rejects.toThrow('must be synced')
    pins.refresh()
    await expect(pins.edit(identity, operations)).rejects.toThrow('must be synced')
    pins.close()
    h.client.arrangementsGet = get
    const reopened = await followPins({ gateway: h.client, onState })
    expect(h.states.at(-1)?.phase).toBe('synced')
    await reopened.edit({ ...identity, id: 'action/reopened', idempotency_key: 'pins-reopened-0001' }, operations)
    reopened.close()
  })
  it('uses only the principal reserved identity', () => {
    expect(pinsSelection('person/example/session/device')).toEqual({ owner: 'person/example', subject })
    expect(() => pinsSelection('agent/example')).toThrow('person principal')
  })
  it('paginates and rereads on an outside-window invalidation', async () => {
    const h = setup(); const pins = await followPins({ gateway: h.client, onState: (state) => h.states.push(state) })
    expect(h.states.at(-1)?.detail).toBeUndefined()
    expect(h.states.at(-1)?.members.map((row) => row.member)).toEqual(['agent/example', 'mission/example'])
    expect(h.subscribed).toHaveLength(2)
    const before = h.requests.length; h.invalidate()
    await vi.waitFor(() => expect(h.requests.length).toBeGreaterThan(before + 2))
    await pins.edit(identity, [{ op: 'remove', member: 'agent/example' }])
    expect(h.actions).toMatchObject([{ type: 'arrangement.membership.edit', parameters: { subject, operations: [{ op: 'remove' }] } }])
    pins.close()
  })
  it('creates v2 only with explicit rollout approval and separate action identities', async () => {
    const h = setup(true); const pins = await followPins({ gateway: h.client, onState: () => {}, v2ReadersReady: true })
    await pins.edit(identity, [{ op: 'place', member: 'agent/example', bucket: null, key: 'a0' }], { ...identity, id: 'action/create', idempotency_key: 'pins-create-00001' })
    expect(h.actions.map((action) => action.type)).toEqual(['arrangement.edit', 'arrangement.membership.edit'])
    expect(h.actions[0]).toMatchObject({ parameters: { subject, version: 2, operations: [{ op: 'create', name: 'Pins' }] } })
    pins.close()
  })
  it('does not create without coordinated readers or migrate an existing v1 body', async () => {
    for (const h of [setup(true), setup(false, 1)]) {
      const pins = await followPins({ gateway: h.client, onState: () => {} })
      await expect(pins.edit(identity, [{ op: 'remove', member: 'agent/example' }])).rejects.toThrow()
      expect(h.actions).toEqual([]); pins.close()
    }
  })
  it('adopts a concurrent reserved create and refuses a retired identity', async () => {
    for (const code of ['arrangement-exists', 'arrangement-retired'] as const) {
      const h = setup(true, 2, code)
      const pins = await followPins({ gateway: h.client, onState: () => {}, v2ReadersReady: true })
      const edit = pins.edit(identity, [{ op: 'place', member: 'agent/example', bucket: null, key: 'a0' }], { ...identity, id: 'action/create', idempotency_key: 'pins-create-00001' })
      if (code === 'arrangement-exists') {
        await edit
        expect(h.actions).toHaveLength(2)
      } else {
        await expect(edit).rejects.toBeDefined()
        expect(h.actions).toHaveLength(1)
        await expect(pins.edit(identity, [{ op: 'remove', member: 'agent/example' }])).rejects.toThrow()
      }
      pins.close()
    }
  })
  it('refuses folder buckets and permanent retirement without changing Sidebar', async () => {
    const h = setup(); const pins = await followPins({ gateway: h.client, onState: (state) => h.states.push(state) })
    await expect(pins.edit(identity, [{ op: 'place', member: 'agent/example', bucket: 'folder/example', key: 'a0' }])).rejects.toThrow()
    h.retire(); pins.refresh()
    await vi.waitFor(() => expect(h.states.at(-1)?.detail).toContain('Restoring removed Pins'))
    await expect(pins.edit(identity, [{ op: 'remove', member: 'agent/example' }])).rejects.toThrow()
    expect(h.actions).toEqual([]); pins.close()
  })
})
