import { Arbitrary, Option, Schema } from 'effect'
import { describe, expect, it } from 'vitest'
import { prop } from './testing.ts'
import {
  activeTab, adoptLayout, canonicalBody, currentSpace, decodeWorkspace, emptyWorkspace, focusedPane, isPane, legacyDecoder,
  makeWorkspaceSchema, panesOf, repairWorkspace, shownPanes, spaceBody, tabsOf, toBinary, toWire, transition, unusedName,
  type Command, type Drop, type Layout, type Space, type WireLayout, type WorkspaceConfig, type WorkspaceState,
} from './mod.ts'

interface View {
  readonly scrollTop?: number | undefined
  readonly face?: string | undefined
}
const View = Schema.Struct({ scrollTop: Schema.optional(Schema.Finite), face: Schema.optional(Schema.String) })
const WorkspaceSchema = makeWorkspaceSchema(View)

/** `alias:x` and `doc:x` are one entity, as a host's two keys for one thing can be. */
const identity = (ref: string) => ref.startsWith('alias:') ? `doc:${ref.slice('alias:'.length)}` : ref

const counter = (start = 0): WorkspaceConfig => {
  let next = start
  return { newId: () => `id-${next++}`, identity }
}

const between = (minimum: number, maximum: number) => Schema.Int.check(Schema.isBetween({ minimum, maximum }))
const looseSize = Schema.Finite.check(Schema.isBetween({ minimum: -1, maximum: 2 }))

const Ref = Schema.Literals(['doc:a', 'doc:b', 'doc:c', 'alias:a', 'alias:c', 'list:home', 'list:all', 'note:x', 'note:y', 'future:z', ''])
const Id = Schema.Literals(Array.from({ length: 24 }, (_, n) => `id-${n}`))
const SideS = Schema.Literals(['left', 'right', 'top', 'bottom'])
const NearDrop = Schema.TaggedUnion({
  Strip: { pane: Id, index: between(-2, 12) },
  Center: { pane: Id },
  Edge: { pane: Id, side: SideS },
})
const Received: Schema.Codec<WireLayout> = Schema.Union([
  Schema.Struct({ tabs: Schema.Array(Schema.Struct({ ref: Ref })).check(Schema.isMaxLength(4)) }),
  Schema.Struct({
    axis: Schema.Literals(['row', 'column']),
    children: Schema.Array(Schema.suspend((): Schema.Codec<WireLayout> => Received)).check(Schema.isMaxLength(3)),
    sizes: Schema.optional(Schema.Array(looseSize).check(Schema.isMaxLength(3))),
  }),
])
const NearCommand = Schema.Union([
  Schema.TaggedStruct('Open', { ref: Ref, activate: Schema.Boolean }),
  Schema.TaggedStruct('Open', { ref: Ref, at: NearDrop }),
  Schema.TaggedStruct('Activate', { tab: Id }),
  Schema.TaggedStruct('FocusPane', { pane: Id }),
  Schema.TaggedStruct('FocusDirection', { direction: Schema.Literals(['left', 'right', 'up', 'down']) }),
  Schema.TaggedStruct('Cycle', { by: Schema.Literals([1, -1]) }),
  Schema.TaggedStruct('ActivateIndex', { index: between(-1, 9) }),
  Schema.TaggedStruct('ActivatePrevious', {}),
  Schema.TaggedStruct('Close', { tab: Id }),
  Schema.TaggedStruct('CloseOthers', { tab: Id }),
  Schema.TaggedStruct('Replace', { tab: Id, ref: Ref }),
  Schema.TaggedStruct('ReopenClosed', {}),
  Schema.TaggedStruct('Move', { tab: Id, to: NearDrop }),
  Schema.TaggedStruct('Split', { side: SideS, ref: Schema.optional(Ref), tab: Schema.optional(Id) }),
  Schema.TaggedStruct('Resize', { split: Id, sizes: Schema.Array(looseSize).check(Schema.isMaxLength(3)) }),
  Schema.TaggedStruct('ResetSizes', { split: Schema.optional(Id) }),
  Schema.TaggedStruct('ToggleZoom', {}),
  Schema.TaggedStruct('SetAside', { pane: Id, aside: Schema.Struct({ size: Schema.optional(between(0, 900)), explicit: Schema.optional(Schema.NullOr(Schema.Struct({ open: Schema.Boolean, band: Schema.Literals(['wide', 'narrow']) }))) }) }),
  Schema.TaggedStruct('SetView', { tab: Id, view: Schema.Struct({ scrollTop: between(0, 1000) }) }),
  Schema.TaggedStruct('SpaceNew', {}),
  Schema.TaggedStruct('SpaceSwitch', { space: Id }),
  Schema.TaggedStruct('SpaceDuplicate', { space: Id }),
  Schema.TaggedStruct('SpaceClose', { space: Id }),
  Schema.TaggedStruct('Receive', { space: Id, body: Schema.Struct({ name: Schema.String, layout: Received }), sizes: Schema.Boolean }),
])
// Without a floor, generation stays small and a sequence rarely opens a tab and then acts on it.
const sequences = <A>(item: Schema.Codec<A>) => Arbitrary.array(Arbitrary.schema(item), { minLength: 40, maxLength: 80 })

