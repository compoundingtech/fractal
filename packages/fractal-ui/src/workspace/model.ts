import { Schema } from 'effect'
import { neighbour, paneRects, type Direction } from './geometry.ts'
import {
  findPane, isPane, locateTab, mapPane, mapSplit, normalizeSizes, panesOf, removePane, splitsOf, tabsOf,
  type Layout, type Pane, type Tab,
} from './layout.ts'
import { adoptLayout, canonicalBody, toWire, type SpaceBody } from './wire.ts'

/**
 * The workspace as pure data: named spaces, each a layout tree plus what is this client's alone (focus,
 * zoom, recency, per-tab views, per-pane aside state). `transition` is total: a command that does not
 * apply returns the same state reference.
 */

export const Band = Schema.Literals(['wide', 'narrow'])
export type Band = typeof Band.Type

export const AsideState = Schema.Struct({
  /** A person's explicit open or close, and the width band it was made in. */
  explicit: Schema.optional(Schema.Struct({ open: Schema.Boolean, band: Band })),
  size: Schema.optional(Schema.Finite),
  /** The aside's own selected tab, if the host has several. */
  tab: Schema.optional(Schema.String),
})
export type AsideState = typeof AsideState.Type

export const PaneLocal = Schema.Struct({ aside: Schema.optional(AsideState) })
export type PaneLocal = typeof PaneLocal.Type

export interface SpaceLocal<V> {
  /** The pane keys and the host act on. */
  readonly focus: string
  readonly zoomed: string | null
  /** Tab ids, most recently active first. */
  readonly mru: readonly string[]
  /** Refs, most recently closed first. */
  readonly closed: readonly string[]
  /** Host view state by tab id. */
  readonly views: Readonly<Record<string, V>>
  readonly panes: Readonly<Record<string, PaneLocal>>
}

export interface Space<V> {
  /** Stable across renames: what an external store keys the space by. */
  readonly id: string
  readonly name: string
  readonly layout: Layout
  /** The external store's opaque revision this space last matched. */
  readonly revision?: string | undefined
  readonly local: SpaceLocal<V>
}

export interface RetiredSpace<V> {
  readonly id: string
  readonly local: SpaceLocal<V>
}

export interface WorkspaceState<V = unknown> {
  readonly version: 1
  readonly current: string
  readonly spaces: readonly Space<V>[]
  /** Local views of spaces another writer removed, in case they return. Most recent first. */
  readonly retired: readonly RetiredSpace<V>[]
}

export type Side = 'left' | 'right' | 'top' | 'bottom'
export const SideSchema = Schema.Literals(['left', 'right', 'top', 'bottom'])

/** Where a tab lands. */
export const Drop = Schema.TaggedUnion({
  /** Into a pane's strip, ending at this index. */
  Strip: { pane: Schema.String, index: Schema.Int },
  /** Into a pane, at the end of its strip. */
  Center: { pane: Schema.String },
  /** A new pane on this side of a pane. */
  Edge: { pane: Schema.String, side: SideSchema },
})
export type Drop = typeof Drop.Type

