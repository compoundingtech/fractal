import { Effect, Layer, PubSub, Schema, Stream } from 'effect'
import * as KeyValueStore from 'effect/persistence/KeyValueStore'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { activeTab, groupsOf, tabsOf, WorkspaceState, type Layout } from './model.ts'
import { createWorkspaceStore, localStorageLayer, memoryStorage, WorkspaceStorage, type WorkspaceStorageLayer } from './store.ts'

const settle = () => Effect.runPromise(Effect.sleep(30))
const quiet = () => Effect.runPromise(Effect.sleep(200))

const key = 'test-workspace'
const unchanged = Layer.succeed(WorkspaceStorage, WorkspaceStorage.of({ key, changed: Stream.empty }))

const backedBy = (backing: Map<string, string>, onSet?: (value: string) => void) => KeyValueStore.makeStringOnly({
  get: key => Effect.sync(() => backing.get(key)),
  set: (key, value) => Effect.sync(() => {
    backing.set(key, value)
    onSet?.(value)
  }),
  remove: key => Effect.sync(() => { backing.delete(key) }),
  clear: Effect.sync(() => backing.clear()),
  size: Effect.sync(() => backing.size),
  keys: Effect.sync(() => [...backing.keys()]),
})

/** A store whose document outlives the store, as a reload's does. */
const lasting = (onSet?: (value: string) => void): { readonly layer: WorkspaceStorageLayer; readonly backing: Map<string, string> } => {
  const backing = new Map<string, string>()
  return { backing, layer: Layer.merge(unchanged, Layer.succeed(KeyValueStore.KeyValueStore, backedBy(backing, onSet))) }
}

/** Two stores over one document, each told about the other's writes, like two browser windows. */
const sharedDocument = () => {
  const backing = new Map<string, string>()
  const writes = Effect.runSync(PubSub.unbounded<number>())
  let windows = 0, written = 0
  const window = (): WorkspaceStorageLayer => {
    const self = windows++
    return Layer.merge(
      Layer.succeed(WorkspaceStorage, WorkspaceStorage.of({
        key,
        changed: Stream.fromPubSub(writes).pipe(Stream.filter(writer => writer !== self), Stream.as(undefined)),
      })),
      Layer.succeed(KeyValueStore.KeyValueStore, backedBy(backing, () => {
        written += 1
        Effect.runSync(PubSub.publish(writes, self))
      })),
    )
  }
  // A count, not the stored string: two windows rewriting the same document forever would leave the string unchanged.
  return { window, writes: () => written }
}

const panes = (layout: Layout) => tabsOf(layout).map(tab => tab.pane)

