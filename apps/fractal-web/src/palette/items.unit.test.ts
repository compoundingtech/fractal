import { describe, expect, it } from 'vitest'
import { emptyWorkspace, transition, type WorkspaceConfig } from '@fractal/ui/workspace'
import { rank, flatten, parseQuery } from '@fractal/ui/workspace/palette'
import { commandItems, leads, paletteSections, recentItems, saidItems, spaceItems, typedOrder, worldItems, type PaletteWorld } from './items.ts'

let next = 0
const config: WorkspaceConfig = { newId: () => `id${next++}` }
const world: PaletteWorld = {
  alerts: [{ ref: 'attention/1', title: 'Approve the release plan', kind: 'decision', status: { tone: 'needs-you', label: 'Waiting for you' } }],
  agents: [
    { ref: 'agent/example/atlas/builder', name: 'Atlas Builder', detail: 'claude · juniper', status: { tone: 'working', label: 'Working' }, terminal: true },
    { ref: 'agent/example/new/scout', name: 'New Scout', detail: 'codex · juniper' },
  ],
  missions: [{ ref: 'mission/release', title: 'Release 3.2', word: 'running' }],
  machines: [{ id: 'juniper', connected: true }],
  resources: [{ ref: 'github/pr/compoundingtech/fractal/12', title: 'Fix the palette', kind: 'pull request' }],
}
const ids = (query: string, items = worldItems(world)) => {
  const parsed = parseQuery(query, paletteSections)
  return flatten(rank(items, { query: parsed.text, sections: paletteSections, section: parsed.section, typedOrder, leads })).map(item => item.id)
}

describe('sections', () => {
  it('keeps the TUI’s eight sections first, so Alt+1 to Alt+8 match its digits', () => {
    expect(paletteSections.slice(0, 8).map(section => section.id)).toEqual(['alerts', 'agents', 'missions', 'fleet', 'spaces', 'start', 'said', 'sidebar'])
  })

  it('scopes by > @ # ! and /', () => {
    expect(Object.fromEntries(paletteSections.filter(section => section.prefix !== undefined).map(section => [section.prefix, section.id])))
      .toEqual({ '!': 'alerts', '@': 'agents', '#': 'missions', '/': 'resources', '>': 'commands' })
  })
})

describe('world rows', () => {
  it('opens each entity by its TUI pane key', () => {
    const byId = new Map(worldItems(world).map(item => [item.id, item.action.action]))
    expect(byId.get('agent:agent/example/atlas/builder')).toEqual({ _tag: 'Open', ref: 'agent:agent/example/atlas/builder' })
    expect(byId.get('mission:mission/release')).toEqual({ _tag: 'Open', ref: 'mission:mission/release' })
    expect(byId.get('home:attention/1')).toEqual({ _tag: 'Open', ref: 'home:attention/1' })
    expect(byId.get('list:fleet')).toEqual({ _tag: 'Open', ref: 'list:fleet' })
    expect(byId.get('machine:machine/juniper')).toEqual({ _tag: 'Open', ref: 'machine:machine/juniper' })
    expect(byId.get('resource:github/pr/compoundingtech/fractal/12')).toEqual({ _tag: 'Open', ref: 'resource:github/pr/compoundingtech/fractal/12' })
  })

  it('offers an agent’s terminal only once something is typed, as the TUI does', () => {
    expect(worldItems(world).some(item => item.id === 'terminal:agent/example/atlas/builder')).toBe(false)
    expect(ids('atlas term', worldItems(world, 'atlas term'))).toContain('terminal:agent/example/atlas/builder')
  })

  it('ranks a typed query agents first, then missions, then alerts', () => {
    const items = [...worldItems(world, 're'), ...commandItems({ state: emptyWorkspace(config), commands: [] })]
    const order = ids('re', items)
    expect(order.indexOf('mission:mission/release')).toBeLessThan(order.indexOf('home:attention/1'))
  })

  it('carries the entity descriptor for the row', () => {
    const agent = worldItems(world).find(item => item.id === 'agent:agent/example/atlas/builder')!
    expect(agent.action.entity).toEqual({ kind: 'agent', id: 'agent/example/atlas/builder', title: 'Atlas Builder', status: { tone: 'working', label: 'Working' } })
  })
})

describe('commands', () => {
  it('lifts a start command typed as it begins above an agent named alike', () => {
    const items = [...worldItems(world, 'new'), ...commandItems({
      state: emptyWorkspace(config),
      commands: [{ id: 'new-mission', section: 'start', label: 'New mission', detail: 'describe it', search: 'new mission launch', run: () => {} }],
    })]
    expect(ids('new', items)[0]).toBe('new-mission')
  })

  it('offers split, zoom and close for the workspace', () => {
    const state = transition(emptyWorkspace(config), { _tag: 'Open', ref: 'agent:agent/a' }, config)
    const labels = (at: typeof state) => commandItems({ state: at, commands: [] }).filter(item => item.section === 'commands').map(item => item.label)
    expect(labels(state)).toEqual(['Split right', 'Split down', 'Close tab'])
    // Zoom means something once there is a split to fill the workspace.
    const split = transition(state, { _tag: 'Split', side: 'right', ref: 'agent:agent/b' }, config)
    expect(labels(split)).toContain('Zoom split')
  })
})

describe('spaces', () => {
  it('lists spaces, and naming rows for the current one; typed names make a row that always shows', () => {
    const state = emptyWorkspace(config, { name: 'Main' })
    const empty = spaceItems(state, '')
    expect(empty.map(item => item.label)).toEqual(['Main', 'Rename “Main”…', 'New space…', 'Duplicate “Main”…'])
    const typed = spaceItems(state, 'Review')
    expect(ids('Review', typed)).toEqual(expect.arrayContaining(['space-new-named', 'space-rename-named', 'space-duplicate-named']))
  })
})

describe('said in conversations', () => {
  it('turns st’s hits into rows that always show for their query, with the caveat first', () => {
    const items = saidItems('harbor keys', {
      items: [{ agent_id: 'agent/example/atlas/builder', conversation_id: 'c1', excerpt: 'the  harbor keys rotate', timestamp: '2026-11-08T09:30:00Z' }],
      refreshing: true,
    }, new Map([['agent/example/atlas/builder', 'Atlas Builder']]))
    expect(items[0]!.label).toBe('the harbor keys rotate')
    expect(items[0]!.detail).toBe('still indexing, more may come · Atlas Builder · 11-08 09:30')
    expect(ids('harbor keys', items)).toHaveLength(1)
    expect(items[0]!.action.action).toEqual({ _tag: 'Said', ref: 'agent:agent/example/atlas/builder', query: 'harbor keys' })
  })
})

describe('recent', () => {
  it('repeats recent rows under their own ids, newest first, for the empty query only', () => {
    const items = worldItems(world)
    const recent = recentItems(['mission:mission/release', 'agent:agent/example/atlas/builder', 'gone'], items)
    expect(recent.map(item => item.id)).toEqual(['recent:mission:mission/release', 'recent:agent:agent/example/atlas/builder'])
    expect(recent[0]!.action.key).toBe('mission:mission/release')
    expect(recent[0]!.search).toBe('')
  })
})