export type Command<V = unknown> =
  | { readonly _tag: 'Open'; readonly ref: string; readonly at?: Drop | undefined; readonly activate?: boolean | undefined }
  | { readonly _tag: 'Activate'; readonly tab: string }
  | { readonly _tag: 'FocusPane'; readonly pane: string }
  | { readonly _tag: 'FocusDirection'; readonly direction: Direction }
  | { readonly _tag: 'Cycle'; readonly by: 1 | -1 }
  | { readonly _tag: 'ActivateIndex'; readonly index: number }
  | { readonly _tag: 'ActivatePrevious' }
  | { readonly _tag: 'Close'; readonly tab: string }
  /** Closes the rest of this tab's pane, and shows this tab. */
  | { readonly _tag: 'CloseOthers'; readonly tab: string }
  /** The tab gives way to another entity in place; not a closed tab to reopen. */
  | { readonly _tag: 'Replace'; readonly tab: string; readonly ref: string }
  | { readonly _tag: 'ReopenClosed' }
  | { readonly _tag: 'Move'; readonly tab: string; readonly to: Drop }
  /**
   * A new pane beside `pane` (the focused one by default), taking `tab`, or opening `ref`, or empty.
   * A pane whose only tab would move to its own edge stays as it is.
   */
  | { readonly _tag: 'Split'; readonly side: Side; readonly pane?: string | undefined; readonly tab?: string | undefined; readonly ref?: string | undefined }
  | { readonly _tag: 'Resize'; readonly split: string; readonly sizes: readonly number[] }
  /** Equal shares for one split, or for every split of the space. */
  | { readonly _tag: 'ResetSizes'; readonly split?: string | undefined }
  | { readonly _tag: 'ToggleZoom' }
  | { readonly _tag: 'SetAside'; readonly pane: string; readonly aside: AsidePatch }
  | { readonly _tag: 'SetView'; readonly tab: string; readonly view: Partial<V> }
  | { readonly _tag: 'SpaceNew'; readonly name?: string | undefined; readonly id?: string | undefined }
  | { readonly _tag: 'SpaceRename'; readonly space: string; readonly name: string }
  | { readonly _tag: 'SpaceDuplicate'; readonly space: string; readonly name?: string | undefined; readonly id?: string | undefined }
  | { readonly _tag: 'SpaceSwitch'; readonly space: string }
  | { readonly _tag: 'SpaceClose'; readonly space: string }
  /** Another writer's body for a space, or its removal. `sizes: false` keeps local sizes. */
  | { readonly _tag: 'Receive'; readonly space: string; readonly body?: SpaceBody | undefined; readonly removed?: boolean | undefined; readonly revision?: string | undefined; readonly sizes?: boolean | undefined }
  | { readonly _tag: 'SetRevision'; readonly space: string; readonly revision: string }

export interface AsidePatch {
  /** `null` clears an explicit toggle. */
  readonly explicit?: { readonly open: boolean; readonly band: Band } | null | undefined
  readonly size?: number | undefined
  readonly tab?: string | undefined
}

export interface WorkspaceConfig {
  readonly newId: () => string
  /** What makes two refs one tab within a space. The ref itself by default. */
  readonly identity?: ((ref: string) => string) | undefined
  readonly closedLimit?: number | undefined
  readonly retiredLimit?: number | undefined
  /** The name a new space gets, numbered when taken. */
  readonly spaceName?: string | undefined
}

const identityOf = (config: WorkspaceConfig) => config.identity ?? ((ref: string) => ref)

const touch = (mru: readonly string[], id: string) => mru[0] === id ? mru : [id, ...mru.filter(other => other !== id)]

const withoutKey = <T>(record: Readonly<Record<string, T>>, key: string): Readonly<Record<string, T>> => {
  if (!(key in record)) return record
  const { [key]: _removed, ...rest } = record
  return rest
}

export const emptyLocal = <V>(focus: string): SpaceLocal<V> => ({ focus, zoomed: null, mru: [], closed: [], views: {}, panes: {} })

export const emptySpace = <V>(config: WorkspaceConfig, options?: { readonly id?: string | undefined; readonly name?: string | undefined }): Space<V> => {
  const pane = config.newId()
  return { id: options?.id ?? config.newId(), name: options?.name ?? config.spaceName ?? 'Space', layout: { id: pane, tabs: [], active: null }, local: emptyLocal(pane) }
}

export const emptyWorkspace = <V>(config: WorkspaceConfig, options?: { readonly id?: string | undefined; readonly name?: string | undefined }): WorkspaceState<V> => {
  const space = emptySpace<V>(config, options)
  return { version: 1, current: space.id, spaces: [space], retired: [] }
}

/** `base`, or `base 2`, `base 3`… whichever is not taken. */
export const unusedName = (taken: readonly string[], base: string): string => {
  if (!taken.includes(base)) return base
  for (let n = 2; ; n += 1) {
    const name = `${base} ${n}`
    if (!taken.includes(name)) return name
  }
}

export const currentSpace = <V>(state: WorkspaceState<V>): Space<V> =>
  state.spaces.find(space => space.id === state.current) ?? state.spaces[0]!

