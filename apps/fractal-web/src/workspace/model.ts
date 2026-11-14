import { Schema } from 'effect'
import { isAgentTerminalPane, paneIdentity } from './subject.ts'

/**
 * The workspace state machine as pure data, shaped like the TUI's glass (`crates/fractal-tui/src/ui/layout.rs`):
 * a tree of splits whose leaves are groups of tabs, each tab one pane key. One subject has one tab in the
 * whole tree. Which tab each group shows, focus, sizes, scroll and local ids stay on this device and are
 * not part of `glassBody`. Every command is total: one that does not apply returns the same state reference.
 */
export const Side = Schema.Literals(['right', 'below'])
export type Side = typeof Side.Type

export const Tab = Schema.Struct({
  /** Stable on this device across moves, never synced (`layout.rs::Tab::id`). */
  id: Schema.String,
  pane: Schema.NonEmptyString,
  title: Schema.optional(Schema.String),
})
export type Tab = typeof Tab.Type

export const Group = Schema.Struct({
  /** Stable on this device, independent of the group's position in the tree. */
  id: Schema.String,
  tabs: Schema.Array(Tab),
  active: Schema.NullOr(Schema.String),
})
export type Group = typeof Group.Type

export interface Split {
  readonly split: Side
  readonly children: readonly Layout[]
  /** The first child's share of a two-way split, once a border was dragged; equal otherwise. */
  readonly ratio?: number | undefined
}
export type Layout = Group | Split

export const Layout: Schema.Codec<Layout> = Schema.Union([
  Group,
  Schema.Struct({
    split: Side,
    children: Schema.Array(Schema.suspend((): Schema.Codec<Layout> => Layout)),
    ratio: Schema.optional(Schema.Finite),
  }),
])

/** Per-tab view state restored when a tab's surface is mounted again. Local, like glass focus and scroll. */
export const TabView = Schema.Struct({
  scrollTop: Schema.optional(Schema.Finite),
  expanded: Schema.optional(Schema.String),
  /** Which face an agent's tab shows; a key naming the terminal itself shows it until told otherwise. */
  terminal: Schema.optional(Schema.Boolean),
  /** Holds an unsent draft. The kit keeps drafts only in the mounted composer, so this pins the surface. */
  dirty: Schema.optional(Schema.Boolean),
})
export type TabView = typeof TabView.Type

/** The right sidebar. One for the workspace: opened for one tab, it stays open for the next. */
export const Panel = Schema.Struct({
  open: Schema.Boolean,
  tab: Schema.String,
  width: Schema.optional(Schema.Finite),
})
export type Panel = typeof Panel.Type

export const WorkspaceState = Schema.Struct({
  layout: Layout,
  /** The group keys and the URL act on. */
  focus: Schema.String,
  /** Most recently active first; drives Ctrl+^ and the resource manager's warm order. */
  mru: Schema.Array(Schema.String),
  /** Most recently closed pane first; drives reopen. */
  closed: Schema.Array(Schema.String),
  views: Schema.Record(Schema.String, TabView),
  panel: Panel,
})
export type WorkspaceState = typeof WorkspaceState.Type

const rootGroup = 'main'

export const emptyWorkspace: WorkspaceState = {
  layout: { id: rootGroup, tabs: [], active: null },
  focus: rootGroup,
  mru: [],
  closed: [],
  views: {},
  panel: { open: false, tab: 'details' },
}

/** Where a tab lands (`glass.rs::Drop`). */
export const Drop = Schema.TaggedUnion({
  /** Into a group's strip, ending at this index. */
  Strip: { group: Schema.String, index: Schema.Int },
  /** A new split beside a group, holding the tab: before the group when `first`. */
  Edge: { group: Schema.String, side: Side, first: Schema.Boolean },
})
export type Drop = typeof Drop.Type

export interface WireTab {
  readonly pane: string
  readonly title?: string | undefined
}