const spaceInvariants = (space: Space<View>) => {
  const tabs = tabsOf(space.layout), panes = panesOf(space.layout)
  const identities = tabs.map(tab => identity(tab.ref)), tabIds = tabs.map(tab => tab.id), paneIds = panes.map(pane => pane.id)
  expect(new Set(identities).size, 'one tab per entity in a space').toBe(identities.length)
  expect(new Set(tabIds).size, 'tab ids are unique').toBe(tabIds.length)
  expect(new Set(paneIds).size, 'pane ids are unique').toBe(paneIds.length)
  expect(paneIds, 'the focus names a pane').toContain(space.local.focus)
  if (space.local.zoomed !== null) {
    expect(paneIds).toContain(space.local.zoomed)
    expect(panes.length, 'only a split layout zooms').toBeGreaterThan(1)
  }
  for (const pane of panes) {
    if (pane.tabs.length === 0) expect(pane.active).toBeNull()
    else expect(pane.tabs.map(tab => tab.id), 'a pane shows one of its tabs').toContain(pane.active)
  }
  const walk = (node: Layout) => {
    if (isPane(node)) return
    expect(node.children.length, 'a split has children to split').toBeGreaterThanOrEqual(2)
    if (node.sizes !== undefined) {
      expect(node.sizes).toHaveLength(node.children.length)
      expect(node.sizes.reduce((sum, size) => sum + size, 0)).toBeCloseTo(1)
    }
    node.children.forEach(walk)
  }
  walk(space.layout)
  expect(new Set(space.local.mru).size, 'recency has no duplicates').toBe(space.local.mru.length)
  for (const id of space.local.mru) expect(tabIds, 'recency only names open tabs').toContain(id)
  for (const id of tabIds) expect(space.local.mru, 'recency names every open tab').toContain(id)
  for (const id of Object.keys(space.local.views)) expect(tabIds, 'view state never outlives its tab').toContain(id)
  for (const id of Object.keys(space.local.panes)) expect(paneIds, 'pane state never outlives its pane').toContain(id)
  expect(space.local.closed.length).toBeLessThanOrEqual(20)
}

const invariants = (state: WorkspaceState<View>) => {
  expect(state.spaces.length).toBeGreaterThan(0)
  expect(new Set(state.spaces.map(space => space.id)).size).toBe(state.spaces.length)
  expect(state.spaces.map(space => space.id)).toContain(state.current)
  state.spaces.forEach(spaceInvariants)
}

const start = (config = counter()) => emptyWorkspace<View>(config, { id: 'space', name: 'Main' })
const run = (state: WorkspaceState<View>, steps: readonly Command<View>[], config = counter(100)) =>
  steps.reduce((current, step) => transition(current, step, config), state)
const open = (...list: readonly string[]) => run(start(), list.map(ref => ({ _tag: 'Open', ref }) as const), counter(1))
const space = (state: WorkspaceState<View>) => currentSpace(state)
const refs = (state: WorkspaceState<View>) => tabsOf(space(state).layout).map(tab => tab.ref)
const shape = (state: WorkspaceState<View>) => panesOf(space(state).layout).map(pane => pane.tabs.map(tab => tab.ref))
const tabOf = (state: WorkspaceState<View>, ref: string) => tabsOf(space(state).layout).find(tab => tab.ref === ref)!
const focus = (state: WorkspaceState<View>) => space(state).local.focus
const root = 'id-0'