export const focusedPane = (space: Pick<Space<unknown>, 'layout' | 'local'>): Pane | undefined =>
  findPane(space.layout, space.local.focus) ?? panesOf(space.layout)[0]

/** The tab with the focus. */
export const activeTab = (space: Pick<Space<unknown>, 'layout' | 'local'>): Tab | undefined => {
  const pane = focusedPane(space)
  return pane?.tabs.find(tab => tab.id === pane.active)
}

/** The panes on screen: only the zoomed one while zoomed. */
export const shownPanes = (space: Pick<Space<unknown>, 'layout' | 'local'>): readonly Pane[] => {
  const zoomed = space.local.zoomed === null ? undefined : findPane(space.layout, space.local.zoomed)
  return zoomed === undefined ? panesOf(space.layout) : [zoomed]
}

/** The shared body of a space. */
export const spaceBody = (space: Pick<Space<unknown>, 'name' | 'layout'>, options: { readonly sizes: boolean }): SpaceBody =>
  ({ name: space.name, layout: toWire(space.layout, options) })

// ---------------------------------------------------------------------------------------------------
// Within one space

type SpaceStep<V> = (space: Space<V>) => Space<V>

/** Keeps local state consistent with the layout: focus on a pane, zoom on a pane when split, no stale ids. */
const settle = <V>(space: Space<V>): Space<V> => {
  const panes = panesOf(space.layout)
  const paneIds = new Set(panes.map(pane => pane.id))
  const tabIds = new Set(tabsOf(space.layout).map(tab => tab.id))
  const { local } = space
  const focus = paneIds.has(local.focus) ? local.focus : panes[0]!.id
  const zoomed = local.zoomed !== null && paneIds.has(local.zoomed) && panes.length > 1 ? local.zoomed : null
  const mru = local.mru.every(id => tabIds.has(id)) ? local.mru : local.mru.filter(id => tabIds.has(id))
  const views = Object.keys(local.views).every(id => tabIds.has(id)) ? local.views : Object.fromEntries(Object.entries(local.views).filter(([id]) => tabIds.has(id)))
  const paneLocal = Object.keys(local.panes).every(id => paneIds.has(id)) ? local.panes : Object.fromEntries(Object.entries(local.panes).filter(([id]) => paneIds.has(id)))
  if (focus === local.focus && zoomed === local.zoomed && mru === local.mru && views === local.views && paneLocal === local.panes) return space
  return { ...space, local: { ...local, focus, zoomed, mru, views, panes: paneLocal } }
}

const withLocal = <V>(space: Space<V>, patch: Partial<SpaceLocal<V>>): Space<V> => ({ ...space, local: { ...space.local, ...patch } })

/** The pane a tab left keeps showing one: the one it showed, else what took its place, else the last. */
const withoutTab = (pane: Pane, id: string): Pane => {
  const index = pane.tabs.findIndex(tab => tab.id === id)
  const tabs = pane.tabs.filter(tab => tab.id !== id)
  return { ...pane, tabs, active: pane.active === id ? tabs[Math.min(index, tabs.length - 1)]?.id ?? null : pane.active }
}

/** Shows a tab in its pane and gives that pane the focus. Focus leaving a zoomed pane unzooms. */
const reveal = <V>(space: Space<V>, paneId: string, id: string): Space<V> => {
  const pane = findPane(space.layout, paneId)
  if (pane === undefined) return space
  const zoomed = space.local.zoomed !== null && space.local.zoomed !== paneId ? null : space.local.zoomed
  if (pane.active === id && space.local.focus === paneId && space.local.mru[0] === id && zoomed === space.local.zoomed) return space
  return {
    ...space,
    layout: pane.active === id ? space.layout : mapPane(space.layout, paneId, row => ({ ...row, active: id })),
    local: { ...space.local, focus: paneId, zoomed, mru: touch(space.local.mru, id) },
  }
}

const axisOf = (side: Side) => side === 'left' || side === 'right' ? 'row' as const : 'column' as const
const leads = (side: Side) => side === 'left' || side === 'top'