/** A glass body's layout, exactly as `PUT /v1/client/glasses/{uuid}` accepts it: splits are two-way. */
export type GlassLayout =
  | { readonly tabs: readonly WireTab[] }
  | { readonly split: Side; readonly children: readonly [GlassLayout, GlassLayout]; readonly ratio?: number }

export interface GlassBody {
  readonly name: string
  readonly layout: GlassLayout
}

/** A layout from another writer: a window's stored one, or a glass's, which has no local identities. */
export type ReceivedLayout =
  | { readonly id?: string; readonly tabs: readonly (WireTab & { readonly id?: string })[]; readonly active?: string | null }
  | { readonly split: Side; readonly children: readonly ReceivedLayout[]; readonly ratio?: number | undefined }

export const ReceivedLayout: Schema.Codec<ReceivedLayout> = Schema.Union([
  Schema.Struct({
    id: Schema.optionalKey(Schema.String),
    tabs: Schema.Array(Schema.Struct({
      id: Schema.optionalKey(Schema.String),
      pane: Schema.String,
      title: Schema.optional(Schema.String),
    })),
    active: Schema.optionalKey(Schema.NullOr(Schema.String)),
  }),
  Schema.Struct({
    split: Side,
    children: Schema.Array(Schema.suspend((): Schema.Codec<ReceivedLayout> => ReceivedLayout)),
    ratio: Schema.optional(Schema.Finite),
  }),
])

export const Command = Schema.TaggedUnion({
  Open: { pane: Schema.String, activate: Schema.optionalKey(Schema.Boolean), at: Schema.optionalKey(Drop) },
  Activate: { id: Schema.String },
  FocusGroup: { group: Schema.String },
  Cycle: { by: Schema.Literals([1, -1]) },
  ActivateIndex: { index: Schema.Int },
  ActivatePrevious: {},
  Close: { id: Schema.String },
  ReopenClosed: {},
  Move: { id: Schema.String, to: Drop },
  /** `node` counts splits in tree order, the whole layout first (`layout.rs::Divider`). */
  SetRatio: { node: Schema.Int, ratio: Schema.Finite },
  SetView: { id: Schema.String, view: TabView },
  SetPanel: {
    panel: Schema.Struct({
      open: Schema.optionalKey(Schema.Boolean),
      tab: Schema.optionalKey(Schema.String),
      width: Schema.optional(Schema.Finite),
    }),
  },
  Receive: { layout: ReceivedLayout },
})
export type Command = typeof Command.Type

export interface TransitionContext {
  readonly newId: () => string
}

const closedLimit = 20

export const isGroup = (layout: Layout): layout is Group => 'tabs' in layout

/** Every group, left to right and top to bottom. */
export const groupsOf = (layout: Layout): readonly Group[] =>
  isGroup(layout) ? [layout] : layout.children.flatMap(groupsOf)

export const tabsOf = (layout: Layout): readonly Tab[] => groupsOf(layout).flatMap(group => group.tabs)

export const focusedGroup = (state: Pick<WorkspaceState, 'layout' | 'focus'>): Group | undefined =>
  groupsOf(state.layout).find(group => group.id === state.focus)

/** The tab with the focus: the one the URL names and the header follows. */
export const activeTab = (state: Pick<WorkspaceState, 'layout' | 'focus'>): Tab | undefined => {
  const group = focusedGroup(state)
  return group?.tabs.find(tab => tab.id === group.active)
}

/** One tab per group is on screen. */
export const visibleTabIds = (layout: Layout): ReadonlySet<string> =>
  new Set(groupsOf(layout).flatMap(group => group.active === null ? [] : [group.active]))

/** Whether this tab shows its terminal face. */
export const showsTerminal = (tab: Pick<Tab, 'pane'>, view: TabView | undefined): boolean =>
  view?.terminal ?? isAgentTerminalPane(tab.pane)

const touch = (mru: readonly string[], id: string) => mru[0] === id ? mru : [id, ...mru.filter(other => other !== id)]