describe('workspace transitions', () => {
  prop('keeps its invariants under any command sequence', sequences<Command<View>>(NearCommand as Schema.Codec<Command<View>>), sequence => {
    const config = counter(1)
    let state = start(counter())
    for (const step of sequence) {
      state = transition(state, step, config)
      invariants(state)
    }
  }, { runs: 400 })

  prop('restores the invariants on any document the schema admits', Arbitrary.schema(WorkspaceSchema), document => {
    if (document.spaces.length === 0) return
    invariants(repairWorkspace(document, counter(1000)))
  })

  it('returns the same reference when a command does not apply', () => {
    const state = open('doc:a')
    const only = activeTab(space(state))!.id
    for (const step of [
      { _tag: 'Open', ref: 'doc:a' },
      { _tag: 'Open', ref: 'alias:a' },
      { _tag: 'Open', ref: '' },
      { _tag: 'Close', tab: 'missing' },
      { _tag: 'Move', tab: only, to: { _tag: 'Strip', pane: root, index: 0 } },
      { _tag: 'Move', tab: only, to: { _tag: 'Center', pane: root } },
      { _tag: 'Move', tab: only, to: { _tag: 'Edge', pane: root, side: 'right' } },
      { _tag: 'Move', tab: only, to: { _tag: 'Strip', pane: 'missing', index: 0 } },
      { _tag: 'FocusPane', pane: root },
      { _tag: 'FocusDirection', direction: 'left' },
      { _tag: 'ToggleZoom' },
      { _tag: 'ResetSizes' },
      { _tag: 'Resize', split: 'missing', sizes: [0.5, 0.5] },
      { _tag: 'SetView', tab: 'missing', view: { scrollTop: 1 } },
      { _tag: 'SpaceSwitch', space: 'space' },
      { _tag: 'SpaceClose', space: 'space' },
      { _tag: 'SpaceRename', space: 'space', name: ' Main ' },
      { _tag: 'Receive', space: 'space', body: { name: 'Main', layout: { tabs: [{ ref: 'doc:a' }] } } },
    ] satisfies readonly Command<View>[]) expect(transition(state, step, counter(100)), step._tag).toBe(state)
  })

  it('closes like a strip does: the tab now at the index, or the new last tab', () => {
    let state = open('doc:a', 'doc:b', 'list:home')
    state = run(state, [{ _tag: 'ActivateIndex', index: 1 }])
    state = run(state, [{ _tag: 'Close', tab: activeTab(space(state))!.id }])
    expect(activeTab(space(state))?.ref).toBe('list:home')
    state = run(state, [{ _tag: 'Close', tab: activeTab(space(state))!.id }])
    expect(activeTab(space(state))?.ref).toBe('doc:a')
    expect(refs(run(state, [{ _tag: 'ReopenClosed' }]))).toEqual(['doc:a', 'list:home'])
  })

  it('focuses an entity\'s existing tab wherever it is, with no second tab', () => {
    let state = open('doc:a', 'doc:b')
    state = run(state, [{ _tag: 'Move', tab: tabOf(state, 'doc:b').id, to: { _tag: 'Edge', pane: root, side: 'right' } }])
    expect(activeTab(space(state))?.ref).toBe('doc:b')
    state = run(state, [{ _tag: 'Open', ref: 'alias:a' }], counter(200))
    expect(shape(state)).toEqual([['doc:a'], ['doc:b']])
    expect(focus(state)).toBe(root)
  })

  it('opens behind without taking the focus', () => {
    const state = run(open('doc:a'), [{ _tag: 'Open', ref: 'doc:b', activate: false }])
    expect(refs(state)).toEqual(['doc:a', 'doc:b'])
    expect(activeTab(space(state))?.ref).toBe('doc:a')
  })

  it('closes others, remembering them, and replaces in place', () => {
    let state = open('doc:a', 'doc:b', 'list:home')
    state = run(state, [{ _tag: 'CloseOthers', tab: tabOf(state, 'doc:b').id }])
    expect(refs(state)).toEqual(['doc:b'])
    expect(space(state).local.closed).toHaveLength(2)
    const replaced = tabOf(state, 'doc:b').id
    state = run(state, [{ _tag: 'SetView', tab: replaced, view: { scrollTop: 5 } }, { _tag: 'Replace', tab: replaced, ref: 'note:x' }])
    expect(refs(state)).toEqual(['note:x'])
    expect(space(state).local.views[replaced]).toBeUndefined()
  })
})