/** Puts a tab that is in no pane at its drop. An edge of an empty pane is that pane. */
const place = (layout: Layout, tab: Tab, to: Drop, config: WorkspaceConfig): { readonly layout: Layout; readonly pane: string } | undefined => {
  const target = findPane(layout, to.pane)
  if (target === undefined) return undefined
  if (to._tag !== 'Edge' || target.tabs.length === 0) {
    const index = to._tag === 'Strip' ? Math.max(0, Math.min(target.tabs.length, Math.trunc(to.index))) : target.tabs.length
    const tabs = [...target.tabs.slice(0, index), tab, ...target.tabs.slice(index)]
    return { layout: mapPane(layout, target.id, pane => ({ ...pane, tabs, active: tab.id })), pane: target.id }
  }
  const fresh: Pane = { id: config.newId(), tabs: [tab], active: tab.id }
  return { layout: splitBeside(layout, target.id, to.side, fresh, config), pane: fresh.id }
}

/** A binary split of `target` and `fresh`, `fresh` on `side`. */
const splitBeside = (layout: Layout, target: string, side: Side, fresh: Pane, config: WorkspaceConfig): Layout =>
  mapPane(layout, target, pane => ({ id: config.newId(), axis: axisOf(side), children: leads(side) ? [fresh, pane] : [pane, fresh] }))

const move = <V>(space: Space<V>, id: string, to: Drop, config: WorkspaceConfig): Space<V> => {
  const from = locateTab(space.layout, tab => tab.id === id)
  if (from === undefined || findPane(space.layout, to.pane) === undefined) return space
  const samePane = from.pane.id === to.pane
  if (to._tag === 'Edge' && samePane && from.pane.tabs.length === 1) return space
  if (to._tag === 'Center' && samePane) return reveal(space, from.pane.id, id)
  if (to._tag === 'Strip' && samePane) {
    const index = Math.max(0, Math.min(from.pane.tabs.length - 1, Math.trunc(to.index)))
    if (from.pane.tabs[index]?.id === id) return reveal(space, from.pane.id, id)
  }
  const left = withoutTab(from.pane, id)
  const remaining = mapPane(space.layout, from.pane.id, () => left)
  // A tab moving within its own pane leaves it empty only for the moment before it lands.
  const layout = left.tabs.length === 0 && !samePane ? removePane(remaining, left.id) : remaining
  if (layout === undefined) return space
  const placed = place(layout, from.tab, to, config)
  if (placed === undefined) return space
  return settle({ ...space, layout: placed.layout, local: { ...space.local, focus: placed.pane, zoomed: null, mru: touch(space.local.mru, id) } })
}

const open = <V>(space: Space<V>, command: Extract<Command<V>, { _tag: 'Open' }>, config: WorkspaceConfig): Space<V> => {
  if (command.ref === '') return space
  const identity = identityOf(config)
  const key = identity(command.ref)
  const existing = locateTab(space.layout, tab => identity(tab.ref) === key)
  if (existing !== undefined) {
    if (command.at !== undefined) return move(space, existing.tab.id, command.at, config)
    return command.activate === false ? space : reveal(space, existing.pane.id, existing.tab.id)
  }
  const tab: Tab = { id: config.newId(), ref: command.ref }
  const closed = space.local.closed.filter(ref => identity(ref) !== key)
  if (command.at !== undefined) {
    const placed = place(space.layout, tab, command.at, config)
    if (placed === undefined) return space
    return settle({ ...space, layout: placed.layout, local: { ...space.local, focus: placed.pane, zoomed: null, closed, mru: touch(space.local.mru, tab.id) } })
  }
  const target = focusedPane(space)
  if (target === undefined) return space
  const quiet = command.activate === false && target.active !== null
  const layout = mapPane(space.layout, target.id, pane => ({ ...pane, tabs: [...pane.tabs, tab], active: quiet ? pane.active : tab.id }))
  return { ...space, layout, local: { ...space.local, focus: target.id, closed, mru: quiet ? [...space.local.mru, tab.id] : touch(space.local.mru, tab.id) } }
}