const withoutKey = <V>(record: Readonly<Record<string, V>>, key: string): Readonly<Record<string, V>> => {
  if (!(key in record)) return record
  const { [key]: _removed, ...rest } = record
  return rest
}

const locate = (layout: Layout, predicate: (tab: Tab) => boolean): { readonly group: Group; readonly tab: Tab } | undefined => {
  for (const group of groupsOf(layout)) {
    const tab = group.tabs.find(predicate)
    if (tab !== undefined) return { group, tab }
  }
  return undefined
}

const replaceGroup = (layout: Layout, id: string, replace: (group: Group) => Layout): Layout => {
  if (isGroup(layout)) return layout.id === id ? replace(layout) : layout
  return { ...layout, children: layout.children.map(child => replaceGroup(child, id, replace)) }
}

/** The layout without a group; `undefined` when that was its only group. A split left with one child becomes that child. */
const removeGroup = (layout: Layout, id: string): Layout | undefined => {
  if (isGroup(layout)) return layout.id === id ? undefined : layout
  const children = layout.children.flatMap(child => {
    const kept = removeGroup(child, id)
    return kept === undefined ? [] : [kept]
  })
  if (children.length === 0) return undefined
  if (children.length === 1) return children[0]
  const { ratio, ...split } = layout
  return ratio === undefined || children.length !== 2 ? { ...split, children } : { ...split, children, ratio }
}

/** The group a tab left keeps showing a tab: the one it showed, else what took that place, else the last. */
const withoutTab = (group: Group, id: string): Group => {
  const index = group.tabs.findIndex(tab => tab.id === id)
  const tabs = group.tabs.filter(tab => tab.id !== id)
  return { ...group, tabs, active: group.active === id ? tabs[Math.min(index, tabs.length - 1)]?.id ?? null : group.active }
}

/** Shows a tab in its group and gives that group the focus. */
const reveal = (state: WorkspaceState, group: string, id: string): WorkspaceState => {
  const current = groupsOf(state.layout).find(row => row.id === group)
  if (current === undefined) return state
  if (current.active === id && state.focus === group && state.mru[0] === id) return state
  return {
    ...state,
    layout: current.active === id ? state.layout : replaceGroup(state.layout, group, row => ({ ...row, active: id })),
    focus: group,
    mru: touch(state.mru, id),
  }
}

/** Puts a tab that is in no group at its drop. An edge of the empty root group is that group. */
const place = (layout: Layout, tab: Tab, to: Drop, context: TransitionContext): { readonly layout: Layout; readonly group: string } | undefined => {
  const target = groupsOf(layout).find(group => group.id === to.group)
  if (target === undefined) return undefined
  if (to._tag === 'Strip' || target.tabs.length === 0) {
    const index = to._tag === 'Strip' ? Math.max(0, Math.min(target.tabs.length, Math.trunc(to.index))) : 0
    const tabs = [...target.tabs.slice(0, index), tab, ...target.tabs.slice(index)]
    return { layout: replaceGroup(layout, target.id, group => ({ ...group, tabs, active: tab.id })), group: target.id }
  }
  const fresh: Group = { id: context.newId(), tabs: [tab], active: tab.id }
  return {
    layout: replaceGroup(layout, target.id, group => ({ split: to.side, children: to.first ? [fresh, group] : [group, fresh] })),
    group: fresh.id,
  }
}

