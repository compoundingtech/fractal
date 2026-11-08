import { Effect, Layer } from 'effect'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import { activeTab, groupsOf, tabsOf, type Layout } from './model.ts'
import { createWorkspaceStore, memoryStorage, WorkspaceStorage } from './store.ts'

// Drain the persistence debounce and any writes triggered by adoption in the other window.
const settle = () => vi.runAllTimersAsync()

/** Two stores over one document, each told about the other's writes, like two browser windows. */
const sharedDocument = () => {
  let document: string | undefined
  const listeners = new Set<(value: string) => void>()
  const window = () => {
    let own: ((value: string) => void) | undefined
    return Layer.succeed(WorkspaceStorage, WorkspaceStorage.of({
      load: Effect.sync(() => document),
      save: next => Effect.sync(() => {
        document = next
        for (const listener of listeners) if (listener !== own) listener(next)
      }),
      subscribe: listener => {
        own = listener
        listeners.add(listener)
        return () => listeners.delete(listener)
      },
    }))
  }
  return { window, read: () => document }
}

const panes = (layout: Layout) => tabsOf(layout).map(tab => tab.pane)

describe('workspace store', () => {
  beforeEach(() => vi.useFakeTimers())
  afterEach(() => vi.useRealTimers())

  it('survives a reload: splits, each group\'s tab, focus, view state and the panel', () => {
    let saved: string | undefined
    const layer = Layer.succeed(WorkspaceStorage, WorkspaceStorage.of({
      load: Effect.sync(() => saved),
      save: next => Effect.sync(() => { saved = next }),
      subscribe: () => () => {},
    }))
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
    const layer = Layer.succeed(WorkspaceStorage, WorkspaceStorage.of({
      load: Effect.succeed(undefined),
      save: () => Effect.sync(() => { writes += 1 }),
      subscribe: () => () => {},
    }))
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
    await settle()
    expect(panes(right.getState().layout)).toEqual(['agent:agent/a', 'agent:agent/b'])
    right.dispatch({ _tag: 'Open', pane: 'agent:agent/a' })
    await settle()
    expect(activeTab(right.getState())?.pane).toBe('agent:agent/a')
    expect(activeTab(left.getState())?.pane).toBe('agent:agent/b')
    right.dispatch({ _tag: 'Open', pane: 'list:home' })
    await settle()
    expect(panes(left.getState().layout)).toEqual(['agent:agent/a', 'agent:agent/b', 'list:home'])
    expect(activeTab(left.getState())?.pane).toBe('agent:agent/b')
    const settled = shared.read()
    await settle()
    expect(shared.read()).toBe(settled)
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
