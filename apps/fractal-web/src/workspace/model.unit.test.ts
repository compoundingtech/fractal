import { it } from '@effect/vitest'
import { Arbitrary, Schema } from 'effect'
import { describe, expect } from 'vitest'
import { activeTab, Command, emptyWorkspace, focusedGroup, glassBody, groupsOf, isGroup, repairWorkspace, showsTerminal, Side, tabsOf, transition, WorkspaceState, type Drop, type GlassLayout, type Layout, type ReceivedLayout } from './model.ts'
import { paneIdentity } from './subject.ts'

const counter = () => {
  let next = 0
  return { newId: () => `id-${next++}` }
}

const between = (minimum: number, maximum: number) => Schema.Int.check(Schema.isBetween({ minimum, maximum }))
const looseRatio = Schema.Finite.check(Schema.isBetween({ minimum: -1, maximum: 2 }))

/** The commands' own shapes over a small world, so that sequences keep hitting tabs and groups that exist. */
const Pane = Schema.Literals(['agent:agent/a', 'agent:agent/b', 'terminal:agent/a', 'terminal:agent/c', 'list:home', 'list:missions', 'usage:', 'document:doc/x', 'declaration:agent/a', 'resource:mail/1', 'machine:machine/m', 'future-kind:x'])
const Id = Schema.Literals(Array.from({ length: 16 }, (_, n) => `id-${n}`))
const GroupId = Schema.Union([Id, Schema.Literal('main')])
const NearDrop = Schema.TaggedUnion({
  Strip: { group: GroupId, index: between(-2, 12) },
  Edge: { group: GroupId, side: Side, first: Schema.Boolean },
})
const Received: Schema.Codec<ReceivedLayout> = Schema.Union([
  Schema.Struct({ tabs: Schema.Array(Schema.Struct({ pane: Pane })).check(Schema.isMaxLength(4)) }),
  Schema.Struct({
    split: Side,
    children: Schema.Array(Schema.suspend((): Schema.Codec<ReceivedLayout> => Received)).check(Schema.isMaxLength(3)),
    ratio: Schema.optional(looseRatio),
  }),
])
const FlatCommand = Schema.TaggedUnion({
  Open: { pane: Pane, activate: Schema.Boolean },
  Activate: { id: Id },
  Cycle: { by: Schema.Literals([1, -1]) },
  ActivateIndex: { index: between(-1, 9) },
  ActivatePrevious: {},
  Close: { id: Id },
  ReopenClosed: {},
  SetView: { id: Id, view: Schema.Struct({ scrollTop: between(0, 1000), terminal: Schema.Boolean }) },
  SetPanel: { panel: Schema.Struct({ open: Schema.Boolean }) },
})
const NearCommand = Schema.Union([
  FlatCommand,
  Schema.TaggedStruct('Open', { pane: Pane, at: NearDrop }),
  Schema.TaggedStruct('Move', { id: Id, to: NearDrop }),
  Schema.TaggedStruct('FocusGroup', { group: GroupId }),
  Schema.TaggedStruct('SetRatio', { node: between(0, 4), ratio: looseRatio }),
  Schema.TaggedStruct('Receive', { layout: Received }),
])
// Without a floor, generation stays near its small default size and a sequence rarely opens a tab and then acts on it.
const sequences = <A>(item: Schema.Codec<A>) => Arbitrary.array(Arbitrary.schema(item), { minLength: 40, maxLength: 80 })

