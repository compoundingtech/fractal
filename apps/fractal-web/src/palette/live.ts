import * as React from 'react'
import { RegistryContext, useAtom, useAtomValue } from '@effect/atom-react'
import * as Atom from 'effect/reactivity/Atom'
import * as DateTime from 'effect/DateTime'
import * as Option from 'effect/Option'
import type * as Fractal from '@fractal/client/schema/read'
import { agentDescriptor, missionDescriptor, type EntityStatus } from '@fractal/ui/assistant-ui'
import type { RecentStore } from '@fractal/ui/workspace/palette'
import { isWaitingForAnswer } from '../data/attention.ts'
import { useAskingAgents, useDataSource, useSubjectIndex } from '../data/react.tsx'
import { missionChildOf } from '../missions/rowData.ts'
import { useMissionRows, type MissionRow } from '../missions/useMissionRows.ts'
import { sidebarAttention } from '../web/sidebarAttention.ts'
import { sidebarRow } from '../web/sidebarRow.ts'
import { workspaceKinds } from '../workspace/kinds.ts'
import type { Fleet } from '../data/source.ts'
import type { SubjectSummary } from '../shell/context.tsx'
import { paneKey, parsePane, subjectOfRef } from '../workspace/subject.ts'
import type { SaidSearch } from './CommandPalette.tsx'
import { devBarAvailable, toggleDevBar } from '../telemetry/devBarPreference.ts'
import { systemEventsPreference } from '../web/conversationPreferences.ts'
import { schemePreference, type SchemePreference } from '../web/themePreference.ts'
import { emptyWorld, type HostCommand, type PaletteWorld } from './items.ts'
import { cellRecentStore } from './recent.ts'

const needsYou: EntityStatus = { tone: 'needs-you', label: 'Waiting for you' }

const resourceKind = (ref: string, icon: string): string =>
  /\/pr\/|\/pull\//u.test(ref) || icon === 'pull-request' ? 'pull request'
    : /\/issues?\//u.test(ref) ? 'issue'
      : /\/runs?\//u.test(ref) ? 'run'
        : ref.startsWith('doc/') ? 'document'
          : 'resource'

/**
 * What the gateway observed, as palette rows' input. A family not yet observed contributes nothing.
 * Agent and mission status come from the kit's descriptors over the sidebar's own rows, so a palette
 * row can never disagree with the sidebar ring.
 */
export const paletteWorld = ({ fleet, stale = false, needsMe = new Set(), missionRows = [], attention, resources = [] }: {
  readonly fleet?: Fleet | undefined
  /** The roster is the retained last observation. */
  readonly stale?: boolean | undefined
  /** Agents the sidebar marks as needing the person (`sidebarAttention`). */
  readonly needsMe?: ReadonlySet<string> | undefined
  /** The missions as the sidebar and Missions screen present them (`useMissionRows`). */
  readonly missionRows?: readonly MissionRow[] | undefined
  readonly attention?: readonly Fractal.Attention[] | undefined
  readonly resources?: readonly Pick<SubjectSummary, 'ref' | 'title' | 'icon'>[] | undefined
}): PaletteWorld => ({
  alerts: (attention ?? []).filter(item => item.state === 'open').map(item => ({
    ref: item.id, title: item.title, kind: item.attention_kind, pane: paneKey(subjectOfRef(item.id)),
    ...(isWaitingForAnswer(item) ? { status: needsYou } : {}),
  })),
  agents: (fleet?.agents ?? []).map(agent => {
    const { status } = agentDescriptor(sidebarRow({ agent, stale, needsMe: needsMe.has(agent.ref) }))
    return {
      ref: agent.ref, name: agent.name, detail: [agent.harness, agent.host].filter(part => part !== undefined && part !== '').join(' · '),
      terminal: agent.terminalDisabledReason === undefined, ...(status === undefined ? {} : { status }),
    }
  }),
  missions: missionRows.map(row => {
    const { title, status } = missionDescriptor(missionChildOf(row))
    return { ref: row.mission.id, title, word: typeof row.mission.state === 'string' ? row.mission.state : row.mission.state.raw, ...(status === undefined ? {} : { status }) }
  }),
  machines: (fleet?.hosts ?? []).map(host => ({ id: host.id, connected: host.connected })),
  resources: resources.map(resource => ({ ref: resource.ref, title: resource.title, kind: resourceKind(resource.ref, resource.icon), pane: paneKey(subjectOfRef(resource.ref)) })),
})

/**
 * Whether this client has a surface for a pane key: one the kind registry resolves to anything but
 * Unsupported. Rows for the rest (the fleet list, a machine) stay hidden until a kind claims them.
 */
export const opensInWeb = (ref: string): boolean => workspaceKinds.kindOf(parsePane(ref)).id !== 'Unsupported'

const valueOf = <A,>(feed: { readonly _tag: string; readonly value?: A }): A | undefined => feed._tag === 'Observed' ? feed.value : undefined

/** Fixtures and scenarios have no signed-in person; their recent list belongs to this browser. */
const browserPerson = Atom.make<string | undefined>('local')

/**
 * The palette's live inputs from the data source: the observed world, st's conversation search when
 * this gateway has it, and the recent list per person and gateway.
 */