describe('workspace store', () => {
  it('does not persist an untouched empty workspace', () => {
    const shared = sharedDocument()
    const store = createWorkspaceStore(shared.window())
    try {
      store.flush()
      expect(shared.writes()).toBe(0)
    } finally {
      store.dispose()
    }
  })

  it('survives a reload: splits, each group\'s tab, focus, view state and the panel', () => {
    const { layer } = lasting()
    const first = createWorkspaceStore(layer, { debounceMs: 1 })
    first.dispatch({ _tag: 'Open', pane: 'agent:agent/a' })
    first.dispatch({ _tag: 'Open', pane: 'list:home' })
    const home = activeTab(first.getState())!
    first.dispatch({ _tag: 'SetView', id: home.id, view: { scrollTop: 120 } })
    first.dispatch({ _tag: 'Move', id: home.id, to: { _tag: 'Edge', group: first.getState().focus, side: 'right', first: false } })
    first.dispatch({ _tag: 'SetPanel', panel: { open: true, tab: 'changes' } })
    first.flush()
    first.dispose()
    const second = createWorkspaceStore(layer, { debounceMs: 1 })
    const state = second.getState()
    expect(groupsOf(state.layout).map(group => group.tabs.map(tab => tab.pane))).toEqual([['agent:agent/a'], ['list:home']])
    expect(activeTab(state)?.pane).toBe('list:home')
    expect(state.views[activeTab(state)!.id]?.scrollTop).toBe(120)
    expect(state.panel).toEqual({ open: true, tab: 'changes' })
    second.dispose()
  })

  it('debounces writes into one and persists without a flush', async () => {
    let writes = 0
    const { layer } = lasting(() => { writes += 1 })
    const store = createWorkspaceStore(layer, { debounceMs: 5 })
    for (const pane of ['agent:agent/a', 'agent:agent/b', 'agent:agent/c']) store.dispatch({ _tag: 'Open', pane })
    await settle()
    expect(writes).toBe(1)
    store.dispose()
  })

  it('falls back to an empty workspace on a corrupt document', () => {
    const store = createWorkspaceStore(memoryStorage('{not json'))
    expect(tabsOf(store.getState().layout)).toEqual([])
    store.dispose()
  })

  it('falls back to an empty workspace on a document of another shape', () => {
    const store = createWorkspaceStore(memoryStorage(JSON.stringify({ tabs: [{ id: 'x', pane: 'list:home' }], active: 'x' })))
    expect(tabsOf(store.getState().layout)).toEqual([])
    store.dispose()
  })

  it('repairs a stored document whose references no longer hold', () => {
    const tab = { id: 'x', pane: 'list:home' }
    const document = JSON.stringify({
      layout: { split: 'right', children: [{ id: 'left', tabs: [tab], active: 'gone' }, { id: 'right', tabs: [{ id: 'y', pane: 'list:home' }], active: 'y' }] },
      focus: 'gone', mru: ['gone'], closed: [], views: { gone: {} }, panel: { open: false, tab: 'details' },
    })
    const store = createWorkspaceStore(memoryStorage(document))
    const state = store.getState()
    expect(state.layout).toEqual({ id: 'left', tabs: [tab], active: 'x' })
    expect(state.focus).toBe('left')
    expect(state.mru).toEqual(['x'])
    expect(state.views).toEqual({})
    store.dispose()
  })

  it('reads a document the plain JSON codec wrote, as the first release of this store did', () => {
    const tab = { id: 'x', pane: 'future-kind:x y/z', title: 'Kept' }
    const document = Schema.encodeSync(Schema.fromJsonString(WorkspaceState))({
      layout: { split: 'right', ratio: 0.3, children: [{ id: 'left', tabs: [tab], active: 'x' }, { id: 'right', tabs: [{ id: 'y', pane: 'list:home' }], active: 'y' }] },
      focus: 'left', mru: ['x', 'y'], closed: [], views: { x: { scrollTop: 4 } }, panel: { open: true, tab: 'changes', width: 300 },
    })
    const state = createWorkspaceStore(memoryStorage(document)).getState()
    expect(groupsOf(state.layout)[0]?.tabs[0]).toEqual(tab)
    expect(state.views).toEqual({ x: { scrollTop: 4 } })
    expect(state.panel.width).toBe(300)
  })

  it('does not write an unchanged document back when it loads', async () => {
    let writes = 0
    const { layer } = lasting(() => { writes += 1 })
    const first = createWorkspaceStore(layer, { debounceMs: 1 })
    first.dispatch({ _tag: 'Open', pane: 'list:home' })
    first.flush()
    first.dispose()
    writes = 0
    const second = createWorkspaceStore(layer, { debounceMs: 1 })
    await settle()
    second.flush()
    expect(writes).toBe(0)
    second.dispose()
  })

  it('notifies only on real changes', () => {
    const store = createWorkspaceStore(memoryStorage())
    let notified = 0
    store.subscribe(() => { notified += 1 })
    store.dispatch({ _tag: 'Open', pane: 'list:home' })
    store.dispatch({ _tag: 'Open', pane: 'list:home' })
    expect(notified).toBe(1)
    store.dispose()
  })

  it('takes a second window\'s structure, keeps its own active tab, and settles', async () => {
    const shared = sharedDocument()
    const left = createWorkspaceStore(shared.window(), { debounceMs: 1 })
    const right = createWorkspaceStore(shared.window(), { debounceMs: 1 })
    left.dispatch({ _tag: 'Open', pane: 'agent:agent/a' })
    left.dispatch({ _tag: 'Open', pane: 'agent:agent/b' })
    await vi.waitFor(() => expect(panes(right.getState().layout)).toEqual(['agent:agent/a', 'agent:agent/b']))
    right.dispatch({ _tag: 'Open', pane: 'agent:agent/a' })
    await settle()
    expect(activeTab(right.getState())?.pane).toBe('agent:agent/a')
    expect(activeTab(left.getState())?.pane).toBe('agent:agent/b')
    right.dispatch({ _tag: 'Open', pane: 'list:home' })
    await vi.waitFor(() => expect(panes(left.getState().layout)).toEqual(['agent:agent/a', 'agent:agent/b', 'list:home']))
    expect(activeTab(left.getState())?.pane).toBe('agent:agent/b')
    // Settled means neither window writes again: wait out any exchange still in flight, then count writes over a window.
    await quiet()
    const settled = shared.writes()
    await quiet()
    expect(shared.writes(), 'no window writes once adoption settles').toBe(settled)
    left.dispose()
    right.dispose()
  })

  it('states the cost of adoption: a tab opened before the other window\'s write arrives is discarded', async () => {
    const shared = sharedDocument()
    const left = createWorkspaceStore(shared.window(), { debounceMs: 1 })
    const right = createWorkspaceStore(shared.window(), { debounceMs: 50 })
    left.dispatch({ _tag: 'Open', pane: 'agent:agent/a' })
    right.dispatch({ _tag: 'Open', pane: 'list:home' })
    await settle()
    expect(panes(right.getState().layout)).toEqual(['agent:agent/a'])
    left.dispose()
    right.dispose()
  })
})

