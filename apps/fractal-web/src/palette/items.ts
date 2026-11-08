import type { ComponentProps } from 'react'
import type { CompositionIcon, EntityDescriptor, EntityStatus } from '@fractal/ui/assistant-ui'
import { activeTab, currentSpace, panesOf, type Command, type WorkspaceState } from '@fractal/ui/workspace'
import type { Leads, PaletteItem, PaletteSection } from '@fractal/ui/workspace/palette'
import type { NamingIntent } from './dispatch.ts'

/**
 * The palette's rows, built from plain observations so they test without a gateway. Sections and their
 * order follow the TUI (crates/fractal-tui/src/ui/glass.rs `SECTIONS`, `RANK`, `choices`); resources,
 * commands and recent are the web's own.
 */

export type IconName = ComponentProps<typeof CompositionIcon>['name']

export const paletteSections: readonly PaletteSection[] = [
  { id: 'alerts', label: 'Alerts', prefix: '!' },
  { id: 'agents', label: 'Agents', prefix: '@' },
  { id: 'missions', label: 'Missions', prefix: '#' },
  { id: 'fleet', label: 'Fleet' },
  { id: 'spaces', label: 'Spaces' },
  { id: 'start', label: 'Start' },
  { id: 'said', label: 'Said in conversations' },
  { id: 'sidebar', label: 'Sidebar' },
  { id: 'resources', label: 'Resources', prefix: '/' },
  { id: 'commands', label: 'Commands', prefix: '>' },
  { id: 'recent', label: 'Recent' },
]

/** The empty palette leads with what was used last; the rest keep the TUI's order. */
export const emptyOrder: readonly string[] = ['recent']

/** The TUI's `RANK`: agents, missions, what needs you, what was said, starting things, fleet, spaces, sidebar. */
export const typedOrder: readonly string[] = ['agents', 'missions', 'resources', 'alerts', 'said', 'start', 'commands', 'fleet', 'spaces', 'sidebar']

/** What choosing a row does. */
export type PaletteAction =
  /** Opens a TUI pane key. */
  | { readonly _tag: 'Open'; readonly ref: string }
  /** Opens a conversation where `query` was said. */
  | { readonly _tag: 'Said'; readonly ref: string; readonly query: string }
  | { readonly _tag: 'Workspace'; readonly commands: readonly Command[] }
  /** Asks for a name, for a space to make, rename or copy. */
  | { readonly _tag: 'Name'; readonly intent: NamingIntent }
  | { readonly _tag: 'Run'; readonly run: () => void }

export interface PaletteEntry {
  readonly action: PaletteAction
  /** What Recent remembers it by; rows that are not worth remembering have none. */
  readonly key?: string | undefined
  readonly entity?: EntityDescriptor | undefined
  readonly icon?: IconName | undefined
  /** A keyboard hint shown at the row's end, such as ⌘D. */
  readonly shortcut?: string | undefined
}

export type PaletteRow = PaletteItem<PaletteEntry>

const row = (fields: Omit<PaletteRow, 'action'>, entry: PaletteEntry): PaletteRow => ({ ...fields, action: entry })
const open = (ref: string, extra: Omit<PaletteEntry, 'action' | 'key'> = {}): PaletteEntry => ({ action: { _tag: 'Open', ref }, key: ref, ...extra })

export interface PaletteWorld {
  readonly alerts: readonly { readonly ref: string; readonly title: string; readonly kind: string; readonly status?: EntityStatus | undefined }[]
  readonly agents: readonly {
    readonly ref: string
    readonly name: string
    readonly detail: string
    readonly status?: EntityStatus | undefined
    /** The agent has a terminal to attach. */
    readonly terminal?: boolean | undefined
  }[]
  readonly missions: readonly { readonly ref: string; readonly title: string; readonly word: string; readonly status?: EntityStatus | undefined }[]
  readonly machines: readonly { readonly id: string; readonly connected: boolean }[]
  /** Pull requests, issues, runs and documents this gateway observed. */
  readonly resources: readonly { readonly ref: string; readonly title: string; readonly kind: string; readonly status?: EntityStatus | undefined }[]
}

export const emptyWorld: PaletteWorld = { alerts: [], agents: [], missions: [], machines: [], resources: [] }

const withStatus = (status: EntityStatus | undefined) => status === undefined ? {} : { status }