const invariants = (state: WorkspaceState) => {
  const tabs = tabsOf(state.layout), groups = groupsOf(state.layout)
  const identities = tabs.map(tab => paneIdentity(tab.pane)), tabIds = tabs.map(tab => tab.id), ownGroupIds = groups.map(group => group.id)
  expect(new Set(identities).size, 'one tab per subject across every group').toBe(identities.length)
  expect(new Set(tabIds).size, 'tab ids are unique').toBe(tabIds.length)
  expect(new Set(ownGroupIds).size, 'group ids are unique').toBe(ownGroupIds.length)
  expect(ownGroupIds, 'the focus names a group').toContain(state.focus)
  for (const group of groups) {
    if (group.tabs.length === 0) {
      expect(groups, 'only a lone group may be empty').toHaveLength(1)
      expect(group.active).toBeNull()
    } else expect(group.tabs.map(tab => tab.id), 'a group shows one of its tabs').toContain(group.active)
  }
  const walk = (node: Layout) => {
    if (isGroup(node)) return
    expect(node.children.length, 'a split has children to split').toBeGreaterThanOrEqual(2)
    if (node.ratio !== undefined) {
      expect(node.children).toHaveLength(2)
      expect(node.ratio).toBeGreaterThanOrEqual(0.1)
      expect(node.ratio).toBeLessThanOrEqual(0.9)
    }
    node.children.forEach(walk)
  }
  walk(state.layout)
  expect(new Set(state.mru).size, 'recency has no duplicates').toBe(state.mru.length)
  for (const id of state.mru) expect(tabIds, 'recency only names open tabs').toContain(id)
  for (const id of tabIds) expect(state.mru, 'recency names every open tab').toContain(id)
  for (const id of Object.keys(state.views)) expect(tabIds, 'view state never outlives its tab').toContain(id)
  expect(state.closed.length).toBeLessThanOrEqual(20)
}

const run = (state: WorkspaceState, steps: readonly Command[], context = counter()) => steps.reduce((current, step) => transition(current, step, context), state)
const open = (...list: readonly string[]) => run(emptyWorkspace, list.map(pane => ({ _tag: 'Open', pane }) as const))
const paneList = (state: WorkspaceState) => tabsOf(state.layout).map(tab => tab.pane)
const shape = (state: WorkspaceState) => groupsOf(state.layout).map(group => group.tabs.map(tab => tab.pane))
const tabOf = (state: WorkspaceState, pane: string) => tabsOf(state.layout).find(tab => tab.pane === pane)!

/** Group ids a later step makes must not collide with the tab ids of the fixture's own counter. */
const fresh = () => {
  let next = 100
  return { newId: () => `id-${next++}` }
}

