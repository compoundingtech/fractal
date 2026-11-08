import { describe, expect, it } from 'vitest'
import * as Workspace from '@fractal/ui/workspace'
import { activeTab, currentSpace, showsTerminal, tabsOf } from './model.ts'
import { createWorkspaceStore } from './store.ts'

// The store's persistence, debouncing, other windows and external space stores are `@fractal/ui/workspace/store`'s,
// and tested there. These cover what Fractal adds: its document, its legacy document and the terminal face.

const counter = () => {
  let next = 0
  return () => `id-${next++}`
}

describe('workspace store', () => {
  it('opens a terminal link on the agent\'s tab, on its terminal face, through dispatch', () => {
    const store = createWorkspaceStore({ storageKey: undefined, newId: counter() })
    store.dispatch({ _tag: 'Open', ref: 'agent:agent/a' })
    store.dispatch({ _tag: 'Open', ref: 'list:home' })
    store.dispatch({ _tag: 'Open', ref: 'terminal:agent/a' })
    const state = store.getState()
    expect(tabsOf(state).map(tab => tab.ref)).toEqual(['agent:agent/a', 'list:home'])
    expect(showsTerminal(activeTab(state)!, currentSpace(state).local.views[activeTab(state)!.id])).toBe(true)
    store.dispose()
  })

  it('starts from the document before the shared primitives', () => {
    const legacy = JSON.stringify({
      layout: { id: 'main', tabs: [{ id: 'a', pane: 'agent:agent/a' }, { id: 'b', pane: 'list:home' }], active: 'b' },
      focus: 'main', mru: ['b', 'a'], closed: [], views: { b: { scrollTop: 120 } },
    })
    const store = createWorkspaceStore({ storageKey: undefined, initial: legacy, newId: counter() })
    const state = store.getState()
    expect(tabsOf(state).map(tab => tab.ref)).toEqual(['agent:agent/a', 'list:home'])
    expect(activeTab(state)?.ref).toBe('list:home')
    expect(currentSpace(state).local.views.b?.scrollTop).toBe(120)
    store.dispose()
  })

  it('falls back to an empty workspace on a corrupt document', () => {
    const store = createWorkspaceStore({ storageKey: undefined, initial: '{not json', newId: counter() })
    expect(tabsOf(store.getState())).toEqual([])
    expect(currentSpace(store.getState()).name).toBe('Main')
    store.dispose()
  })

  it('notifies only on real changes', () => {
    const store = createWorkspaceStore({ storageKey: undefined, newId: counter() })
    let notified = 0
    store.subscribe(() => { notified += 1 })
    store.dispatch({ _tag: 'Open', ref: 'list:home' })
    store.dispatch({ _tag: 'Open', ref: 'list:home' })
    expect(notified).toBe(1)
    expect(Workspace.panesOf(currentSpace(store.getState()).layout)).toHaveLength(1)
    store.dispose()
  })
})