const move = (state: WorkspaceState, id: string, to: Drop, context: TransitionContext): WorkspaceState => {
  const from = locate(state.layout, tab => tab.id === id)
  if (from === undefined || !groupsOf(state.layout).some(group => group.id === to.group)) return state
  const sameGroup = from.group.id === to.group
  if (to._tag === 'Edge' && sameGroup && from.group.tabs.length === 1) return state
  if (to._tag === 'Strip' && sameGroup) {
    const index = Math.max(0, Math.min(from.group.tabs.length - 1, Math.trunc(to.index)))
    if (from.group.tabs[index]?.id === id) return reveal(state, from.group.id, id)
  }
  const left = withoutTab(from.group, id)
  const remaining = replaceGroup(state.layout, from.group.id, () => left)
  // A tab moving within its own group leaves it empty only for the moment before it lands.
  const layout = left.tabs.length === 0 && !sameGroup ? removeGroup(remaining, left.id) : remaining
  if (layout === undefined) return state
  const placed = place(layout, from.tab, to, context)
  if (placed === undefined) return state
  return { ...state, layout: placed.layout, focus: placed.group, mru: touch(state.mru, id) }
}

const open = (state: WorkspaceState, command: Extract<Command, { _tag: 'Open' }>, context: TransitionContext): WorkspaceState => {
  if (command.pane === '') return state
  const identity = paneIdentity(command.pane)
  // A terminal link opens the agent's one tab, on its terminal face.
  const withFace = (next: WorkspaceState, id: string) =>
    isAgentTerminalPane(command.pane) && next.views[id]?.terminal !== true
      ? { ...next, views: { ...next.views, [id]: { ...next.views[id], terminal: true } } }
      : next
  const existing = locate(state.layout, tab => paneIdentity(tab.pane) === identity)
  if (existing !== undefined) {
    const placed = command.at !== undefined
      ? move(state, existing.tab.id, command.at, context)
      : command.activate === false ? state : reveal(state, existing.group.id, existing.tab.id)
    return withFace(placed, existing.tab.id)
  }
  const tab: Tab = { id: context.newId(), pane: identity }
  const closed = state.closed.filter(pane => paneIdentity(pane) !== identity)
  if (command.at !== undefined) {
    const placed = place(state.layout, tab, command.at, context)
    if (placed !== undefined) return withFace({ ...state, layout: placed.layout, focus: placed.group, closed, mru: touch(state.mru, tab.id) }, tab.id)
  }
  const target = focusedGroup(state)
  if (target === undefined) return state
  const quiet = command.activate === false && target.active !== null
  const layout = replaceGroup(state.layout, target.id, group => ({ ...group, tabs: [...group.tabs, tab], active: quiet ? group.active : tab.id }))
  return withFace({ ...state, layout, closed, mru: quiet ? [...state.mru, tab.id] : touch(state.mru, tab.id) }, tab.id)
}

const close = (state: WorkspaceState, id: string): WorkspaceState => {
  const found = locate(state.layout, tab => tab.id === id)
  if (found === undefined) return state
  const groups = groupsOf(state.layout)
  const left = withoutTab(found.group, id)
  const emptied = left.tabs.length === 0 && groups.length > 1
  const remaining = replaceGroup(state.layout, left.id, () => left)
  const layout = (emptied ? removeGroup(remaining, left.id) : remaining) ?? remaining
  // `glass.rs::close_tab`: the focus left by an emptied group goes to the group before it.
  const index = groups.findIndex(group => group.id === left.id)
  const focus = emptied && state.focus === left.id ? groups[index === 0 ? 1 : index - 1]!.id : state.focus
  const mru = state.mru.filter(other => other !== id)
  const shown = activeTab({ layout, focus })?.id
  return {
    ...state,
    layout,
    focus,
    mru: shown === undefined ? mru : touch(mru, shown),
    closed: [found.tab.pane, ...state.closed.filter(pane => paneIdentity(pane) !== paneIdentity(found.tab.pane))].slice(0, closedLimit),
    views: withoutKey(state.views, id),
  }
}

const setRatio = (layout: Layout, node: number, ratio: number): Layout => {
  let seen = -1
  const walk = (current: Layout): Layout => {
    if (isGroup(current)) return current
    seen += 1
    if (seen === node) return current.children.length === 2 && current.ratio !== ratio ? { ...current, ratio } : current
    const children = current.children.map(walk)
    return children.every((child, index) => child === current.children[index]) ? current : { ...current, children }
  }
  return walk(layout)
}

