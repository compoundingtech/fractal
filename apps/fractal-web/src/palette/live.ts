import * as React from 'react'
import { RegistryContext, useAtom, useAtomValue } from '@effect/atom-react'
import * as Atom from 'effect/reactivity/Atom'
import * as DateTime from 'effect/DateTime'
import * as Option from 'effect/Option'
import type * as Fractal from '@fractal/client/schema/read'
import type { EntityStatus } from '@fractal/ui/assistant-ui'
import type { RecentStore } from '@fractal/ui/workspace/palette'
import { isWaitingForAnswer } from '../data/attention.ts'
import { attentionRefs } from '../data/projections.ts'
import { useDataSource, useSubjectIndex } from '../data/react.tsx'
import type { Fleet } from '../data/source.ts'
import type { SubjectSummary } from '../shell/context.tsx'
import { paneKey, subjectOfRef } from '../workspace/subject.ts'
import type { SaidSearch } from './CommandPalette.tsx'
import { devBarAvailable, toggleDevBar } from '../telemetry/devBarPreference.ts'
import { systemEventsPreference } from '../web/conversationPreferences.ts'
import { schemePreference, type SchemePreference } from '../web/themePreference.ts'
import { emptyWorld, type HostCommand, type PaletteWorld } from './items.ts'
import { cellRecentStore } from './recent.ts'

const needsYou: EntityStatus = { tone: 'needs-you', label: 'Waiting for you' }

const agentStatus = (agent: Fleet['agents'][number], waiting: boolean): EntityStatus | undefined =>
  waiting ? needsYou
    : !agent.connected ? { tone: 'neutral', label: 'Unavailable' }
      : agent.activity === 'working' ? { tone: 'working', label: 'Working' }
        : agent.activity === 'errored' ? { tone: 'failed', label: 'Errored' }
          : undefined

const resourceKind = (ref: string, icon: string): string =>
  /\/pr\/|\/pull\//u.test(ref) || icon === 'pull-request' ? 'pull request'
    : /\/issues?\//u.test(ref) ? 'issue'
      : /\/runs?\//u.test(ref) ? 'run'
        : ref.startsWith('doc/') ? 'document'
          : 'resource'

/** What the gateway observed, as palette rows' input. A family not yet observed contributes nothing. */
export const paletteWorld = ({ fleet, missions, attention, resources = [] }: {
  readonly fleet?: Fleet | undefined
  readonly missions?: readonly Fractal.Mission[] | undefined
  readonly attention?: readonly Fractal.Attention[] | undefined
  readonly resources?: readonly Pick<SubjectSummary, 'ref' | 'title' | 'icon'>[] | undefined
}): PaletteWorld => {
  const waiting = attentionRefs((attention ?? []).filter(isWaitingForAnswer))
  return {
    alerts: (attention ?? []).filter(item => item.state === 'open').map(item => ({
      ref: item.id, title: item.title, kind: item.attention_kind, pane: paneKey(subjectOfRef(item.id)),
      ...(isWaitingForAnswer(item) ? { status: needsYou } : {}),
    })),
    agents: (fleet?.agents ?? []).map(agent => {
      const status = agentStatus(agent, waiting.has(agent.ref))
      return {
        ref: agent.ref, name: agent.name, detail: [agent.harness, agent.host].filter(part => part !== undefined && part !== '').join(' · '),
        terminal: agent.terminalDisabledReason === undefined, ...(status === undefined ? {} : { status }),
      }
    }),
    missions: (missions ?? []).map(mission => ({
      ref: mission.id, title: mission.title, word: typeof mission.state === 'string' ? mission.state : mission.state.raw,
      ...(waiting.has(mission.id) ? { status: needsYou } : {}),
    })),
    machines: (fleet?.hosts ?? []).map(host => ({ id: host.id, connected: host.connected })),
    resources: resources.map(resource => ({ ref: resource.ref, title: resource.title, kind: resourceKind(resource.ref, resource.icon), pane: paneKey(subjectOfRef(resource.ref)) })),
  }
}

const valueOf = <A,>(feed: { readonly _tag: string; readonly value?: A }): A | undefined => feed._tag === 'Observed' ? feed.value : undefined

/** Fixtures and scenarios have no signed-in person; their recent list belongs to this browser. */
const browserPerson = Atom.make<string | undefined>('local')

/**
 * The palette's live inputs from the data source: the observed world, st's conversation search when
 * this gateway has it, and the recent list per person and gateway.
 */
export const useLivePalette = (): { readonly world: PaletteWorld; readonly search: SaidSearch | undefined; readonly recentStore: RecentStore | undefined } => {
  const source = useDataSource()
  const index = useSubjectIndex()
  const registry = React.useContext(RegistryContext)
  const person = useAtomValue(source.person ?? browserPerson)
  const gateway = source.gateway ?? source.mode
  const fleet = valueOf(index.families.agents)
  const missions = valueOf(index.families.missions)
  const attention = valueOf(index.families.attention)
  const world = React.useMemo(
    () => fleet === undefined && missions === undefined && attention === undefined && index.resources === undefined ? emptyWorld : paletteWorld({ fleet, missions, attention, resources: index.resources }),
    [fleet, missions, attention, index.resources],
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
  return { world, search, recentStore }
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