const close = <V>(space: Space<V>, id: string, config: WorkspaceConfig): Space<V> => {
  const found = locateTab(space.layout, tab => tab.id === id)
  if (found === undefined) return space
  const identity = identityOf(config)
  const panes = panesOf(space.layout)
  const left = withoutTab(found.pane, id)
  const emptied = left.tabs.length === 0 && panes.length > 1
  const remaining = mapPane(space.layout, left.id, () => left)
  const layout = (emptied ? removePane(remaining, left.id) : remaining) ?? remaining
  // The focus an emptied pane leaves goes to the pane before it.
  const index = panes.findIndex(pane => pane.id === left.id)
  const focus = emptied && space.local.focus === left.id ? panes[index === 0 ? 1 : index - 1]!.id : space.local.focus
  const mru = space.local.mru.filter(other => other !== id)
  const shown = activeTab({ layout, local: { ...space.local, focus } })?.id
  const closedLimit = config.closedLimit ?? 20
  return settle({
    ...space,
    layout,
    local: {
      ...space.local,
      focus,
      mru: shown === undefined ? mru : touch(mru, shown),
      closed: [found.tab.ref, ...space.local.closed.filter(ref => identity(ref) !== identity(found.tab.ref))].slice(0, closedLimit),
      views: withoutKey(space.local.views, id),
    },
  })
}

const split = <V>(space: Space<V>, command: Extract<Command<V>, { _tag: 'Split' }>, config: WorkspaceConfig): Space<V> => {
  const target = command.pane === undefined ? focusedPane(space) : findPane(space.layout, command.pane)
  if (target === undefined) return space
  if (command.tab !== undefined) return move(space, command.tab, { _tag: 'Edge', pane: target.id, side: command.side }, config)
  if (command.ref !== undefined) {
    if (target.tabs.length === 0) return open(space, { _tag: 'Open', ref: command.ref, at: { _tag: 'Center', pane: target.id } }, config)
    return open(space, { _tag: 'Open', ref: command.ref, at: { _tag: 'Edge', pane: target.id, side: command.side } }, config)
  }
  const fresh: Pane = { id: config.newId(), tabs: [], active: null }
  return settle({ ...space, layout: splitBeside(space.layout, target.id, command.side, fresh, config), local: { ...space.local, focus: fresh.id, zoomed: null } })
}

const sameNumbers = (left: readonly number[] | undefined, right: readonly number[] | undefined) =>
  left === right || (left !== undefined && right !== undefined && left.length === right.length && left.every((value, index) => Math.abs(value - right[index]!) < 1e-9))

const withoutSizes = (layout: Layout, only: string | undefined): Layout => {
  if (isPane(layout)) return layout
  const children = layout.children.map(child => withoutSizes(child, only))
  const changedChildren = !children.every((child, index) => child === layout.children[index])
  const reset = layout.sizes !== undefined && (only === undefined || only === layout.id)
  if (!reset && !changedChildren) return layout
  const { sizes, ...rest } = layout
  return reset ? { ...rest, children } : { ...rest, children, sizes }
}