describe('moving and splitting', () => {
  const strip = (pane: string, index: number): Drop => ({ _tag: 'Strip', pane, index })
  const edge = (pane: string, side: 'left' | 'right' | 'top' | 'bottom'): Drop => ({ _tag: 'Edge', pane, side })

  it('reorders within a strip by final index, clamped', () => {
    let state = open('doc:a', 'doc:b', 'list:home')
    state = run(state, [{ _tag: 'Move', tab: tabOf(state, 'doc:a').id, to: strip(root, 1) }])
    expect(refs(state)).toEqual(['doc:b', 'doc:a', 'list:home'])
    state = run(state, [{ _tag: 'Move', tab: tabOf(state, 'doc:b').id, to: strip(root, 99) }])
    expect(refs(state)).toEqual(['doc:a', 'list:home', 'doc:b'])
  })

  it('splits off a tab on any side of a pane', () => {
    const base = open('doc:a', 'doc:b')
    const id = tabOf(base, 'doc:b').id
    const right = run(base, [{ _tag: 'Move', tab: id, to: edge(root, 'right') }])
    expect(shape(right)).toEqual([['doc:a'], ['doc:b']])
    expect(space(right).layout).toMatchObject({ axis: 'row' })
    const top = run(base, [{ _tag: 'Move', tab: id, to: edge(root, 'top') }])
    expect(shape(top)).toEqual([['doc:b'], ['doc:a']])
    expect(space(top).layout).toMatchObject({ axis: 'column' })
  })

  it('collapses a pane its last tab left, and a one-child split, keeping the other pane\'s id', () => {
    let state = open('doc:a', 'doc:b')
    state = run(state, [{ _tag: 'Move', tab: tabOf(state, 'doc:b').id, to: edge(root, 'right') }])
    state = run(state, [{ _tag: 'Move', tab: tabOf(state, 'doc:b').id, to: strip(root, 0) }])
    expect(isPane(space(state).layout)).toBe(true)
    expect(focus(state)).toBe(root)
    expect(refs(state)).toEqual(['doc:b', 'doc:a'])
  })

  it('opens a dropped entity at its drop, or moves its existing tab there', () => {
    let state = open('doc:a')
    state = run(state, [{ _tag: 'Open', ref: 'doc:b', at: strip(root, 0) }])
    expect(refs(state)).toEqual(['doc:b', 'doc:a'])
    state = run(state, [{ _tag: 'Open', ref: 'list:home', at: edge(root, 'right') }])
    expect(shape(state)).toEqual([['doc:b', 'doc:a'], ['list:home']])
    state = run(state, [{ _tag: 'Open', ref: 'alias:a', at: { _tag: 'Center', pane: focus(state) } }], counter(300))
    expect(shape(state)).toEqual([['doc:b'], ['list:home', 'doc:a']])
  })

  it('fills an empty pane when something lands on its edge', () => {
    const state = run(start(), [{ _tag: 'Open', ref: 'doc:a', at: edge(root, 'right') }])
    expect(space(state).layout).toMatchObject({ id: root })
    expect(refs(state)).toEqual(['doc:a'])
  })

  it('splits the focused pane empty, with a ref, or with a tab', () => {
    let state = open('doc:a', 'doc:b')
    state = run(state, [{ _tag: 'Split', side: 'right' }])
    expect(shape(state)).toEqual([['doc:a', 'doc:b'], []])
    expect(focusedPane(space(state))!.tabs).toEqual([])
    state = run(state, [{ _tag: 'Split', side: 'bottom', ref: 'note:x' }], counter(400))
    // An empty pane takes the ref itself.
    expect(shape(state)).toEqual([['doc:a', 'doc:b'], ['note:x']])
    state = run(state, [{ _tag: 'Split', side: 'bottom', pane: root, tab: tabOf(state, 'doc:b').id }], counter(500))
    expect(shape(state)).toEqual([['doc:a'], ['doc:b'], ['note:x']])
  })

  it('closing the last tab of a pane collapses it and focuses the pane before', () => {
    let state = open('doc:a', 'doc:b')
    state = run(state, [{ _tag: 'Move', tab: tabOf(state, 'doc:b').id, to: edge(root, 'right') }])
    state = run(state, [{ _tag: 'Close', tab: tabOf(state, 'doc:b').id }])
    expect(panesOf(space(state).layout).map(pane => pane.id)).toEqual([root])
    expect(focus(state)).toBe(root)
  })
})