describe('workspace store over localStorage', () => {
  const documentKey = 'workspace-under-test'
  const seen = (() => {
    const values = new Map<string, string>()
    return {
      getItem: (name: string) => values.get(name) ?? null,
      setItem: (name: string, value: string) => { values.set(name, value) },
      removeItem: (name: string) => { values.delete(name) },
      clear: () => values.clear(),
      key: () => null,
      get length() { return values.size },
    }
  })()
  afterEach(() => {
    vi.unstubAllGlobals()
    seen.clear()
  })
  const windowStub = () => {
    const target = new EventTarget()
    vi.stubGlobal('window', target)
    return target
  }
  const storageEvent = (name: string | null, newValue: string | null) => Object.assign(new Event('storage'), { key: name, newValue })

  it('persists a reload through the browser store, first render included', () => {
    windowStub()
    vi.stubGlobal('localStorage', seen)
    const first = createWorkspaceStore(localStorageLayer(documentKey))
    first.dispatch({ _tag: 'Open', pane: 'list:home' })
    first.flush()
    first.dispose()
    const second = createWorkspaceStore(localStorageLayer(documentKey))
    expect(panes(second.getState().layout)).toEqual(['list:home'])
    second.dispose()
  })

  it('stays in memory when reading localStorage itself throws', () => {
    windowStub()
    vi.stubGlobal('localStorage', undefined)
    Object.defineProperty(globalThis, 'localStorage', { configurable: true, get: () => { throw new Error('SecurityError') } })
    const store = createWorkspaceStore(localStorageLayer(documentKey))
    store.dispatch({ _tag: 'Open', pane: 'list:home' })
    store.flush()
    expect(panes(store.getState().layout)).toEqual(['list:home'])
    store.dispose()
  })

  it('stays in memory when a write is refused', () => {
    windowStub()
    vi.stubGlobal('localStorage', { ...seen, getItem: seen.getItem, setItem: () => { throw new Error('QuotaExceededError') } })
    const store = createWorkspaceStore(localStorageLayer(documentKey))
    store.dispatch({ _tag: 'Open', pane: 'list:home' })
    store.flush()
    expect(panes(store.getState().layout)).toEqual(['list:home'])
    store.dispose()
  })

  it('hears another window from the moment it exists, only for its own key, and stops when disposed', async () => {
    const target = windowStub()
    vi.stubGlobal('localStorage', seen)
    const writer = createWorkspaceStore(memoryStorage())
    writer.dispatch({ _tag: 'Open', pane: 'list:missions' })
    const document = Schema.encodeSync(Schema.fromJsonString(WorkspaceState))(writer.getState())
    writer.dispose()
    const store = createWorkspaceStore(localStorageLayer(documentKey))
    seen.setItem(documentKey, document)
    target.dispatchEvent(storageEvent('another-key', document))
    await settle()
    expect(panes(store.getState().layout)).toEqual([])
    target.dispatchEvent(storageEvent(documentKey, document))
    await settle()
    expect(panes(store.getState().layout)).toEqual(['list:missions'])
    store.dispose()
    await settle()
    seen.setItem(documentKey, Schema.encodeSync(Schema.fromJsonString(WorkspaceState))({ ...writer.getState(), layout: { id: 'main', tabs: [], active: null } }))
    target.dispatchEvent(storageEvent(documentKey, ''))
    await settle()
    expect(panes(store.getState().layout)).toEqual(['list:missions'])
  })
})