const spaceStep = <V>(command: Command<V>, config: WorkspaceConfig): SpaceStep<V> | undefined => {
  switch (command._tag) {
    case 'Open':
      return space => open(space, command, config)
    case 'Activate':
      return space => {
        const found = locateTab(space.layout, tab => tab.id === command.tab)
        return found === undefined ? space : reveal(space, found.pane.id, found.tab.id)
      }
    case 'FocusPane':
      return space => {
        const pane = findPane(space.layout, command.pane)
        if (pane === undefined || pane.id === space.local.focus) return space
        return withLocal(space, {
          focus: pane.id,
          zoomed: space.local.zoomed === null ? null : pane.id === space.local.zoomed ? pane.id : null,
          mru: pane.active === null ? space.local.mru : touch(space.local.mru, pane.active),
        })
      }
    case 'FocusDirection':
      return space => {
        // Moving out of a zoomed pane brings the layout back first.
        if (space.local.zoomed !== null) return withLocal(space, { zoomed: null })
        const to = neighbour(paneRects(space.layout), space.local.focus, command.direction)
        const pane = to === undefined ? undefined : findPane(space.layout, to)
        if (pane === undefined) return space
        return withLocal(space, { focus: pane.id, mru: pane.active === null ? space.local.mru : touch(space.local.mru, pane.active) })
      }
    case 'Cycle':
      return space => {
        const pane = focusedPane(space)
        if (pane === undefined || pane.tabs.length === 0) return space
        const index = pane.tabs.findIndex(tab => tab.id === pane.active)
        const next = (Math.max(index, 0) + command.by + pane.tabs.length) % pane.tabs.length
        return reveal(space, pane.id, pane.tabs[next]!.id)
      }
    case 'ActivateIndex':
      return space => {
        const pane = focusedPane(space)
        const tab = pane?.tabs[command.index]
        return pane === undefined || tab === undefined ? space : reveal(space, pane.id, tab.id)
      }
    case 'ActivatePrevious':
      return space => {
        const shown = activeTab(space)?.id
        const previous = space.local.mru.find(id => id !== shown)
        const found = previous === undefined ? undefined : locateTab(space.layout, tab => tab.id === previous)
        return found === undefined ? space : reveal(space, found.pane.id, found.tab.id)
      }
    case 'Close':
      return space => close(space, command.tab, config)
    case 'CloseOthers':
      return space => {
        const found = locateTab(space.layout, tab => tab.id === command.tab)
        if (found === undefined) return space
        const closed = found.pane.tabs.filter(tab => tab.id !== command.tab).reduce((current, tab) => close(current, tab.id, config), space)
        return reveal(closed, found.pane.id, command.tab)
      }
    case 'Replace':
      return space => {
        const found = locateTab(space.layout, tab => tab.id === command.tab)
        if (found === undefined || command.ref === '') return space
        const identity = identityOf(config)
        const key = identity(command.ref)
        if (key === identity(found.tab.ref)) return reveal(space, found.pane.id, found.tab.id)
        // The entity already has its tab: that one shows, and this one goes.
        if (locateTab(space.layout, tab => identity(tab.ref) === key) !== undefined) {
          const { closed } = space.local
          const next = open(close(space, command.tab, config), { _tag: 'Open', ref: command.ref }, config)
          return withLocal(next, { closed })
        }
        const tab: Tab = { id: config.newId(), ref: command.ref }
        return {
          ...space,
          layout: mapPane(space.layout, found.pane.id, pane => ({
            ...pane,
            tabs: pane.tabs.map(row => row.id === command.tab ? tab : row),
            active: pane.active === command.tab ? tab.id : pane.active,
          })),
          local: { ...space.local, mru: space.local.mru.map(id => id === command.tab ? tab.id : id), views: withoutKey(space.local.views, command.tab) },
        }
      }
    case 'ReopenClosed':
      return space => {
        const ref = space.local.closed[0]
        return ref === undefined ? space : open(space, { _tag: 'Open', ref }, config)
      }
    case 'Move':
      return space => move(space, command.tab, command.to, config)
    case 'Split':
      return space => split(space, command, config)
    case 'Resize':
      return space => {
        const layout = mapSplit(space.layout, command.split, current => {
          const sizes = normalizeSizes(command.sizes, current.children.length)
          if (sizes === undefined || sameNumbers(normalizeSizes(current.sizes, current.children.length), sizes)) return current
          return { ...current, sizes }
        })
        return layout === space.layout ? space : { ...space, layout }
      }
    case 'ResetSizes':
      return space => {
        const layout = withoutSizes(space.layout, command.split)
        return layout === space.layout ? space : { ...space, layout }
      }
    case 'ToggleZoom':
      return space => {
        if (space.local.zoomed !== null) return withLocal(space, { zoomed: null })
        return panesOf(space.layout).length > 1 ? withLocal(space, { zoomed: space.local.focus }) : space
      }
    case 'SetAside':
      return space => {
        if (findPane(space.layout, command.pane) === undefined) return space
        const current = space.local.panes[command.pane]?.aside ?? {}
        const { explicit, size, tab } = command.aside
        const next: Record<string, unknown> = { ...current }
        if (explicit === null) delete next.explicit
        else if (explicit !== undefined) next.explicit = { open: explicit.open, band: explicit.band }
        if (size !== undefined) next.size = size
        if (tab !== undefined) next.tab = tab
        if (JSON.stringify(next) === JSON.stringify(current)) return space
        return withLocal(space, { panes: { ...space.local.panes, [command.pane]: { ...space.local.panes[command.pane], aside: next as AsideState } } })
      }
    case 'SetView':
      return space => {
        if (locateTab(space.layout, tab => tab.id === command.tab) === undefined) return space
        const current = space.local.views[command.tab]
        const next = { ...current, ...command.view } as V
        if (current !== undefined && Object.keys(next as object).every(key => (current as Record<string, unknown>)[key] === (next as Record<string, unknown>)[key])) return space
        return withLocal(space, { views: { ...space.local.views, [command.tab]: next } })
      }
    default:
      return undefined
  }
}