describe('sizes, focus by direction and zoom', () => {
  const grid = () => {
    let state = open('doc:a', 'doc:b', 'doc:c')
    state = run(state, [{ _tag: 'Move', tab: tabOf(state, 'doc:b').id, to: { _tag: 'Edge', pane: root, side: 'right' } }])
    const right = focus(state)
    state = run(state, [{ _tag: 'Move', tab: tabOf(state, 'doc:c').id, to: { _tag: 'Edge', pane: right, side: 'bottom' } }], counter(200))
    return { state, right, below: focus(state) }
  }

  it('moves focus to the nearest pane on that side, and nowhere at an edge', () => {
    const { state, right, below } = grid()
    expect(focus(run(state, [{ _tag: 'FocusDirection', direction: 'up' }]))).toBe(right)
    expect(focus(run(state, [{ _tag: 'FocusDirection', direction: 'left' }]))).toBe(root)
    expect(run(state, [{ _tag: 'FocusDirection', direction: 'right' }])).toBe(state)
    const left = run(state, [{ _tag: 'FocusDirection', direction: 'left' }])
    expect(run(left, [{ _tag: 'FocusDirection', direction: 'left' }])).toBe(left)
    expect(focus(run(left, [{ _tag: 'FocusDirection', direction: 'right' }]))).not.toBe(root)
    expect(below).not.toBe(right)
  })

  it('zooms the focused pane, and a move or a split brings the layout back', () => {
    const { state } = grid()
    const zoomed = run(state, [{ _tag: 'ToggleZoom' }])
    expect(shownPanes(space(zoomed)).map(pane => pane.id)).toEqual([focus(state)])
    const back = run(zoomed, [{ _tag: 'FocusDirection', direction: 'left' }])
    expect(space(back).local.zoomed).toBeNull()
    expect(focus(back)).toBe(focus(state))
    expect(space(run(zoomed, [{ _tag: 'ToggleZoom' }])).local.zoomed).toBeNull()
    expect(space(run(zoomed, [{ _tag: 'Split', side: 'right' }])).local.zoomed).toBeNull()
  })

  it('resizes a split by its id, normalised, and resets to equal shares', () => {
    const { state } = grid()
    const split = space(state).layout as Extract<Layout, { axis: string }>
    const resized = run(state, [{ _tag: 'Resize', split: split.id, sizes: [3, 1] }])
    expect((space(resized).layout as typeof split).sizes).toEqual([0.75, 0.25])
    expect(run(resized, [{ _tag: 'Resize', split: split.id, sizes: [0.75, 0.25] }])).toBe(resized)
    expect((space(run(resized, [{ _tag: 'ResetSizes' }])).layout as typeof split).sizes).toBeUndefined()
  })
})

