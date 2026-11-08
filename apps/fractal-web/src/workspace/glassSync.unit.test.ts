import { describe, expect, it, vi } from 'vitest'
import * as Workspace from '@fractal/ui/workspace'
import type { CollectionFrame, CollectionStream, Glass, GlassBody, GlassLayout } from '@fractal/client'
import { ClientError } from '@fractal/client'
import { bootGlasses, fromGlassBody, glassProblem, toGlassBody, type GlassGateway } from './glassSync.ts'
import { currentSpace, tabsOf, workspaceConfig } from './model.ts'
import { createWorkspaceStore } from './store.ts'

const uuid = (n: number) => `019a0000-0000-7000-8000-${n.toString(16).padStart(12, '0')}`
const glass = (n: number, body: GlassBody | null, revision = `r${n}`): Glass => ({
  id: `glass/person/ada/${uuid(n)}`, kind: 'glass', revision, updated_at: '2026-11-08T00:00:00Z',
  deleted: body === null, body, base_revision: null, replaced_revision: null,
} as Glass)

const counter = () => {
  let next = 0
  return () => uuid(1000 + next++)
}

/** A gateway over one fake socket: `emit` sends a frame for the workspace's subscription. */
const fakeGateway = ({ granted = true, version = 2, glasses = [] as readonly Glass[], silent = false, echoMs = 5 } = {}) => {
  const puts: { id: string; base: string | null; body: GlassBody; key: string }[] = []
  const deletes: string[] = []
  let onFrame: ((frame: CollectionFrame) => void) | undefined
  let subscription = ''
  let revision = 100
  const gateway: GlassGateway = {
    discover: async () => ({ value: { capabilities: [{ id: 'glasses', version, state: granted ? 'granted' : 'ungranted' }] } }) as never,
    collectionStream: async options => {
      onFrame = options.onFrame
      return {
        subscribeGlasses: (id: string) => {
          subscription = id
          if (!silent) queueMicrotask(() => onFrame?.({ kind: 'snapshot', id, collection: 'glasses', items: glasses, order: glasses.map(row => row.id), has_more: false, snapshot: {} } as never))
        },
        close: () => {},
      } as unknown as CollectionStream
    },
    putGlass: async (id, request, key) => {
      puts.push({ id, base: request.base_revision, body: request.body, key })
      revision += 1
      // st stores the body as written and sends it back on the subscription, as every client sees it.
      const stored = { ...glass(0, request.body, `r${revision}`), id: `glass/person/ada/${id}` } as Glass
      setTimeout(() => emit([stored]), echoMs)
      return { value: stored } as never
    },
    deleteGlass: async id => {
      deletes.push(id)
      return { value: glass(0, null) } as never
    },
  }
  function emit(upserts: readonly Glass[]) {
    onFrame?.({ kind: 'changes', id: subscription, collection: 'glasses', upserts, removes: [], order: [], has_more: false, snapshot: {} } as never)
  }
  return { gateway, puts, deletes, emit }
}

const settle = () => new Promise(resolve => setTimeout(resolve, 60))
const split = (side: 'right' | 'below', a: GlassLayout, b: GlassLayout, ratio?: number): GlassLayout =>
  ratio === undefined ? { split: side, children: [a, b] } : { split: side, children: [a, b], ratio }
const tabs = (...panes: readonly string[]): GlassLayout => ({ tabs: panes.map(pane => ({ pane })) })