export const useLivePalette = (): {
  readonly world: PaletteWorld
  readonly search: SaidSearch | undefined
  readonly recentStore: RecentStore | undefined
  readonly canOpen: (ref: string) => boolean
} => {
  const source = useDataSource()
  const index = useSubjectIndex()
  const asking = useAskingAgents()
  const { rows: missionRows } = useMissionRows()
  const registry = React.useContext(RegistryContext)
  const person = useAtomValue(source.person ?? browserPerson)
  const gateway = source.gateway ?? source.mode
  const fleet = valueOf(index.families.agents)
  const stale = index.families.agents._tag === 'Observed' && index.families.agents.freshness === 'stale'
  const attention = valueOf(index.families.attention)
  // The sidebar's own needs-you set, from the same inputs.
  const needsMe = React.useMemo(() => sidebarAttention({ agents: fleet?.agents ?? [], subjects: index.subjects, asking }), [fleet, index.subjects, asking])
  const world = React.useMemo(
    () => fleet === undefined && missionRows.length === 0 && attention === undefined && index.resources === undefined
      ? emptyWorld
      : paletteWorld({ fleet, stale, needsMe, missionRows, attention, resources: index.resources }),
    [fleet, stale, needsMe, missionRows, attention, index.resources],
  )
  const port = source.contentSearch
  const search = React.useMemo((): SaidSearch | undefined => port === undefined ? undefined : async query => {
    const result = await port.search({ text: query, limit: 20 })
    if (result._tag !== 'Success') return undefined
    const found = result.value
    return {
      refreshing: found.refreshing,
      incomplete_sources: found.incomplete_sources,
      items: found.items.map(hit => ({ agent_id: Option.getOrNull(hit.agent_id), conversation_id: hit.conversation_id, excerpt: hit.excerpt, timestamp: DateTime.formatIso(hit.timestamp) })),
    }
  }, [port])
  const recentStore = React.useMemo(() => person === undefined ? undefined : cellRecentStore({ registry, person, gateway }), [registry, person, gateway])
  return { world, search, recentStore, canOpen: opensInWeb }
}

/** What only the mount site can do; each present one becomes a palette command. */
export interface HostActions {
  readonly openSettings?: (() => void) | undefined
  readonly toggleSidebar?: (() => void) | undefined
  /** The new mission form, in a new tab. */
  readonly newMission?: (() => void) | undefined
  /** A plain shell in a new tab. */
  readonly newTerminal?: (() => void) | undefined
  /** Sidebar modes and previews, as the TUI's sidebar section offers them. */
  readonly sidebar?: readonly HostCommand[] | undefined
}

const schemes: readonly SchemePreference[] = ['dark', 'light', 'system']

/**
 * The palette's commands: the theme, the conversation and devbar toggles (their own preferences, the
 * ones Settings writes), and what the host passes in. Stable while nothing changes, as the palette needs.
 */
export const useLiveHostCommands = (actions: HostActions = {}): readonly HostCommand[] => {
  const [scheme, setScheme] = useAtom(schemePreference)
  const [systemEvents, setSystemEvents] = useAtom(systemEventsPreference)
  const { openSettings, toggleSidebar, newMission, newTerminal, sidebar } = actions
  return React.useMemo((): readonly HostCommand[] => [
    ...(newMission === undefined ? [] : [{ id: 'new-mission', section: 'start' as const, label: 'New mission', detail: 'describe it; a planner drafts it for you to approve', search: 'new mission launch', run: newMission }]),
    ...(newTerminal === undefined ? [] : [{ id: 'new-terminal', section: 'start' as const, label: 'New terminal', detail: 'a shell in a new tab', search: 'new terminal shell', icon: 'terminal' as const, run: newTerminal }]),
    ...schemes.filter(each => each !== scheme).map(each => ({
      id: `theme-${each}`, section: 'commands' as const, label: `Use ${each === 'system' ? 'the system' : each} theme`, detail: `now ${scheme}`,
      search: `theme appearance scheme ${each}`, icon: 'swap' as const, run: () => setScheme(each),
    })),
    {
      id: 'system-events', section: 'commands', label: systemEvents ? 'Hide system events in conversations' : 'Show system events in conversations',
      search: 'system events conversations toggle', icon: 'sliders', run: () => setSystemEvents(!systemEvents),
    },
    ...(devBarAvailable() ? [{ id: 'devbar', section: 'commands' as const, label: 'Toggle devbar', search: 'devbar dev tools toggle', icon: 'gauge' as const, shortcut: 'Mod+Shift+B', run: toggleDevBar }] : []),
    ...(openSettings === undefined ? [] : [{ id: 'settings', section: 'commands' as const, label: 'Settings', search: 'settings preferences', icon: 'gear' as const, run: openSettings }]),
    ...(toggleSidebar === undefined ? [] : [{ id: 'toggle-sidebar', section: 'commands' as const, label: 'Toggle sidebar', search: 'sidebar toggle hide show', icon: 'panel-left' as const, run: toggleSidebar }]),
    ...(sidebar ?? []),
  ], [scheme, setScheme, systemEvents, setSystemEvents, openSettings, toggleSidebar, newMission, newTerminal, sidebar])
}