/** Alerts, agents, missions, the fleet and resources. An agent's terminal shows once something is typed. */
export const worldItems = (world: PaletteWorld, query = ''): readonly PaletteRow[] => {
  const typed = query.trim() !== ''
  return [
    row({ id: 'list:home', section: 'alerts', label: 'Inbox', detail: 'alerts and updates', search: 'inbox home' }, open('list:home', { icon: 'bell' })),
    row({ id: 'usage:', section: 'alerts', label: 'Usage', detail: 'spend by agent, mission, step, model, account or host', search: 'usage spend tokens cost' }, open('usage:', { icon: 'gauge' })),
    ...world.alerts.map(alert => {
      const ref = `home:${alert.ref}`
      return row({ id: ref, section: 'alerts', label: alert.title, detail: alert.kind, search: `${alert.title} ${ref}` },
        open(ref, { icon: 'alert', entity: { kind: 'resource', id: alert.ref, title: alert.title, ...withStatus(alert.status) } }))
    }),
    ...world.agents.map(agent => {
      const ref = `agent:${agent.ref}`
      return row({ id: ref, section: 'agents', label: agent.name, detail: agent.detail, search: `${agent.name} ${ref}` },
        open(ref, { icon: 'message', entity: { kind: 'agent', id: agent.ref, title: agent.name, ...withStatus(agent.status) } }))
    }),
    ...(typed ? world.agents.filter(agent => agent.terminal === true).map(agent => {
      const ref = `terminal:${agent.ref}`
      return row({ id: ref, section: 'agents', label: `${agent.name} terminal`, detail: 'its terminal, attached', search: `${agent.name} terminal ${ref}` }, open(ref, { icon: 'terminal' }))
    }) : []),
    ...world.missions.map(mission => {
      const ref = `mission:${mission.ref}`
      return row({ id: ref, section: 'missions', label: mission.title, detail: mission.word, search: `${mission.title} ${ref}` },
        open(ref, { icon: 'checklist', entity: { kind: 'mission', id: mission.ref, title: mission.title, ...withStatus(mission.status) } }))
    }),
    row({ id: 'list:fleet', section: 'fleet', label: 'Fleet', detail: 'every machine, its clients and this person’s devices', search: 'fleet machines clients devices' }, open('list:fleet', { icon: 'globe' })),
    ...world.machines.map(machine => {
      const ref = `machine:machine/${machine.id}`
      return row({ id: ref, section: 'fleet', label: machine.id, detail: machine.connected ? 'reachable' : 'unreachable', search: `${machine.id} ${ref}` }, open(ref, { icon: 'globe' }))
    }),
    ...world.resources.map(resource => {
      const ref = `resource:${resource.ref}`
      return row({ id: ref, section: 'resources', label: resource.title, detail: resource.kind, search: `${resource.title} ${resource.kind} ${resource.ref}` },
        open(ref, { icon: resource.kind === 'pull request' ? 'pull-request' : 'folder', entity: { kind: 'resource', id: resource.ref, title: resource.title, ...withStatus(resource.status) } }))
    }),
  ]
}

/** A command the host owns: theme, settings, a toggle, a new terminal or mission, a sidebar mode. */
export interface HostCommand {
  readonly id: string
  readonly section: 'start' | 'commands' | 'sidebar'
  readonly label: string
  readonly detail?: string | undefined
  readonly search?: string | undefined
  readonly icon?: IconName | undefined
  readonly shortcut?: string | undefined
  readonly run: () => void
}

/** The host's commands, and the workspace's own: split, zoom, close. */
export const commandItems = ({ state, commands }: { readonly state: WorkspaceState; readonly commands: readonly HostCommand[] }): readonly PaletteRow[] => {
  const space = currentSpace(state)
  const tab = activeTab(space)
  const split = panesOf(space.layout).length > 1
  const workspace = (id: string, label: string, search: string, icon: IconName, shortcut: string, commands: readonly Command[]) =>
    row({ id, section: 'commands', label, search: `${label} ${search}` }, { action: { _tag: 'Workspace', commands }, key: id, icon, shortcut })
  return [
    ...commands.map(command => row(
      { id: command.id, section: command.section, label: command.label, detail: command.detail, search: command.search ?? command.label },
      { action: { _tag: 'Run', run: command.run }, key: command.id, icon: command.icon ?? (command.section === 'start' ? 'plus' : 'sliders'), shortcut: command.shortcut },
    )),
    ...(tab === undefined ? [] : [
      workspace('split-right', 'Split right', 'split vertical pane', 'panel', 'Mod+D', [{ _tag: 'Split', side: 'right', tab: tab.id }]),
      workspace('split-down', 'Split down', 'split horizontal pane below', 'panel', 'Mod+Shift+D', [{ _tag: 'Split', side: 'bottom', tab: tab.id }]),
      workspace('close-tab', 'Close tab', 'close tab', 'x', 'Alt+W', [{ _tag: 'Close', tab: tab.id }]),
    ]),
    ...(split ? [workspace('zoom', space.local.zoomed === null ? 'Zoom split' : 'Unzoom split', 'zoom maximize split', 'panel', 'Alt+O', [{ _tag: 'ToggleZoom' }])] : []),
    ...(space.local.closed.length > 0 ? [workspace('reopen', 'Reopen closed tab', 'reopen closed tab undo', 'swap', 'Alt+Shift+T', [{ _tag: 'ReopenClosed' }])] : []),
  ]
}

/**
 * Spaces to switch to, and naming rows for the current one, always there so making spaces is easy to
 * find. With a name typed, rows that use it directly; their search is the query, so they always show.
 */