describe('workspace transitions', () => {
  it.prop('keeps its invariants under any command sequence', { sequence: sequences<Command>(NearCommand) }, ({ sequence }) => {
    const context = counter()
    let state: WorkspaceState = emptyWorkspace
    for (const step of sequence) {
      state = transition(state, step, context)
      invariants(state)
    }
  }, { arbitrary: { runs: 500 } })

  it.prop('is total over every command its schema admits', { sequence: Arbitrary.array(Arbitrary.schema(Command), { maxLength: 20 }) }, ({ sequence }) => {
    const context = counter()
    let state: WorkspaceState = emptyWorkspace
    for (const step of sequence) {
      state = transition(state, step, context)
      invariants(state)
    }
  })

  it.prop('restores the invariants on any document the schema admits', { document: WorkspaceState }, ({ document }) => {
    invariants(repairWorkspace(document, counter()))
  })

  it.prop('matches a naive reference model while there is one group', { sequence: sequences<Command>(FlatCommand) }, ({ sequence }) => {
    const context = counter()
    let state: WorkspaceState = emptyWorkspace
    // The reference is a plain list of panes and an active pane: no ids, no recency.
    let list: string[] = []
    let shown: string | null = null
    for (const step of sequence) {
      const before = state
      state = transition(state, step, context)
      const paneOf = (id: string) => tabsOf(before.layout).find(tab => tab.id === id)?.pane
      if (step._tag === 'Open') {
        const key = paneIdentity(step.pane)
        if (!list.includes(key)) list = [...list, key]
        if (step.activate !== false || shown === null) shown = key
      } else if (step._tag === 'Activate') {
        shown = paneOf(step.id) ?? shown
      } else if (step._tag === 'Cycle') {
        if (list.length > 0) shown = list[(Math.max(shown === null ? -1 : list.indexOf(shown), 0) + step.by + list.length) % list.length]!
      } else if (step._tag === 'ActivateIndex') {
        shown = list[step.index] ?? shown
      } else if (step._tag === 'Close') {
        const pane = paneOf(step.id)
        if (pane !== undefined) {
          const index = list.indexOf(pane)
          list = list.filter(other => other !== pane)
          if (shown === pane) shown = list[Math.min(index, list.length - 1)] ?? null
        }
      } else {
        // Resynchronize after commands the reference does not model.
        list = [...paneList(state)]
        shown = activeTab(state)?.pane ?? null
      }
      expect(paneList(state)).toEqual(list)
      expect(activeTab(state)?.pane ?? null).toBe(shown)
    }
  })

  it('returns the same reference when a command does not apply, so the URL adapter cannot loop', () => {
    const context = counter()
    const state = open('agent:agent/a')
    const only = activeTab(state)!.id
    for (const step of [
      { _tag: 'Open', pane: 'agent:agent/a' },
      { _tag: 'Open', pane: '' },
      { _tag: 'Close', id: 'missing' },
      { _tag: 'Move', id: only, to: { _tag: 'Strip', group: 'main', index: 0 } },
      { _tag: 'Move', id: only, to: { _tag: 'Edge', group: 'main', side: 'right', first: false } },
      { _tag: 'Move', id: only, to: { _tag: 'Strip', group: 'missing', index: 0 } },
      { _tag: 'FocusGroup', group: 'main' },
      { _tag: 'SetRatio', node: 0, ratio: 0.5 },
      { _tag: 'SetView', id: 'missing', view: { scrollTop: 1 } },
      { _tag: 'SetPanel', panel: { open: false } },
      { _tag: 'Receive', layout: { tabs: [{ pane: 'agent:agent/a' }] } },
    ] satisfies readonly Command[]) expect(transition(state, step, context), step._tag).toBe(state)
  })

  it('closes like the TUI: the tab now at the index, or the new last tab', () => {
    let state = open('agent:agent/a', 'agent:agent/b', 'list:home')
    state = run(state, [{ _tag: 'ActivateIndex', index: 1 }])
    state = run(state, [{ _tag: 'Close', id: activeTab(state)!.id }])
    expect(activeTab(state)?.pane).toBe('list:home')
    state = run(state, [{ _tag: 'Close', id: activeTab(state)!.id }])
    expect(activeTab(state)?.pane).toBe('agent:agent/a')
    expect(paneList(run(state, [{ _tag: 'ReopenClosed' }], fresh()))).toEqual(['agent:agent/a', 'list:home'])
  })

  it('opening a subject shown in another split focuses it there, with no second tab', () => {
    const context = fresh()
    let state = open('agent:agent/a', 'agent:agent/b')
    state = run(state, [{ _tag: 'Move', id: tabOf(state, 'agent:agent/b').id, to: { _tag: 'Edge', group: 'main', side: 'right', first: false } }], context)
    expect(activeTab(state)?.pane).toBe('agent:agent/b')
    state = run(state, [{ _tag: 'Open', pane: 'agent:agent/a' }], context)
    expect(shape(state)).toEqual([['agent:agent/a'], ['agent:agent/b']])
    expect(activeTab(state)?.pane).toBe('agent:agent/a')
    expect(state.focus).toBe('main')
  })

  it('opens a new subject in the focused split', () => {
    const context = fresh()
    let state = open('agent:agent/a', 'agent:agent/b')
    state = run(state, [{ _tag: 'Move', id: tabOf(state, 'agent:agent/b').id, to: { _tag: 'Edge', group: 'main', side: 'below', first: false } }], context)
    state = run(state, [{ _tag: 'Open', pane: 'list:home' }], context)
    expect(shape(state)).toEqual([['agent:agent/a'], ['agent:agent/b', 'list:home']])
  })

  it('opens behind without taking the focus', () => {
    const state = run(open('agent:agent/a'), [{ _tag: 'Open', pane: 'agent:agent/b', activate: false }], fresh())
    expect(paneList(state)).toEqual(['agent:agent/a', 'agent:agent/b'])
    expect(activeTab(state)?.pane).toBe('agent:agent/a')
  })
})

