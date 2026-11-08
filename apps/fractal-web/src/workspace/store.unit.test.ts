import { describe, expect, it } from 'vitest'
import * as Workspace from '@fractal/ui/workspace'
import { activeTab, currentSpace, emptyWorkspace, showsTerminal, tabsOf, transition, workspaceConfig } from './model.ts'
import { createWorkspaceStore } from './store.ts'

// The store's persistence, debouncing, other windows and external space stores are `@fractal/ui/workspace/store`'s,
// and tested there. These cover what Fractal adds: where it starts and the terminal face.

const counter = () => {
  let next = 0
  return () => `id-${next++}`
}

describe('workspace store', () => {
  it('opens a terminal link on the agent\'s tab, on its terminal face, through dispatch', () => {
    const store = createWorkspaceStore({ newId: counter() })
    store.dispatch({ _tag: 'Open', ref: 'agent:agent/a' })
    store.dispatch({ _tag: 'Open', ref: 'list:home' })
    store.dispatch({ _tag: 'Open', ref: 'terminal:agent/a' })
    const state = store.getState()
    expect(tabsOf(state).map(tab => tab.ref)).toEqual(['agent:agent/a', 'list:home'])
    expect(showsTerminal(activeTab(state)!, currentSpace(state).local.views[activeTab(state)!.id])).toBe(true)
    store.dispose()
  })

  it('starts from what it is given, such as the person\'s glasses', () => {
    const config = workspaceConfig(counter())
    const initial = transition(emptyWorkspace(config), { _tag: 'Open', ref: 'list:home' }, config)
    const store = createWorkspaceStore({ initial, newId: counter() })
    expect(tabsOf(store.getState()).map(tab => tab.ref)).toEqual(['list:home'])
    expect(currentSpace(store.getState()).id).toBe(currentSpace(initial).id)
    store.dispose()
  })

  it('starts empty, in one space called Main, with nothing given', () => {
    const store = createWorkspaceStore({ newId: counter() })
    expect(tabsOf(store.getState())).toEqual([])
    expect(currentSpace(store.getState()).name).toBe('Main')
    store.dispose()
  })

  it('notifies only on real changes', () => {
    const store = createWorkspaceStore({ newId: counter() })
    let notified = 0
    store.subscribe(() => { notified += 1 })
    store.dispatch({ _tag: 'Open', ref: 'list:home' })
    store.dispatch({ _tag: 'Open', ref: 'list:home' })
    expect(notified).toBe(1)
    expect(Workspace.panesOf(currentSpace(store.getState()).layout)).toHaveLength(1)
    store.dispose()
  })
})