/** A layout's structure on the wire. Longer splits nest two at a time, as the TUI's `to_wire` does. */
const wireOf = (layout: Layout, ratios: boolean): GlassLayout => {
  if (isGroup(layout)) return { tabs: layout.tabs.map(({ pane, title }) => title === undefined ? { pane } : { pane, title }) }
  const [first, ...rest] = layout.children
  if (first === undefined) return { tabs: [] }
  if (rest.length === 0) return wireOf(first, ratios)
  const second: Layout = rest.length === 1 ? rest[0]! : { split: layout.split, children: rest }
  const children: readonly [GlassLayout, GlassLayout] = [wireOf(first, ratios), wireOf(second, ratios)]
  return ratios && layout.ratio !== undefined && rest.length === 1 ? { split: layout.split, children, ratio: layout.ratio } : { split: layout.split, children }
}

/** A glass body, as the graph would store it. `ratios` is for a gateway whose glasses carry them (`glass.rs::ratios_on_the_wire`). */
export const glassBody = ({ layout, name, ratios = false }: { readonly layout: Layout; readonly name: string; readonly ratios?: boolean }): GlassBody =>
  ({ name, layout: wireOf(layout, ratios) })

const sameStructure = (left: Layout, right: Layout) => JSON.stringify(wireOf(left, false)) === JSON.stringify(wireOf(right, false))

/**
 * Takes another writer's structure wholesale, as the TUI's `glasses_from_graph` does, keeping what is this
 * device's where it still applies: tab and group identities, which tab each group shows, focus and recency.
 * Unknown keys stay; a second tab for one subject, an empty group or a one-child split do not.
 */
const adopt = (state: WorkspaceState, received: ReceivedLayout, context: TransitionContext): WorkspaceState => {
  const localGroups = groupsOf(state.layout)
  // The first of any duplicates keeps its identity, as it is the one `walk` keeps.
  const localTabs = new Map(localGroups.flatMap(group => group.tabs.map(tab => [paneIdentity(tab.pane), { tab, group }] as const)).toReversed())
  const seen = new Set<string>(), groupIds = new Set<string>(), tabIds = new Set<string>()
  const unused = (used: Set<string>, ...candidates: readonly (string | undefined)[]) => {
    const id = candidates.find(candidate => candidate !== undefined && !used.has(candidate)) ?? context.newId()
    used.add(id)
    return id
  }
  const walk = (node: ReceivedLayout): Layout | undefined => {
    if ('split' in node) {
      const children = node.children.flatMap(child => {
        const kept = walk(child)
        return kept === undefined ? [] : [kept]
      })
      if (children.length < 2) return children[0]
      const ratio = children.length === 2 ? node.ratio : undefined
      return ratio === undefined ? { split: node.split, children } : { split: node.split, children, ratio: Math.max(0.1, Math.min(0.9, ratio)) }
    }
    const rows = node.tabs.filter(tab => {
      const identity = paneIdentity(tab.pane)
      if (tab.pane === '' || seen.has(identity)) return false
      seen.add(identity)
      return true
    })
    if (rows.length === 0) return undefined
    const id = unused(groupIds, ...rows.map(tab => localTabs.get(paneIdentity(tab.pane))?.group.id), node.id)
    const tabs = rows.map((row): Tab => {
      const tabId = unused(tabIds, localTabs.get(paneIdentity(row.pane))?.tab.id, row.id)
      return row.title === undefined ? { id: tabId, pane: row.pane } : { id: tabId, pane: row.pane, title: row.title }
    })
    const local = localGroups.find(group => group.id === id)
    const shown = tabs.find(tab => tab.id === local?.active)?.id ?? tabs.find(tab => tab.id === node.active)?.id
    const position = local?.tabs.findIndex(tab => tab.id === local.active) ?? 0
    return { id, tabs, active: shown ?? tabs[Math.min(Math.max(position, 0), tabs.length - 1)]!.id }
  }
  const layout = walk(received) ?? { id: state.focus === '' ? rootGroup : state.focus, tabs: [], active: null }
  const groups = groupsOf(layout)
  const previous = activeTab(state)?.id
  const focus = groups.find(group => group.id === state.focus)?.id
    ?? groups.find(group => group.tabs.some(tab => tab.id === previous))?.id
    ?? groups[0]!.id
  const ids = new Set(tabsOf(layout).map(tab => tab.id))
  const shown = activeTab({ layout, focus })?.id
  // A stored document can repeat an id; recency keeps its first, most recent, mention.
  const known = [...new Set(state.mru)].filter(id => ids.has(id))
  const recent = shown === undefined ? known : touch(known, shown)
  return {
    ...state,
    layout,
    focus,
    mru: [...recent, ...[...ids].filter(id => !recent.includes(id))],
    views: Object.fromEntries(Object.entries(state.views).filter(([id]) => ids.has(id))),
  }
}