describe('one tab per agent', () => {
  it('keeps a terminal as a face of the agent\'s tab, however it is reached', () => {
    let state = open('agent:agent/a', 'list:home')
    state = run(state, [{ _tag: 'Open', pane: 'terminal:agent/a' }], fresh())
    expect(paneList(state)).toEqual(['agent:agent/a', 'list:home'])
    const tab = activeTab(state)!
    expect(tab.pane).toBe('agent:agent/a')
    expect(showsTerminal(tab, state.views[tab.id])).toBe(true)
  })

  it('opens a terminal link as the agent\'s tab on its terminal face', () => {
    const state = open('terminal:agent/a')
    const tab = activeTab(state)!
    expect(tab.pane).toBe('agent:agent/a')
    expect(showsTerminal(tab, state.views[tab.id])).toBe(true)
  })

  it('remembers the face per tab across focus, and a sidebar open does not reset it', () => {
    let state = open('agent:agent/a', 'agent:agent/b')
    const a = tabOf(state, 'agent:agent/a').id
    state = run(state, [{ _tag: 'SetView', id: a, view: { terminal: true } }, { _tag: 'Open', pane: 'agent:agent/a' }, { _tag: 'Open', pane: 'agent:agent/b' }], fresh())
    expect(showsTerminal(activeTab(state)!, state.views[activeTab(state)!.id])).toBe(false)
    state = run(state, [{ _tag: 'Open', pane: 'agent:agent/a' }], fresh())
    expect(showsTerminal(activeTab(state)!, state.views[a])).toBe(true)
  })

  it('collapses a glass that holds an agent and its terminal to one tab', () => {
    const state = run(open('list:home'), [{ _tag: 'Receive', layout: { tabs: [{ pane: 'agent:agent/a' }, { pane: 'terminal:agent/a' }, { pane: 'list:home' }] } }], fresh())
    expect(paneList(state)).toEqual(['agent:agent/a', 'list:home'])
  })

  it('keeps a key naming the terminal itself, showing that face until toggled', () => {
    let state = run(emptyWorkspace, [{ _tag: 'Receive', layout: { tabs: [{ pane: 'terminal:agent/a' }] } }])
    const tab = activeTab(state)!
    expect(tab.pane).toBe('terminal:agent/a')
    expect(showsTerminal(tab, state.views[tab.id])).toBe(true)
    state = run(state, [{ _tag: 'SetView', id: tab.id, view: { terminal: false } }])
    expect(showsTerminal(tab, state.views[tab.id])).toBe(false)
    expect(glassBody({ layout: state.layout, name: 'Web' }).layout).toEqual({ tabs: [{ pane: 'terminal:agent/a' }] })
  })
})

describe('the right sidebar', () => {
  it('stays as it is across every tab change', () => {
    let state = open('agent:agent/a', 'agent:agent/b')
    state = run(state, [{ _tag: 'SetPanel', panel: { open: true, tab: 'changes', width: 420 } }])
    const panel = state.panel
    state = run(state, [{ _tag: 'Open', pane: 'agent:agent/a' }, { _tag: 'Cycle', by: 1 }, { _tag: 'Open', pane: 'list:home' }, { _tag: 'Close', id: tabOf(state, 'agent:agent/a').id }], fresh())
    expect(state.panel).toBe(panel)
  })
})