describe('the glass body', () => {
  it('carries a layout both ways, longer splits nested two at a time', () => {
    const body: Workspace.SpaceBody = {
      name: 'Main',
      layout: { axis: 'row', sizes: [0.5, 0.25, 0.25], children: [{ tabs: [{ ref: 'agent:agent/a', title: 'Mine' }] }, { tabs: [{ ref: 'list:home' }] }, { axis: 'column', children: [{ tabs: [{ ref: 'usage:' }] }, { tabs: [] }] }] },
    }
    const glassBody = toGlassBody(body, { ratios: true })
    expect(glassBody.layout).toEqual(split('right', { tabs: [{ pane: 'agent:agent/a', title: 'Mine' }] }, split('right', tabs('list:home'), split('below', tabs('usage:'), tabs()), 0.5), 0.5))
    expect(Workspace.canonicalBody(fromGlassBody(glassBody))).toBe(Workspace.canonicalBody({ ...body, layout: Workspace.toBinary(body.layout) }))
  })

  it('clamps ratios to what st keeps, and leaves them out where it keeps none', () => {
    const body: Workspace.SpaceBody = { name: 'Main', layout: { axis: 'row', sizes: [0.02, 0.98], children: [{ tabs: [{ ref: 'a:1' }] }, { tabs: [{ ref: 'a:2' }] }] } }
    expect(toGlassBody(body, { ratios: true }).layout).toMatchObject({ ratio: 0.1 })
    expect(toGlassBody(body, { ratios: false }).layout).not.toHaveProperty('ratio')
  })

  it('names what st would refuse', () => {
    let deep: GlassLayout = tabs('a:1')
    for (let level = 0; level < 32; level += 1) deep = split('right', deep, tabs('a:2'))
    expect(glassProblem({ name: 'Main', layout: deep })).toMatch(/deeper/)
    expect(glassProblem({ name: 'Main', layout: tabs(...Array.from({ length: 1025 }, (_, index) => `a:${index}`)) })).toMatch(/more than 1024/)
    expect(glassProblem({ name: '', layout: tabs() })).toMatch(/name/)
    expect(glassProblem({ name: 'Main', layout: split('below', tabs('a:1'), tabs('a:2'), 0.4) })).toBeUndefined()
  })
})

describe('booting from the person\'s glasses', () => {
  it('stays in memory without the grant', async () => {
    const fake = fakeGateway({ granted: false })
    expect(await bootGlasses({ gateway: fake.gateway, config: workspaceConfig(counter()) })).toEqual({ _tag: 'Local', reason: 'ungranted' })
  })

  it('stays in memory when st cannot be reached, or sends nothing in time', async () => {
    const unreachable: GlassGateway = { ...fakeGateway().gateway, discover: async () => { throw new TypeError('fetch failed') } }
    expect(await bootGlasses({ gateway: unreachable, config: workspaceConfig(counter()) })).toEqual({ _tag: 'Local', reason: 'unreachable' })
    const silent = fakeGateway({ silent: true })
    expect(await bootGlasses({ gateway: silent.gateway, config: workspaceConfig(counter()), timeoutMs: 20 })).toEqual({ _tag: 'Local', reason: 'unreachable' })
  })

  it('starts from the glasses st has, the first one shown, and writes nothing', async () => {
    const fake = fakeGateway({ glasses: [glass(1, { name: 'Main', layout: split('right', tabs('agent:agent/a'), tabs('list:home'), 0.3) }), glass(2, { name: 'Review', layout: tabs('mission:m/1') })] })
    const boot = await bootGlasses({ gateway: fake.gateway, config: workspaceConfig(counter()) })
    if (boot._tag !== 'Synced') throw new Error(boot._tag)
    expect(boot.initial.spaces.map(space => [space.id, space.name, space.revision])).toEqual([[uuid(1), 'Main', 'r1'], [uuid(2), 'Review', 'r2']])
    expect(boot.initial.current).toBe(uuid(1))
    expect(Workspace.panesOf(currentSpace(boot.initial).layout).map(pane => pane.tabs.map(tab => tab.ref))).toEqual([['agent:agent/a'], ['list:home']])
    expect(currentSpace(boot.initial).layout).toMatchObject({ axis: 'row', sizes: [0.3, 0.7] })
    expect(fake.puts).toEqual([])
    boot.close()
  })

  it('makes one Main glass when the person has none', async () => {
    const fake = fakeGateway()
    const boot = await bootGlasses({ gateway: fake.gateway, config: workspaceConfig(counter()) })
    if (boot._tag !== 'Synced') throw new Error(boot._tag)
    expect(fake.puts).toEqual([{ id: currentSpace(boot.initial).id, base: null, body: { name: 'Main', layout: { tabs: [] } }, key: `create-${currentSpace(boot.initial).id}` }])
    expect(currentSpace(boot.initial).revision).toBe('r101')
    boot.close()
  })
})