describe('spaces', () => {
  it('makes, renames, duplicates, switches and closes named spaces', () => {
    let state = open('doc:a')
    state = run(state, [{ _tag: 'SpaceNew' }])
    expect(state.spaces.map(row => row.name)).toEqual(['Main', 'Space'])
    expect(refs(state)).toEqual([])
    state = run(state, [{ _tag: 'SpaceDuplicate', space: 'space' }], counter(300))
    expect(state.spaces.map(row => row.name)).toEqual(['Main', 'Space', 'Main 2'])
    expect(refs(state)).toEqual(['doc:a'])
    expect(tabOf(state, 'doc:a').id).not.toBe(tabOf(open('doc:a'), 'doc:a').id)
    state = run(state, [{ _tag: 'SpaceRename', space: state.current, name: 'Review' }, { _tag: 'SpaceSwitch', space: 'space' }])
    expect(space(state).name).toBe('Main')
    state = run(state, [{ _tag: 'SpaceClose', space: 'space' }])
    expect(state.spaces.map(row => row.name)).toEqual(['Space', 'Review'])
    expect(state.spaces.map(row => row.id)).toContain(state.current)
  })

  it('numbers a taken name', () => {
    expect(unusedName(['Main', 'Main 2'], 'Main')).toBe('Main 3')
    expect(unusedName([], 'Main')).toBe('Main')
  })

  it('keeps one tab per entity per space, not across spaces', () => {
    let state = open('doc:a')
    state = run(state, [{ _tag: 'SpaceNew' }, { _tag: 'Open', ref: 'doc:a' }])
    expect(refs(state)).toEqual(['doc:a'])
    expect(state.spaces).toHaveLength(2)
  })
})

describe('the shared body', () => {
  const wire = (...list: readonly string[]): WireLayout => ({ tabs: list.map(ref => ({ ref })) })

  it('sends only structure, sizes on request, empty panes included', () => {
    const layout: Layout = {
      id: 's', axis: 'row', sizes: [0.2, 0.3, 0.5],
      children: [
        { id: 'one', tabs: [{ id: 'a', ref: 'doc:a', title: 'Mine' }], active: 'a' },
        { id: 'two', tabs: [], active: null },
        { id: 'three', tabs: [{ id: 'c', ref: 'future:z' }], active: 'c' },
      ],
    }
    expect(toWire(layout, { sizes: false })).toEqual({ axis: 'row', children: [{ tabs: [{ ref: 'doc:a', title: 'Mine' }] }, wire(), wire('future:z')] })
    expect(toWire(layout, { sizes: true })).toMatchObject({ sizes: [0.2, 0.3, 0.5] })
  })

  it('nests longer splits two at a time for binary stores, keeping shares', () => {
    const binary = toBinary({ axis: 'row', sizes: [0.2, 0.3, 0.5], children: [wire('a'), wire('b'), wire('c')] })
    expect(binary).toMatchObject({ axis: 'row', children: [wire('a'), { axis: 'row', children: [wire('b'), wire('c')] }] })
    const sizes = (node: WireLayout) => 'axis' in node ? node.sizes!.map(size => Math.round(size * 1000) / 1000) : []
    expect(sizes(binary)).toEqual([0.2, 0.8])
    expect(sizes((binary as Extract<WireLayout, { axis: string }>).children[1]!)).toEqual([0.375, 0.625])
  })

  prop('round trips any body, unknown refs and empty panes included', Arbitrary.schema(Received), layout => {
    const first = run(start(), [{ _tag: 'Receive', space: 'space', body: { name: 'Main', layout } }])
    const body = spaceBody(space(first), { sizes: true })
    const second = run(start(), [{ _tag: 'Receive', space: 'space', body }], counter(500))
    expect(canonicalBody(spaceBody(space(second), { sizes: true }))).toBe(canonicalBody(body))
  })

  it('adopts another writer\'s structure, keeping this client\'s ids, shown tabs, views and focus', () => {
    let state = open('doc:a', 'doc:b')
    const [a, b] = tabsOf(space(state).layout)
    state = run(state, [{ _tag: 'SetView', tab: b!.id, view: { scrollTop: 9 } }])
    state = run(state, [{ _tag: 'Receive', space: 'space', body: { name: 'Main', layout: { axis: 'row', children: [wire('future:z'), wire('doc:b', 'alias:a')] } } }])
    expect(shape(state)).toEqual([['future:z'], ['doc:b', 'alias:a']])
    expect(tabOf(state, 'alias:a').id).toBe(a!.id)
    expect(tabOf(state, 'doc:b').id).toBe(b!.id)
    expect(activeTab(space(state))?.id).toBe(b!.id)
    expect(space(state).local.views[b!.id]?.scrollTop).toBe(9)
    invariants(state)
  })

  it('keeps a received empty pane, and local sizes when sizes do not sync', () => {
    let state = open('doc:a', 'doc:b')
    state = run(state, [{ _tag: 'Split', side: 'right' }])
    const split = space(state).layout as Extract<Layout, { axis: string }>
    state = run(state, [{ _tag: 'Resize', split: split.id, sizes: [0.3, 0.7] }])
    const remote = { name: 'Main', layout: { axis: 'row' as const, children: [wire('doc:a', 'doc:b'), wire()] } }
    expect(run(state, [{ _tag: 'Receive', space: 'space', body: remote, sizes: false }])).toBe(state)
    const adopted = run(state, [{ _tag: 'Receive', space: 'space', body: remote, sizes: true }])
    expect((space(adopted).layout as typeof split).sizes).toBeUndefined()
    expect(shape(adopted)).toEqual([['doc:a', 'doc:b'], []])
  })

  it('creates a space it has not seen, and retires and restores a removed one', () => {
    let state = open('doc:a')
    state = run(state, [{ _tag: 'Receive', space: 'other', body: { name: 'Elsewhere', layout: wire('note:x') }, revision: 'r1' }])
    expect(state.spaces.map(row => [row.id, row.name, row.revision])).toEqual([['space', 'Main', undefined], ['other', 'Elsewhere', 'r1']])
    expect(state.current).toBe('space')
    state = run(state, [{ _tag: 'SpaceSwitch', space: 'other' }, { _tag: 'SetView', tab: activeTab(space(state.spaces[1] === undefined ? state : { ...state, current: 'other' }))!.id, view: { scrollTop: 4 } }])
    const viewed = activeTab(space(state))!.id
    state = run(state, [{ _tag: 'Receive', space: 'other', removed: true }])
    expect(state.spaces.map(row => row.id)).toEqual(['space'])
    expect(state.current).toBe('space')
    state = run(state, [{ _tag: 'Receive', space: 'other', body: { name: 'Elsewhere', layout: wire('note:x') } }])
    expect(state.spaces.find(row => row.id === 'other')?.local.views[viewed]?.scrollTop).toBe(4)
  })

  it('records a revision without treating it as a change of structure', () => {
    const state = open('doc:a')
    const revised = run(state, [{ _tag: 'Receive', space: 'space', body: spaceBody(space(state), { sizes: true }), revision: 'r2' }])
    expect(space(revised).revision).toBe('r2')
    expect(space(revised).layout).toBe(space(state).layout)
  })

  it('re-keys by identity across adoptions with no local ids to keep', () => {
    const layout = adoptLayout({ id: 'p', tabs: [], active: null }, wire('doc:a', 'alias:a', 'doc:b'), { identity, newId: counter(1).newId, sizes: true })
    expect(tabsOf(layout).map(tab => tab.ref)).toEqual(['doc:a', 'doc:b'])
    expect(layout).toMatchObject({ id: 'p' })
  })
})