describe('moving tabs', () => {
  const strip = (group: string, index: number): Drop => ({ _tag: 'Strip', group, index })
  const edge = (group: string, side: 'right' | 'below', first = false): Drop => ({ _tag: 'Edge', group, side, first })

  it('reorders within a strip by final index, clamped', () => {
    let state = open('agent:agent/a', 'agent:agent/b', 'list:home')
    state = run(state, [{ _tag: 'Move', id: tabOf(state, 'agent:agent/a').id, to: strip('main', 1) }])
    expect(paneList(state)).toEqual(['agent:agent/b', 'agent:agent/a', 'list:home'])
    state = run(state, [{ _tag: 'Move', id: tabOf(state, 'agent:agent/b').id, to: strip('main', 99) }])
    expect(paneList(state)).toEqual(['agent:agent/a', 'list:home', 'agent:agent/b'])
  })

  it('splits off a tab on any side of a group, as `Layout::split` and `split_first` do', () => {
    const base = open('agent:agent/a', 'agent:agent/b')
    const id = tabOf(base, 'agent:agent/b').id
    const after = run(base, [{ _tag: 'Move', id, to: edge('main', 'right') }], fresh())
    expect(shape(after)).toEqual([['agent:agent/a'], ['agent:agent/b']])
    expect(after.layout).toMatchObject({ split: 'right' })
    const before = run(base, [{ _tag: 'Move', id, to: edge('main', 'below', true) }], fresh())
    expect(shape(before)).toEqual([['agent:agent/b'], ['agent:agent/a']])
    expect(before.layout).toMatchObject({ split: 'below' })
    expect(activeTab(before)?.pane).toBe('agent:agent/b')
  })

  it('removes a group its last tab left, and a split left with one child, keeping the other group\'s identity', () => {
    const context = fresh()
    let state = open('agent:agent/a', 'agent:agent/b')
    state = run(state, [{ _tag: 'Move', id: tabOf(state, 'agent:agent/b').id, to: edge('main', 'right') }], context)
    state = run(state, [{ _tag: 'Move', id: tabOf(state, 'agent:agent/b').id, to: strip('main', 0) }], context)
    expect(isGroup(state.layout)).toBe(true)
    expect(state.focus).toBe('main')
    expect(paneList(state)).toEqual(['agent:agent/b', 'agent:agent/a'])
  })

  it('moves a tab to the edge of another group when its own group empties', () => {
    const context = fresh()
    let state = open('agent:agent/a', 'agent:agent/b', 'list:home')
    state = run(state, [{ _tag: 'Move', id: tabOf(state, 'list:home').id, to: edge('main', 'right') }], context)
    const right = focusedGroup(state)!.id
    state = run(state, [{ _tag: 'Move', id: tabOf(state, 'list:home').id, to: edge('main', 'below') }], context)
    expect(groupsOf(state.layout).map(group => group.id)).not.toContain(right)
    expect(shape(state)).toEqual([['agent:agent/a', 'agent:agent/b'], ['list:home']])
    expect(state.layout).toMatchObject({ split: 'below' })
  })

  it('keeps the source group showing a tab', () => {
    const context = fresh()
    let state = open('agent:agent/a', 'agent:agent/b', 'list:home')
    state = run(state, [{ _tag: 'ActivateIndex', index: 1 }, { _tag: 'Move', id: tabOf(state, 'agent:agent/b').id, to: edge('main', 'right') }], context)
    expect(groupsOf(state.layout)[0]!.active).toBe(tabOf(state, 'list:home').id)
  })

  it('opens a dragged-in subject at its drop, or moves its existing tab there', () => {
    const context = fresh()
    let state = open('agent:agent/a')
    state = run(state, [{ _tag: 'Open', pane: 'agent:agent/b', at: strip('main', 0) }], context)
    expect(paneList(state)).toEqual(['agent:agent/b', 'agent:agent/a'])
    state = run(state, [{ _tag: 'Open', pane: 'list:home', at: edge('main', 'right') }], context)
    expect(shape(state)).toEqual([['agent:agent/b', 'agent:agent/a'], ['list:home']])
    state = run(state, [{ _tag: 'Open', pane: 'agent:agent/a', at: { _tag: 'Strip', group: state.focus, index: 0 } }], context)
    expect(shape(state)).toEqual([['agent:agent/b'], ['agent:agent/a', 'list:home']])
  })

  it('fills the empty workspace when something is dropped on its edge', () => {
    const state = run(emptyWorkspace, [{ _tag: 'Open', pane: 'agent:agent/a', at: edge('main', 'right') }])
    expect(state.layout).toMatchObject({ id: 'main' })
    expect(paneList(state)).toEqual(['agent:agent/a'])
  })
})

describe('closing in splits', () => {
  it('removes an emptied group and moves focus to the group before it', () => {
    const context = fresh()
    let state = open('agent:agent/a', 'agent:agent/b', 'list:home')
    state = run(state, [{ _tag: 'Move', id: tabOf(state, 'agent:agent/b').id, to: { _tag: 'Edge', group: 'main', side: 'right', first: false } }], context)
    const right = state.focus
    state = run(state, [{ _tag: 'Close', id: tabOf(state, 'agent:agent/b').id }], context)
    expect(groupsOf(state.layout).map(group => group.id)).toEqual(['main'])
    expect(state.focus).toBe('main')
    expect(right).not.toBe('main')
  })

  it('leaves the focus alone when closing in another group', () => {
    const context = fresh()
    let state = open('agent:agent/a', 'agent:agent/b')
    state = run(state, [{ _tag: 'Move', id: tabOf(state, 'agent:agent/b').id, to: { _tag: 'Edge', group: 'main', side: 'right', first: false } }], context)
    const right = state.focus
    state = run(state, [{ _tag: 'Close', id: tabOf(state, 'agent:agent/a').id }], context)
    expect(state.focus).toBe(right)
    expect(activeTab(state)?.pane).toBe('agent:agent/b')
  })
})