// ---------------------------------------------------------------------------------------------------
// Across spaces

const replaceSpace = <V>(state: WorkspaceState<V>, id: string, next: Space<V>): WorkspaceState<V> =>
  ({ ...state, spaces: state.spaces.map(space => space.id === id ? next : space) })

/** A space from a body: fresh local ids, empty local state. */
const spaceFromBody = <V>(body: SpaceBody, config: WorkspaceConfig, options: { readonly id: string; readonly revision?: string | undefined; readonly local?: SpaceLocal<V> | undefined }): Space<V> => {
  const seed = emptySpace<V>(config, { id: options.id, name: body.name })
  const base: Space<V> = options.local === undefined ? seed : { ...seed, local: options.local }
  const space = adoptSpace(base, body, config, { sizes: true })
  return options.revision === undefined ? space : { ...space, revision: options.revision }
}

/** Adopts another writer's structure, keeping this client's identities and view where they still apply. */
export const adoptSpace = <V>(space: Space<V>, body: SpaceBody, config: WorkspaceConfig, options: { readonly sizes: boolean }): Space<V> => {
  const layout = adoptLayout(space.layout, body.layout, { identity: identityOf(config), newId: config.newId, sizes: options.sizes })
  const previous = activeTab(space)?.id
  const panes = panesOf(layout)
  const focus = panes.find(pane => pane.id === space.local.focus)?.id
    ?? panes.find(pane => pane.tabs.some(tab => tab.id === previous))?.id
    ?? panes[0]!.id
  const ids = new Set(tabsOf(layout).map(tab => tab.id))
  const shown = activeTab({ layout, local: { ...space.local, focus } })?.id
  // A stored document can repeat an id; recency keeps its first, most recent, mention.
  const known = [...new Set(space.local.mru)].filter(id => ids.has(id))
  const recent = shown === undefined ? known : touch(known, shown)
  return settle({
    ...space,
    name: body.name,
    layout,
    local: { ...space.local, focus, mru: [...recent, ...[...ids].filter(id => !recent.includes(id))] },
  })
}

const receive = <V>(state: WorkspaceState<V>, command: Extract<Command<V>, { _tag: 'Receive' }>, config: WorkspaceConfig): WorkspaceState<V> => {
  const sizes = command.sizes ?? true
  const existing = state.spaces.find(space => space.id === command.space)
  const retiredLimit = config.retiredLimit ?? 20
  if (command.removed === true) {
    if (existing === undefined) return state
    if (state.spaces.length === 1) {
      // The last space stays; it only forgets its revision.
      return existing.revision === undefined ? state : replaceSpace(state, existing.id, { ...existing, revision: undefined })
    }
    const spaces = state.spaces.filter(space => space.id !== command.space)
    const index = state.spaces.findIndex(space => space.id === command.space)
    return {
      ...state,
      spaces,
      current: state.current === command.space ? spaces[Math.max(0, index - 1)]!.id : state.current,
      retired: [{ id: existing.id, local: existing.local }, ...state.retired.filter(row => row.id !== existing.id)].slice(0, retiredLimit),
    }
  }
  if (command.body === undefined) return state
  if (existing === undefined) {
    const retired = state.retired.find(row => row.id === command.space)
    const space = spaceFromBody<V>(command.body, config, { id: command.space, revision: command.revision, local: retired?.local })
    return { ...state, spaces: [...state.spaces, space], retired: state.retired.filter(row => row.id !== command.space) }
  }
  const adopted = adoptSpace(existing, command.body, config, { sizes })
  const unchanged = adopted.name === existing.name
    && canonicalBody(spaceBody(adopted, { sizes: true })) === canonicalBody(spaceBody(existing, { sizes: true }))
  const revised = command.revision === undefined || command.revision === existing.revision ? undefined : command.revision
  if (unchanged) return revised === undefined ? state : replaceSpace(state, existing.id, { ...existing, revision: revised })
  return replaceSpace(state, existing.id, revised === undefined ? adopted : { ...adopted, revision: revised })
}

