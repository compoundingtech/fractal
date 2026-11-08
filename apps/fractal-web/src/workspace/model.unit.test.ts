import { describe, expect, it } from 'vitest'
import * as Workspace from '@fractal/ui/workspace'
import { activeTab, currentSpace, emptyWorkspace, showsTerminal, tabsOf, transition, workspaceConfig, type Command, type WorkspaceState } from './model.ts'

// The reducer, its invariants and its property tests belong to `@fractal/ui/workspace`. These cover Fractal's
// binding of it: pane keys as refs, and one tab per agent with its terminal as a face.

const counter = () => {
  let next = 0
  return () => `id-${next++}`
}

const setup = () => {
  const config = workspaceConfig(counter())
  const run = (state: WorkspaceState, steps: readonly Command[]) => steps.reduce((current, step) => transition(current, step, config), state)
  const open = (...panes: readonly string[]) => run(emptyWorkspace(config), panes.map(ref => ({ _tag: 'Open', ref }) as const))
  return { config, run, open }
}
const refs = (state: WorkspaceState) => tabsOf(state).map(tab => tab.ref)
const tabOf = (state: WorkspaceState, ref: string) => tabsOf(state).find(tab => tab.ref === ref)!

describe('one tab per agent', () => {
  it('keeps a terminal as a face of the agent\'s tab, however it is reached', () => {
    const { run, open } = setup()
    const state = run(open('agent:agent/a', 'list:home'), [{ _tag: 'Open', ref: 'terminal:agent/a' }])
    expect(refs(state)).toEqual(['agent:agent/a', 'list:home'])
    const tab = activeTab(state)!
    expect(tab.ref).toBe('agent:agent/a')
    expect(showsTerminal(tab, currentSpace(state).local.views[tab.id])).toBe(true)
  })

  it('opens a terminal link as the agent\'s tab on its terminal face', () => {
    const { open } = setup()
    const state = open('terminal:agent/a')
    const tab = activeTab(state)!
    expect(tab.ref).toBe('agent:agent/a')
    expect(showsTerminal(tab, currentSpace(state).local.views[tab.id])).toBe(true)
  })

  it('remembers the face per tab across focus', () => {
    const { run, open } = setup()
    let state = open('agent:agent/a', 'agent:agent/b')
    const a = tabOf(state, 'agent:agent/a').id
    state = run(state, [{ _tag: 'SetView', tab: a, view: { terminal: true } }, { _tag: 'Open', ref: 'agent:agent/a' }, { _tag: 'Open', ref: 'agent:agent/b' }])
    expect(showsTerminal(activeTab(state)!, currentSpace(state).local.views[activeTab(state)!.id])).toBe(false)
    state = run(state, [{ _tag: 'Open', ref: 'agent:agent/a' }])
    expect(showsTerminal(activeTab(state)!, currentSpace(state).local.views[a])).toBe(true)
  })

  it('collapses a glass that holds an agent and its terminal to one tab', () => {
    const { run, open } = setup()
    const initial = open('list:home')
    const state = run(initial, [{ _tag: 'Receive', space: initial.current, body: { name: 'Main', layout: { tabs: [{ ref: 'agent:agent/a' }, { ref: 'terminal:agent/a' }, { ref: 'list:home' }] } } }])
    expect(refs(state)).toEqual(['agent:agent/a', 'list:home'])
  })

  it('keeps a key naming the terminal itself, showing that face until toggled', () => {
    const { config, run } = setup()
    const initial = emptyWorkspace(config)
    let state = run(initial, [{ _tag: 'Receive', space: initial.current, body: { name: 'Main', layout: { tabs: [{ ref: 'terminal:agent/a' }] } } }])
    const tab = activeTab(state)!
    expect(tab.ref).toBe('terminal:agent/a')
    expect(showsTerminal(tab, currentSpace(state).local.views[tab.id])).toBe(true)
    state = run(state, [{ _tag: 'SetView', tab: tab.id, view: { terminal: false } }])
    expect(showsTerminal(tab, currentSpace(state).local.views[tab.id])).toBe(false)
    expect(Workspace.spaceBody(currentSpace(state), { sizes: false }).layout).toEqual({ tabs: [{ ref: 'terminal:agent/a' }] })
  })
})