describe('the workspace on the person\'s glasses', () => {
  const synced = async (glasses: readonly Glass[], { debounceMs = 5, echoMs = 5, open }: { debounceMs?: number; echoMs?: number; open?: string } = {}) => {
    const fake = fakeGateway({ glasses, echoMs })
    const boot = await bootGlasses({ gateway: fake.gateway, config: workspaceConfig(counter()) })
    if (boot._tag !== 'Synced') throw new Error(boot._tag)
    const store = createWorkspaceStore({ initial: boot.initial, sync: [boot.sync], debounceMs, newId: counter() })
    // As `useWorkspaceStore` opens the URL's subject.
    if (open !== undefined) store.dispatch({ _tag: 'Open', ref: open })
    return { fake, store, close: () => { store.dispose(); boot.close() } }
  }
  const shape = (state: ReturnType<ReturnType<typeof createWorkspaceStore>['getState']>) => JSON.stringify(Workspace.toWire(currentSpace(state).layout, { sizes: true }))

  it('keeps the tab a first visit opens, and writes it into the new Main glass', async () => {
    const { fake, store, close } = await synced([], { open: 'agent:agent/a' })
    await settle()
    expect(tabsOf(store.getState()).map(tab => tab.ref)).toEqual(['agent:agent/a'])
    expect(fake.puts.map(put => put.body.layout)).toEqual([tabs(), tabs('agent:agent/a')])
    close()
  })

  it('still writes a change made while its last write was coming back', async () => {
    const { fake, store, close } = await synced([glass(1, { name: 'Main', layout: tabs('list:home') })], { debounceMs: 30, echoMs: 5 })
    store.dispatch({ _tag: 'Open', ref: 'usage:' })
    await vi.waitFor(() => expect(fake.puts).toHaveLength(1))
    store.dispatch({ _tag: 'Open', ref: 'agent:agent/a' })
    await settle()
    expect(fake.puts).toHaveLength(2)
    expect(tabsOf(store.getState()).map(tab => tab.ref)).toEqual(['list:home', 'usage:', 'agent:agent/a'])
    close()
  })

  it('does not take its own write back as another writer\'s, though st keeps it binary and clamped', async () => {
    const { fake, store, close } = await synced([glass(1, { name: 'Main', layout: tabs('a:1') })])
    store.dispatch({ _tag: 'Split', side: 'right', ref: 'a:2' })
    store.dispatch({ _tag: 'Split', side: 'right', ref: 'a:3' })
    const split = currentSpace(store.getState()).layout as Extract<Workspace.Layout, { axis: string }>
    store.dispatch({ _tag: 'Resize', split: split.id, sizes: split.children.length === 3 ? [0.02, 0.49, 0.49] : [0.02, 0.98] })
    const before = shape(store.getState())
    await settle()
    expect(fake.puts.length).toBeGreaterThan(0)
    expect(shape(store.getState())).toBe(before)
    close()
  })

  it('adopts what another writer sends and never writes it back, even where it normalised it', async () => {
    const { fake, store, close } = await synced([glass(1, { name: 'Main', layout: tabs('list:home') })])
    // The TUI keeps an agent and its terminal as two tabs; here they are one.
    fake.emit([glass(1, { name: 'Main', layout: split('right', tabs('agent:agent/a', 'terminal:agent/a'), tabs('list:home'), 0.95) }, 'r7')])
    await settle()
    expect(tabsOf(store.getState()).map(tab => tab.ref)).toEqual(['agent:agent/a', 'list:home'])
    expect(currentSpace(store.getState()).revision).toBe('r7')
    expect(fake.puts).toEqual([])
    // Opening a tab here is a change of structure: written on the adopted revision.
    store.dispatch({ _tag: 'Open', ref: 'usage:' })
    await settle()
    expect(fake.puts).toHaveLength(1)
    expect(fake.puts[0]).toMatchObject({ id: uuid(1), base: 'r7' })
    close()
  })

  it('takes in a glass made elsewhere and lets one go when it is closed there', async () => {
    const { fake, store, close } = await synced([glass(1, { name: 'Main', layout: tabs('list:home') })])
    fake.emit([glass(2, { name: 'Review', layout: tabs('mission:m/1') })])
    await settle()
    expect(store.getState().spaces.map(space => space.name)).toEqual(['Main', 'Review'])
    fake.emit([glass(2, null)])
    await settle()
    expect(store.getState().spaces.map(space => space.name)).toEqual(['Main'])
    expect(fake.puts).toEqual([])
    expect(fake.deletes).toEqual([])
    close()
  })

  it('drops a write st refuses for good instead of retrying it', async () => {
    const { fake, store, close } = await synced([glass(1, { name: 'Main', layout: tabs('list:home') })])
    let attempts = 0
    fake.gateway.putGlass = async () => {
      attempts += 1
      throw new ClientError({ code: 'validation-failed', message: 'no', retryable: false } as never, 422)
    }
    store.dispatch({ _tag: 'Open', ref: 'usage:' })
    await settle()
    store.dispatch({ _tag: 'SetView', tab: tabsOf(store.getState())[0]!.id, view: { scrollTop: 4 } })
    await settle()
    expect(attempts).toBe(1)
    close()
  })
})