/** A stored document can name anything: restores the machine's invariants on what it says. */
export const repairWorkspace = (state: WorkspaceState, context: TransitionContext): WorkspaceState => adopt(state, state.layout, context)

export const transition = (state: WorkspaceState, command: Command, context: TransitionContext): WorkspaceState => {
  switch (command._tag) {
    case 'Open':
      return open(state, command, context)
    case 'Activate': {
      const found = locate(state.layout, tab => tab.id === command.id)
      return found === undefined ? state : reveal(state, found.group.id, found.tab.id)
    }
    case 'FocusGroup': {
      const group = groupsOf(state.layout).find(row => row.id === command.group)
      if (group === undefined || group.id === state.focus) return state
      return { ...state, focus: group.id, mru: group.active === null ? state.mru : touch(state.mru, group.active) }
    }
    case 'Cycle': {
      const group = focusedGroup(state)
      if (group === undefined || group.tabs.length === 0) return state
      const index = group.tabs.findIndex(tab => tab.id === group.active)
      const next = (Math.max(index, 0) + command.by + group.tabs.length) % group.tabs.length
      return reveal(state, group.id, group.tabs[next]!.id)
    }
    case 'ActivateIndex': {
      const group = focusedGroup(state)
      const tab = group?.tabs[command.index]
      return group === undefined || tab === undefined ? state : reveal(state, group.id, tab.id)
    }
    case 'ActivatePrevious': {
      const shown = activeTab(state)?.id
      const previous = state.mru.find(id => id !== shown)
      const found = previous === undefined ? undefined : locate(state.layout, tab => tab.id === previous)
      return found === undefined ? state : reveal(state, found.group.id, found.tab.id)
    }
    case 'Close':
      return close(state, command.id)
    case 'ReopenClosed': {
      const pane = state.closed[0]
      return pane === undefined ? state : open(state, { _tag: 'Open', pane }, context)
    }
    case 'Move':
      return move(state, command.id, command.to, context)
    case 'SetRatio': {
      const layout = setRatio(state.layout, command.node, Math.max(0.1, Math.min(0.9, command.ratio)))
      return layout === state.layout ? state : { ...state, layout }
    }
    case 'SetView': {
      if (locate(state.layout, tab => tab.id === command.id) === undefined) return state
      const current = state.views[command.id]
      const next = { ...current, ...command.view }
      if (current !== undefined && Object.keys(next).every(key => current[key as keyof TabView] === next[key as keyof TabView])) return state
      return { ...state, views: { ...state.views, [command.id]: next } }
    }
    case 'SetPanel': {
      const next = { ...state.panel, ...command.panel }
      return next.open === state.panel.open && next.tab === state.panel.tab && next.width === state.panel.width ? state : { ...state, panel: next }
    }
    case 'Receive': {
      const adopted = adopt(state, command.layout, context)
      return sameStructure(adopted.layout, state.layout) ? state : adopted
    }
  }
}