describe('the glass contract', () => {
  const wire = (...list: readonly string[]): GlassLayout => ({ tabs: list.map(pane => ({ pane })) })

  it('sends only structure, with a longer split nested two at a time', () => {
    const layout: Layout = {
      split: 'right',
      ratio: 0.3,
      children: [
        { id: 'one', tabs: [{ id: 'a', pane: 'agent:agent/a', title: 'Mine' }], active: 'a' },
        { id: 'two', tabs: [{ id: 'b', pane: 'list:home' }], active: 'b' },
        { id: 'three', tabs: [{ id: 'c', pane: 'future-kind:x' }], active: 'c' },
      ],
    }
    expect(glassBody({ layout, name: 'Web' })).toEqual({
      name: 'Web',
      layout: { split: 'right', children: [{ tabs: [{ pane: 'agent:agent/a', title: 'Mine' }] }, { split: 'right', children: [wire('list:home'), wire('future-kind:x')] }] },
    })
  })

  it('carries a two-way ratio only for a gateway that takes one', () => {
    const layout: Layout = { split: 'below', ratio: 0.3, children: [{ id: 'one', tabs: [], active: null }, { id: 'two', tabs: [], active: null }] }
    expect(glassBody({ layout, name: 'Web' }).layout).not.toHaveProperty('ratio')
    expect(glassBody({ layout, name: 'Web', ratios: true }).layout).toMatchObject({ ratio: 0.3 })
  })

  it.prop('round trips any layout through the wire, unknown keys included', { layout: Received }, ({ layout }) => {
    const context = counter()
    const first = transition(emptyWorkspace, { _tag: 'Receive', layout }, context)
    const body = glassBody({ layout: first.layout, name: 'Web' })
    const second = transition(emptyWorkspace, { _tag: 'Receive', layout: body.layout as ReceivedLayout }, context)
    expect(glassBody({ layout: second.layout, name: 'Web' })).toEqual(body)
  })

  it('adopts the other side\'s structure, keeping this device\'s ids, active tabs and focus where they apply', () => {
    const context = fresh()
    let state = open('agent:agent/a', 'agent:agent/b')
    const [a, b] = tabsOf(state.layout)
    state = run(state, [{ _tag: 'SetView', id: b!.id, view: { scrollTop: 9 } }], context)
    state = transition(state, { _tag: 'Receive', layout: { split: 'right', children: [{ tabs: [{ pane: 'future-kind:x' }] }, { tabs: [{ pane: 'agent:agent/b' }, { pane: 'agent:agent/a' }] }] } }, context)
    expect(shape(state)).toEqual([['future-kind:x'], ['agent:agent/b', 'agent:agent/a']])
    expect(tabOf(state, 'agent:agent/a').id).toBe(a!.id)
    expect(tabOf(state, 'agent:agent/b').id).toBe(b!.id)
    expect(activeTab(state)?.id).toBe(b!.id)
    expect(state.views[b!.id]?.scrollTop).toBe(9)
    invariants(state)
  })

  it('moves the shown tab to what took its place when the other side closed it', () => {
    const context = fresh()
    let state = open('agent:agent/a', 'agent:agent/b', 'list:home')
    state = run(state, [{ _tag: 'ActivateIndex', index: 1 }], context)
    state = transition(state, { _tag: 'Receive', layout: { tabs: [{ pane: 'agent:agent/a' }, { pane: 'list:home' }] } }, context)
    expect(activeTab(state)?.pane).toBe('list:home')
  })

  it('repairs what a stored document cannot be trusted for', () => {
    const state: WorkspaceState = {
      ...emptyWorkspace,
      layout: { split: 'right', children: [{ id: 'one', tabs: [], active: null }, { split: 'below', children: [{ id: 'two', tabs: [{ id: 't', pane: 'list:home' }], active: 'x' }] }] },
      focus: 'one',
      mru: ['ghost'],
      views: { ghost: {} },
    }
    const repaired = repairWorkspace(state, counter())
    expect(repaired.layout).toEqual({ id: 'two', tabs: [{ id: 't', pane: 'list:home' }], active: 't' })
    invariants(repaired)
  })
})
