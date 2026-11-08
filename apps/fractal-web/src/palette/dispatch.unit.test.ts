import { describe, expect, it } from 'vitest'
import { activeTab, currentSpace, emptyWorkspace, focusedPane, panesOf, tabsOf, transition, type Command, type WorkspaceConfig, type WorkspaceState } from '@fractal/ui/workspace'
import { openCommands, namingCommand, paneOf } from './dispatch.ts'

let next = 0
const config: WorkspaceConfig = { newId: () => `id${next++}` }
const run = (state: WorkspaceState, commands: readonly Command[]) => commands.reduce((current, command) => transition(current, command, config), state)
const refs = (state: WorkspaceState) => tabsOf(currentSpace(state).layout).map(tab => tab.ref)
const shown = (state: WorkspaceState) => activeTab(currentSpace(state))?.ref

const start = (...opened: readonly string[]) => run(emptyWorkspace(config, { name: 'Main' }), opened.map(ref => ({ _tag: 'Open', ref })))

describe('open targets', () => {
  it('here replaces the focused pane’s shown tab, as the TUI’s Open::Here does', () => {
    const state = start('agent:agent/a', 'agent:agent/b')
    const after = run(state, openCommands(state, 'mission:mission/m', 'here'))
    expect(refs(after)).toEqual(['agent:agent/a', 'mission:mission/m'])
    expect(shown(after)).toBe('mission:mission/m')
  })

  it('here reveals a tab already open anywhere instead of opening it twice', () => {
    const opened = run(start('agent:agent/a'), [{ _tag: 'Split', side: 'right', ref: 'agent:agent/b' }])
    const split = run(opened, [{ _tag: 'FocusPane', pane: panesOf(currentSpace(opened).layout)[0]!.id }])
    const after = run(split, openCommands(split, 'agent:agent/b', 'here'))
    expect(refs(after)).toEqual(['agent:agent/a', 'agent:agent/b'])
    expect(shown(after)).toBe('agent:agent/b')
  })

  it('here opens into an empty pane', () => {
    const state = start()
    expect(refs(run(state, openCommands(state, 'list:home', 'here')))).toEqual(['list:home'])
  })

  it('tab adds a tab to the focused pane', () => {
    const state = start('agent:agent/a')
    const after = run(state, openCommands(state, 'agent:agent/b', 'tab'))
    expect(refs(after)).toEqual(['agent:agent/a', 'agent:agent/b'])
    expect(panesOf(currentSpace(after).layout)).toHaveLength(1)
  })

  it('right and below open a split on that side', () => {
    const state = start('agent:agent/a')
    const right = run(state, openCommands(state, 'agent:agent/b', 'right'))
    expect(panesOf(currentSpace(right).layout).map(pane => pane.tabs.map(tab => tab.ref))).toEqual([['agent:agent/a'], ['agent:agent/b']])
    const below = run(state, openCommands(state, 'agent:agent/b', 'below'))
    const layout = currentSpace(below).layout
    expect('axis' in layout && layout.axis).toBe('column')
    expect(focusedPane(currentSpace(below))?.tabs.map(tab => tab.ref)).toEqual(['agent:agent/b'])
  })

  it('compares refs by the host’s identity, so an agent’s terminal is its conversation tab', () => {
    const state = start('agent:agent/a', 'mission:mission/m')
    const identity = (ref: string) => ref.replace(/^terminal:/u, 'agent:')
    expect(openCommands(state, 'terminal:agent/a', 'here', identity)).toEqual([{ _tag: 'Open', ref: 'terminal:agent/a' }])
  })
})

describe('naming', () => {
  it('names a new space, renames and duplicates the current one', () => {
    const state = start('agent:agent/a')
    const space = currentSpace(state).id
    expect(namingCommand(state, 'create', 'Review')).toEqual({ _tag: 'SpaceNew', name: 'Review' })
    expect(namingCommand(state, 'rename', 'Work')).toEqual({ _tag: 'SpaceRename', space, name: 'Work' })
    expect(namingCommand(state, 'duplicate', 'Copy')).toEqual({ _tag: 'SpaceDuplicate', space, name: 'Copy' })
    const renamed = run(state, [namingCommand(state, 'rename', 'Work')])
    expect(currentSpace(renamed).name).toBe('Work')
  })
})

describe('paneOf', () => {
  it('is the focused pane’s id', () => {
    const state = start('agent:agent/a')
    expect(paneOf(state)).toBe(focusedPane(currentSpace(state))?.id)
  })
})