/** A copy of a space's structure under fresh local ids. */
const duplicate = <V>(state: WorkspaceState<V>, command: Extract<Command<V>, { _tag: 'SpaceDuplicate' }>, config: WorkspaceConfig): WorkspaceState<V> => {
  const source = state.spaces.find(space => space.id === command.space)
  if (source === undefined) return state
  const name = command.name ?? unusedName(state.spaces.map(space => space.name), source.name)
  const id = command.id ?? config.newId()
  if (state.spaces.some(space => space.id === id)) return state
  const space = spaceFromBody<V>({ ...spaceBody(source, { sizes: true }), name }, config, { id })
  return { ...state, spaces: [...state.spaces, space], current: space.id }
}

export const transition = <V>(state: WorkspaceState<V>, command: Command<V>, config: WorkspaceConfig): WorkspaceState<V> => {
  const step = spaceStep(command, config)
  if (step !== undefined) {
    const space = currentSpace(state)
    const next = step(space)
    return next === space ? state : replaceSpace(state, space.id, next)
  }
  switch (command._tag) {
    case 'SpaceNew': {
      const id = command.id ?? config.newId()
      if (state.spaces.some(space => space.id === id)) return state
      const space = emptySpace<V>(config, { id, name: command.name ?? unusedName(state.spaces.map(row => row.name), config.spaceName ?? 'Space') })
      return { ...state, spaces: [...state.spaces, space], current: space.id }
    }
    case 'SpaceRename': {
      const space = state.spaces.find(row => row.id === command.space)
      const name = command.name.trim()
      if (space === undefined || name === '' || name === space.name) return state
      return replaceSpace(state, space.id, { ...space, name })
    }
    case 'SpaceDuplicate':
      return duplicate(state, command, config)
    case 'SpaceSwitch':
      return command.space === state.current || !state.spaces.some(space => space.id === command.space) ? state : { ...state, current: command.space }
    case 'SpaceClose': {
      const index = state.spaces.findIndex(space => space.id === command.space)
      if (index === -1 || state.spaces.length === 1) return state
      const spaces = state.spaces.filter(space => space.id !== command.space)
      return { ...state, spaces, current: state.current === command.space ? spaces[Math.max(0, index - 1)]!.id : state.current }
    }
    case 'Receive':
      return receive(state, command, config)
    case 'SetRevision': {
      const space = state.spaces.find(row => row.id === command.space)
      if (space === undefined || space.revision === command.revision) return state
      return replaceSpace(state, space.id, { ...space, revision: command.revision })
    }
    default:
      return state
  }
}

/** A stored document can name anything: restores the machine's invariants on what it says. */
export const repairWorkspace = <V>(state: WorkspaceState<V>, config: WorkspaceConfig): WorkspaceState<V> => {
  const seenSpaces = new Set<string>()
  const spaces = state.spaces.flatMap(space => {
    if (seenSpaces.has(space.id)) return []
    seenSpaces.add(space.id)
    return [adoptSpace(space, spaceBody(space, { sizes: true }), config, { sizes: true })]
  })
  if (spaces.length === 0) return emptyWorkspace(config)
  const current = spaces.some(space => space.id === state.current) ? state.current : spaces[0]!.id
  return { version: 1, current, spaces, retired: state.retired.filter(row => !seenSpaces.has(row.id)) }
}

export { splitsOf }
