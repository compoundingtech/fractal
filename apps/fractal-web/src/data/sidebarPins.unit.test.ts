import { FractalClient, PINS_ARRANGEMENT_UUID, type CollectionStream } from '@fractal/client'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import * as Atom from 'effect/reactivity/Atom'
import { describe, expect, it, vi } from 'vitest'
import { pinsEditable, planPinToggle, sidebarPins } from './sidebarPins.ts'
import type { PinsState } from './pins.ts'

const state: PinsState = { phase: 'synced', editable: true, members: ['a1', 'a0'].map((key) => ({
  id: `agent/${key}`, container: `arrangement/person/example/${PINS_ARRANGEMENT_UUID}`, member: `agent/${key}`,
  revision: 'claim/place', position: { bucket: null, key },
})) }
describe('sidebar Pins binding', () => {
  it('aborts an initializing read and closes its socket when the source switches', async () => {
    const started = Promise.withResolvers<void>(), closed = vi.fn()
    let signal: AbortSignal | undefined
    const client = new FractalClient({ baseUrl: 'http://gateway.test', fetchImpl: async (input, init) => {
      if (String(input).endsWith('/capabilities')) return Response.json({ api_version: 'st3.client.v0', request_id: 'request/test', snapshot: { id: 'snapshot/test/7/proof', host_id: 'host/test', store_index: 7, projection_version: 'client-projection.v0', created_at: '2026-01-01T00:00:00Z' }, value: {
        session_actor: 'person/example', limits: { max_page_items: 200 },
        capabilities: [{ id: 'arrangements', version: 2, state: 'granted' }, { id: 'control.arrangements', state: 'granted' }],
      } })
      signal = init?.signal ?? undefined; started.resolve()
      return new Promise<Response>((_resolve, reject) => signal?.addEventListener('abort', () => reject(signal?.reason), { once: true }))
    } })
    vi.spyOn(client, 'collectionStream').mockResolvedValue({ close: closed, subscribeArrangements: vi.fn() } as unknown as CollectionStream)
    const registry = AtomRegistry.make(), source = Atom.make<FractalClient | undefined>(client)
    const release = registry.mount(Atom.make(get => get(sidebarPins(get(source)))))
    try {
      await started.promise
      registry.set(source, undefined)
      await vi.waitFor(() => expect(closed).toHaveBeenCalled())
      expect(signal?.aborted).toBe(true)
    } finally { release(); registry.dispose() }
  })
  it('appends after the last root key and removes existing membership', () => {
    expect(planPinToggle(state, 'mission/example')).toEqual([{ op: 'place', member: 'mission/example', bucket: null, key: 'a2' }])
    expect(planPinToggle(state, 'agent/a0')).toEqual([{ op: 'remove', member: 'agent/a0' }])
    expect(planPinToggle({ ...state, members: [] }, 'agent/first')).toEqual([{ op: 'place', member: 'agent/first', bucket: null, key: 'a0' }])
  })
  it('requires the live follow grant, and never opts absent arrangements into creation', () => {
    expect(pinsEditable(state)).toBe(true)
    for (const next of [{ phase: 'synced' as const, members: state.members }, { ...state, editable: false },
      ...(['connecting', 'pending', 'unavailable'] as const).map((phase) => ({ ...state, phase }))]) {
      expect(pinsEditable(next)).toBe(false)
      expect(planPinToggle(next, 'agent/new')).toEqual([])
    }
  })
  it('keeps disabled fixtures inert without a browser origin or client', () => {
    const registry = AtomRegistry.make()
    try {
      expect(sidebarPins()).toBe(sidebarPins())
      const pins = registry.get(sidebarPins())
      pins.toggle('agent/example'); pins.retry()
      expect(pins).toMatchObject({ editable: false, state: { phase: 'synced', members: [] } })
    } finally { registry.dispose() }
  })
})