describe('the persisted document', () => {
  const LegacyV0 = Schema.Struct({ tabs: Schema.Array(Schema.String) })

  it('decodes the current version, repaired', () => {
    const state = open('doc:a', 'doc:b')
    const encoded = Schema.encodeSync(WorkspaceSchema)(state)
    const decoded = decodeWorkspace(JSON.parse(JSON.stringify(encoded)), { schema: WorkspaceSchema, config: counter(900) })
    expect(Option.getOrThrow(decoded)).toEqual(state)
  })

  it('migrates a legacy document through a host decoder, and refuses anything else', () => {
    const legacy = [legacyDecoder(LegacyV0, (old): WorkspaceState<View> => old.tabs.reduce(
      (state, ref) => transition(state, { _tag: 'Open', ref }, counter(50)),
      emptyWorkspace<View>(counter(40), { id: 'legacy', name: 'Default' }),
    ))]
    const migrated = decodeWorkspace({ tabs: ['doc:a', 'note:x'] }, { schema: WorkspaceSchema, config: counter(900), legacy })
    expect(Option.map(migrated, refs)).toEqual(Option.some(['doc:a', 'note:x']))
    expect(decodeWorkspace({ nonsense: true }, { schema: WorkspaceSchema, config: counter(900), legacy })).toEqual(Option.none())
  })
})