export const spaceItems = (state: WorkspaceState, query: string): readonly PaletteRow[] => {
  const current = currentSpace(state)
  const name = query.trim()
  const space = (id: string, label: string, detail: string, search: string, icon: IconName, action: PaletteAction, key?: string) =>
    row({ id, section: 'spaces', label, detail, search }, { action, key, icon })
  return [
    ...state.spaces.map(each => space(`space:${each.id}`, each.name, each.id === current.id ? 'shown' : 'switch', each.name, 'drawer',
      { _tag: 'Workspace', commands: [{ _tag: 'SpaceSwitch', space: each.id }] }, `space:${each.id}`)),
    space('space-rename', `Rename “${current.name}”…`, '', 'rename space glass', 'pencil', { _tag: 'Name', intent: 'rename' }),
    space('space-new', 'New space…', '', 'new space glass', 'plus', { _tag: 'Name', intent: 'create' }),
    space('space-duplicate', `Duplicate “${current.name}”…`, '', 'duplicate space glass copy', 'copy', { _tag: 'Name', intent: 'duplicate' }),
    ...(name === '' ? [] : [
      space('space-new-named', `New space “${name}”`, 'Alt+G switches', query, 'plus', { _tag: 'Workspace', commands: [{ _tag: 'SpaceNew', name }] }),
      space('space-rename-named', `Rename “${current.name}” to “${name}”`, '', query, 'pencil', { _tag: 'Workspace', commands: [{ _tag: 'SpaceRename', space: current.id, name }] }),
      space('space-duplicate-named', `Duplicate “${current.name}” as “${name}”`, '', query, 'copy', { _tag: 'Workspace', commands: [{ _tag: 'SpaceDuplicate', space: current.id, name }] }),
    ]),
    ...(state.spaces.length > 1 ? [space('space-close', `Close “${current.name}”`, 'its tabs go with it', 'close space glass', 'x', { _tag: 'Workspace', commands: [{ _tag: 'SpaceClose', space: current.id }] })] : []),
  ]
}

/** The one row naming mode offers: what Enter does with the typed name. */
export const namingItem = (state: WorkspaceState, intent: NamingIntent, name: string): string => {
  const current = currentSpace(state).name
  switch (intent) {
    case 'create': return `New space “${name}”`
    case 'rename': return `Rename “${current}” to “${name}”`
    case 'duplicate': return `Duplicate “${current}” as “${name}”`
  }
}

export interface SaidHit {
  readonly agent_id: string | null
  readonly conversation_id: string
  readonly excerpt: string
  readonly timestamp: string
}

/**
 * st's conversation search as rows, at most twelve. While the index refreshes or missed conversations,
 * the first row says so. Their search is the query, so they show for exactly what they answer.
 */
export const saidItems = (query: string, found: { readonly items: readonly SaidHit[]; readonly refreshing?: boolean; readonly incomplete_sources?: readonly string[] }, names: ReadonlyMap<string, string>): readonly PaletteRow[] => {
  const caveat = found.refreshing === true ? 'still indexing, more may come · ' : (found.incomplete_sources?.length ?? 0) > 0 ? 'some conversations not searched · ' : ''
  return found.items.slice(0, 12).map((hit, index) => {
    const agent = hit.agent_id ?? hit.conversation_id
    const name = names.get(agent) ?? agent.replace(/^agent\//u, '')
    const excerpt = hit.excerpt.split(/\s+/u).filter(word => word !== '').join(' ')
    const at = (hit.timestamp.slice(5, 16) || hit.timestamp).replace('T', ' ')
    return row(
      { id: `said:${agent}:${index}`, section: 'said', label: excerpt.length > 90 ? `${excerpt.slice(0, 89)}…` : excerpt, detail: `${index === 0 ? caveat : ''}${name} · ${at}`, search: query },
      { action: { _tag: 'Said', ref: `agent:${agent}`, query }, icon: 'message' },
    )
  })
}

/** Recent rows repeat what was chosen last, under their own ids, and only for the empty query. */
export const recentItems = (recent: readonly string[], items: readonly PaletteRow[]): readonly PaletteRow[] => {
  const byKey = new Map(items.flatMap(item => item.action.key === undefined ? [] : [[item.action.key, item] as const]))
  return recent.flatMap(key => {
    const item = byKey.get(key)
    return item === undefined ? [] : [{ ...item, id: `recent:${item.id}`, section: 'recent', search: '' }]
  })
}

/**
 * The TUI's first ranking term: a command typed as it begins ("new mission", "fleet", "inbox") comes
 * before anything that merely matches.
 */
export const leads: Leads<PaletteEntry> = (item, query) => {
  const command = item.action.action._tag !== 'Open' || ['list:home', 'usage:', 'list:fleet'].includes(item.id)
  return command && ['start', 'commands', 'spaces', 'sidebar', 'alerts', 'fleet'].includes(item.section) && item.label.toLowerCase().startsWith(query.trim().toLowerCase())
}
