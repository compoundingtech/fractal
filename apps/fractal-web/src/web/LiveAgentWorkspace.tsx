import { selectedFaviconState, tabTitle, useTabMetadata } from './useTabMetadata.ts'
import { cancelMeasure, markStart, measureEnd } from './perfMarks.ts'
import { prefetchChangelog } from '../changelog/load.ts'
import { agentSeatChildren, sidebarMissionChild, sidebarRow, sidebarSubagentRow, subagentRowId, sidebarMissionFeedsAtom } from './sidebarRow.ts'
import { subagentThreadFocus } from './subagentThreadFocus.ts'
import { ConversationHeaderActions } from './ConversationHeaderActions.tsx'
import { DeclarationDialog } from './DeclarationDialog.tsx'
import { AgentActions } from './AgentActions.tsx'
import type * as SidebarMissionsListModule from '../missions/SidebarMissionsList.tsx'
import { alertTabMarkers } from './alertsTab.ts'
import { missionTabMarkersAtom } from './missionsTab.ts'
import { activeTab, focusedGroup, showsTerminal, tabsOf, type Command, type TabView, type WorkspaceState as WorkspaceStateValue } from '../workspace/model.ts'
import { OpenProvider, openCommand, type Open } from '../workspace/openable.ts'
import { paneKey, parsePane, subjectAgent, subjectOfRef, subjectRef } from '../workspace/subject.ts'
import { AlertsTabCodeBoundary } from '../workspace/surfaces.tsx'
import { prefetchConversationPane, prefetchTerminalDetail } from '../workspace/chunks.ts'
import { tabPresentation } from '../workspace/tabPresentation.tsx'
import { useWorkspaceResources, useWorkspaceRoute, useWorkspaceState, useWorkspaceStore, type WorkspaceNavigation } from '../workspace/useWorkspace.ts'
import { useWorkspaceKeys } from '../workspace/useWorkspaceKeys.ts'
import { WorkspaceBody } from '../workspace/WorkspaceBody.tsx'
import { WorkspaceEmpty, WorkspaceStrip } from '../workspace/WorkspaceStrip.tsx'
import { narrowLayoutQuery } from './narrowLayout.ts'
import { rightPanelAtom, type RightPanel } from './rightPanel.ts'
import { AlertFlashBand } from './AlertFlashBand.tsx'
import { useVisibleAttention } from '../conversation/alerts/useAlertAnswers.ts'
import { LiveAlertNames } from '../conversation/alerts/LiveAlertNames.tsx'
import { ThreadHeaderSlotContext } from '../shell/threadHeaderSlot.tsx'
import { liveLegacyTheme } from '../ui-compat/live-theme.stylex.ts'
import { colorVars as c, typeVars as t, spaceVars as s, geometryVars as g, geometryNumbers, sidebarRowSize } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import * as React from 'react'
import { RegisteredPortal, usePortalContainerRegistration } from '../../../../packages/fractal-ui/src/assistant-ui/taste/PortalContainer.tsx'
import * as Aria from 'react-aria-components'
import { UNSAFE_PortalProvider } from 'react-aria'
import * as stylex from '@stylexjs/stylex'
import { useAtom, useAtomValue } from '@effect/atom-react'
import { Schema } from 'effect'
import * as Atom from 'effect/reactivity/Atom'
import { ChromeControl } from '../../../../packages/fractal-ui/src/assistant-ui/composition/ChromeControl.tsx'
import { MilestoneAgentRow } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/MilestoneAgentRow.tsx'
import { SidebarMissionChildRow, missionChildAccessibleName } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/SidebarMissionLine.tsx'
import { SidebarQueryContext, sidebarRowAccessibleName } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/rowPresentation.tsx'
import { SystemNotice, ConnectionStatus } from '../../../../packages/fractal-ui/src/assistant-ui/composition/SystemState.tsx'
import { ThreadHeader, PhoneTerminalToggle, ResizableSplit } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Shell.tsx'
import { DockedSplit } from '../../../../packages/fractal-ui/src/assistant-ui/composition/DockedSplit.tsx'
import { NarrowSheet } from '../../../../packages/fractal-ui/src/assistant-ui/composition/NarrowLayout.tsx'
import { darkTheme as assistantDarkTheme, liveComposerDarkTheme, liveAccentTheme } from '../../../../packages/fractal-ui/src/assistant-ui/embrace-theme.ts'
import { lightTheme as compositionLightTheme } from '../../../../packages/fractal-ui/src/assistant-ui/composition-theme.ts'
import type { SidebarActions } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/model.ts'
import { PinsSection } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/PinsSection.tsx'
import type { WorkLogCall } from '@fractal/ui/assistant-ui'
import { terminalSubjectForAgent } from '../data/projections.ts'
import { sidebarPins } from '../data/sidebarPins.ts'
import { useFleet, useAgent, useSubjectList, useConnection, useNow, useNowSnapshot, useDataSource, useRosterDecode, useAskingAgents } from '../data/react.tsx'
import { persistedAtom } from '../state/persistence.ts'
import { leftSidebar } from '../state/cells/panels.ts'
import { setTheme } from '../ui-compat/foundations.ts'
import { WorkbenchContextProvider, ResourcePanelProvider, MonitorDetailProvider, type OpenRequest } from '../shell/context.tsx'
import type { ResourcePanelState } from '../shell/state.ts'
import { afterNextPaint, type UxTelemetry } from '../telemetry/ux.ts'
import { formatDuration } from '@fractal/ui/human-time/format'
import { useGatewaySync } from '../data/react.tsx'
import { NarrowLayoutContext } from '../../../../packages/fractal-ui/src/assistant-ui/composition/NarrowLayout.tsx'
import { PhoneAgentDrawer, type PhoneAgentDrawerHandle } from './PhoneAgentDrawer.tsx'
import { AgentFolders } from '../shell/AgentFolders.tsx'
import { useSidebarFolders, type SidebarFoldersBinding } from '../folders/useSidebarFolders.ts'
import { fixtureFolders } from '../folders/fixture.ts'
import { defaultFilters, fixtureSidebarState, sidebarState } from '../shell/sidebar/state.ts'
import { useMachineMemory, withRememberedMachines } from '../shell/sidebar/show.ts'
import { parseSidebarQuery } from '../shell/sidebar/filter.ts'
import { SidebarToolbar, type SidebarToolbarProps } from '../shell/sidebar/Toolbar.tsx'
import { credentialNotice, type Agent } from '../data/source.ts'
import { sidebarAttention } from './sidebarAttention.ts'
import { FolderAttentionSignal } from './AttentionControls.tsx'
import { liveShowRows, liveSidebarTree } from './liveSidebarTree.ts'
import { RosterLoading, RosterStaleNotice, RosterUnavailable, SidebarEmptyState } from './SidebarEmptyState.tsx'
import { useAgentsLastObservedAt } from './lastObserved.tsx'
import { ResponsiveAgentSidebar, sidebarMinimumWidth, useSidebarShortcut, useSidebarNavigation } from './ResponsiveAgentSidebar.tsx'
import { SidebarTabs } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/SidebarTabs.tsx'
import { LiveUsageSection } from '../monitor/UsageSection.tsx'
import { SettingsControl } from './SettingsDialog.tsx'
import type { SettingsBuild } from './settingsModel.ts'
import { syncLine } from '../../../../packages/fractal-ui/src/assistant-ui/sync-presentation/sync-line.ts'
import { DecodeNotice, shownRosterRefs } from './LiveRoster.tsx'
import { schemePreference, useResolvedScheme } from './themePreference.ts'

const emptyAgents: readonly Agent[] = []


/** A last-observed roster's observation instant: its row times stop there instead of ticking on as if live. */
const RosterFrozenAt = React.createContext<number | undefined>(undefined)

/** Clock ticks reach SidebarTime leaves, never the Aria row or its collection. */
const LiveSidebarAgentRow = React.memo((props: {
  readonly agentRef: string
  readonly childId?: string
  readonly active: boolean
  readonly needsMe: boolean
  readonly openRef?: string
  readonly touchDisclosure?: React.ReactNode
  readonly select: (ref: string) => void
  readonly openMission?: (mission: string) => void
  readonly actions: SidebarActions
}) => {
  const feed = useAgent(props.agentRef)
  const source = useDataSource()
  const missionAtom = React.useMemo(() => sidebarMissionFeedsAtom(source), [source])
  const mission = useAtomValue(missionAtom)
  const connection = useConnection()
  const agent = feed._tag === 'Observed' ? feed.value : undefined
  const stale = feed._tag === 'Observed' && (feed.freshness === 'stale' || connection._tag !== 'Live')
  const frozenAt = React.useContext(RosterFrozenAt)
  const [, requestThreadFocus] = useAtom(subagentThreadFocus(props.agentRef))
  const item = React.useMemo(() => {
    if (agent === undefined) return undefined
    if (props.childId === undefined) return sidebarRow({ agent, stale, needsMe: props.needsMe, ...mission })
    const child = agent.subagents?.find(child => child.id === props.childId)
    return child === undefined ? undefined : sidebarSubagentRow({ parent: agent, child, stale })
  }, [agent, stale, props.needsMe, props.childId, mission])
  const onOpen = React.useCallback(() => { if (props.openRef !== undefined) props.select(props.openRef) }, [props.openRef, props.select])
  const onSubagentsPress = React.useCallback(() => {
    props.select(props.agentRef)
    requestThreadFocus(value => value + 1)
  }, [props.select, props.agentRef, requestThreadFocus])
  return item === undefined ? null : <MilestoneAgentRow row={item} reportedStatusOnly displayOnly={props.childId !== undefined} active={props.active} frozen={stale} now={frozenAt} touchDisclosure={props.touchDisclosure} onOpenMission={props.childId === undefined ? props.openMission : undefined} actions={props.childId === undefined ? props.actions : undefined} onOpen={props.openRef === undefined ? undefined : onOpen} onSubagentsPress={props.childId === undefined ? onSubagentsPress : undefined} />
})
LiveSidebarAgentRow.displayName = 'LiveSidebarAgentRow'

/** One further current mission of an agent, as its own 48px tree row. */
const LiveSidebarMissionChild = React.memo((props: { readonly agentRef: string; readonly missionId: string; readonly label: string; readonly active: boolean }) => {
  const feed = useAgent(props.agentRef)
  const source = useDataSource()
  const missionAtom = React.useMemo(() => sidebarMissionFeedsAtom(source), [source])
  const { missions, attention, missionInfo } = useAtomValue(missionAtom)
  const connection = useConnection()
  const agent = feed._tag === 'Observed' ? feed.value : undefined
  if (agent === undefined) return null
  const mission = sidebarMissionChild({ agent, missionId: props.missionId, missions, attention })
  return <SidebarMissionChildRow mission={{ ...mission, title: mission.title ?? props.label }} active={props.active} frozen={feed._tag === 'Observed' && (feed.freshness === 'stale' || connection._tag !== 'Live')} retained={missionInfo._tag !== 'Live'} />
})
LiveSidebarMissionChild.displayName = 'LiveSidebarMissionChild'

const sameSeatChildren = (previous: ReturnType<typeof agentSeatChildren>, next: ReturnType<typeof agentSeatChildren>): boolean =>
  (previous?.length ?? 0) === (next?.length ?? 0) && (next ?? []).every((child, index) => previous?.[index]?.id === child.id && previous[index]?.label === child.label)

const compactAgentHeight = { compact: sidebarRowSize }
const displayOnlyAgentHeight = { compact: sidebarRowSize, touch: sidebarRowSize }

const LiveSidebarTreeView = React.memo((props: {
  readonly binding: SidebarFoldersBinding
  readonly tree: SidebarFoldersBinding['tree']
  readonly agents: readonly Agent[]
  readonly stale: boolean
  readonly needsYou: ReadonlySet<string>
  readonly current: string
  readonly query: string
  readonly revealKey: string
  readonly select: (ref: string) => void
  readonly openMission: (mission: string) => void
  /** The mission the route shows, for its child row's selection. */
  readonly currentMission?: string
  readonly actions: SidebarActions
}) => {
  const source = useDataSource()
  const previousAccessibleNames = React.useRef<ReadonlyMap<string, string>>(new Map())
  const namesAtom = React.useMemo(() => {
    const mission = sidebarMissionFeedsAtom(source)
    return Atom.make(get => {
      const observation = props.agents.some(agent => (agent.currentWork?.length ?? 0) > 0 || agent.mission !== undefined)
        ? get(mission)
        : undefined
      const next = new Map(props.agents.flatMap(agent => [
        [agent.ref, sidebarRowAccessibleName(sidebarRow({ agent, stale: props.stale, needsMe: props.needsYou.has(agent.ref), ...observation }))] as const,
        ...(agent.subagents ?? []).map(child => [subagentRowId({ parent: agent.ref, id: child.id }), sidebarRowAccessibleName(sidebarSubagentRow({ parent: agent, child, stale: props.stale }))] as const),
        ...(agentSeatChildren(agent) ?? []).flatMap(child => child.missionId === undefined ? [] : [[child.id, missionChildAccessibleName({ ...sidebarMissionChild({ agent, missionId: child.missionId, missions: observation?.missions, attention: observation?.attention }), title: child.label })] as const]),
      ]))
      const previous = previousAccessibleNames.current
      if (previous.size === next.size && [...next].every(([ref, name]) => previous.get(ref) === name)) return previous
      previousAccessibleNames.current = next
      return next
    })
  }, [source, props.agents, props.stale, props.needsYou])
  const accessibleNames = useAtomValue(namesAtom)
  const agentRowHeight = React.useCallback((node: Extract<SidebarFoldersBinding['tree'][number], { readonly _tag: 'Agent' }>) =>
    accessibleNames.has(node.id) ? (node.parent === undefined ? compactAgentHeight : displayOnlyAgentHeight) : undefined, [accessibleNames])
  const renderAgent = React.useCallback((node: Extract<SidebarFoldersBinding['tree'][number], { readonly _tag: 'Agent' }>, touchDisclosure?: React.ReactNode) =>
    node.missionId !== undefined ? <LiveSidebarMissionChild agentRef={node.subject} missionId={node.missionId} label={node.label} active={props.currentMission === node.missionId} /> : <LiveSidebarAgentRow agentRef={node.parent ?? node.subject} childId={node.childId} active={node.parent === undefined && props.current === node.subject} needsMe={node.parent === undefined && props.needsYou.has(node.subject)} openRef={node.parent === undefined ? node.subject : undefined} touchDisclosure={touchDisclosure} select={props.select} openMission={props.openMission} actions={props.actions} />, [props.current, props.currentMission, props.needsYou, props.select, props.openMission, props.actions])
  const renderFolderSignal = React.useCallback((folder: Exclude<SidebarFoldersBinding['tree'][number], { readonly _tag: 'Agent' }>) =>
    <FolderAttentionSignal folder={folder} refs={props.needsYou} />, [props.needsYou])
  return <SidebarQueryContext.Provider value={props.query}><AgentFolders {...props.binding} tree={props.tree} accessibleNames={accessibleNames} selectedId={props.current} revealKey={props.revealKey} onSelect={props.select} onOpenMission={props.openMission} renderFolderSignal={renderFolderSignal} renderAgent={renderAgent} agentRowHeight={agentRowHeight} /></SidebarQueryContext.Provider>
})
LiveSidebarTreeView.displayName = 'LiveSidebarTreeView'

// Details, its measured resource tabs and Changes import the rich kit; keep them outside the eager shell.
const AgentDetailsPanel = React.lazy(() => import('./AgentDetailsPanel.tsx').then(module => ({ default: module.AgentDetailsPanel })))
const SidebarMissionsList = (props: React.ComponentProps<typeof SidebarMissionsListModule.SidebarMissionsList>) => <React.Suspense fallback={<SystemNotice kind="loading" title="Loading missions" />}><LazySidebarMissionsList {...props} /></React.Suspense>
const LazySidebarMissionsList = React.lazy(() => import('../missions/SidebarMissionsList.tsx').then((module) => ({ default: module.SidebarMissionsList })))

// The sidebar's open state and width live in one boot cell so the inline script paints them on the first frame.
const sidebarRatio = Atom.writable((get) => get(leftSidebar).ratio, (ctx, ratio: number) => { ctx.set(leftSidebar, { ...ctx.get(leftSidebar), ratio }) })
const panelRatio = persistedAtom({ key: 'round2.panelRatio', schema: Schema.Number, defaultValue: 380 / 1440 })
const sidebarClosed = Atom.writable((get) => !get(leftSidebar).open, (ctx, closed: boolean) => { ctx.set(leftSidebar, { ...ctx.get(leftSidebar), open: !closed }) })
const viewportWidth = () => window.innerWidth
const subscribeViewport = (callback: () => void) => {
  window.addEventListener('resize', callback)
  return () => window.removeEventListener('resize', callback)
}
// Mobile keyboards resize the visual viewport, not necessarily the layout viewport.
const phoneViewportHeight = () => window.visualViewport?.height ?? window.innerHeight
const subscribePhoneViewport = (callback: () => void) => {
  window.visualViewport?.addEventListener('resize', callback)
  window.addEventListener('resize', callback)
  return () => { window.visualViewport?.removeEventListener('resize', callback); window.removeEventListener('resize', callback) }
}
const narrowLayoutMatches = () => window.matchMedia(narrowLayoutQuery).matches
const serverNarrow = () => false
const locationState = () => `${window.location.pathname}${window.location.search}`
const subscribeLocation = (callback: () => void) => {
  window.addEventListener('popstate', callback)
  return () => window.removeEventListener('popstate', callback)
}

/** The app uses browser history; static embeds supply a private in-memory location. */
export type { WorkspaceNavigation }
const browserNavigation: WorkspaceNavigation = {
  getLocation: locationState,
  subscribe: subscribeLocation,
  navigate: (href) => {
    window.history.pushState(null, '', href)
    window.dispatchEvent(new PopStateEvent('popstate'))
  },
  replace: (href) => {
    window.history.replaceState(null, '', href)
    window.dispatchEvent(new PopStateEvent('popstate'))
  },
}
interface WorkspaceProps {
  readonly ux?: UxTelemetry
  readonly onSelectConversation?: (ref: string) => void
  readonly dimensions?: { readonly width: number; readonly height: number }
  readonly navigation?: WorkspaceNavigation
  readonly persistGeometry?: boolean
  readonly workspaceLabel?: string
  /** The running build, from the app entry: stories and the embed have no build identity. */
  readonly build?: SettingsBuild
  /** What an empty workspace with no subject in its URL starts on: a story's or an embed's known state. */
  readonly initialRef?: string
}

/**
 * React Aria overlays (menus, popovers, tooltips, modals) portal into a host inside the themed
 * root instead of document.body, where only the legacy document palette and fonts apply. The
 * provider sits above the shell state, so its context value never changes on a switch and the
 * portalled panes inherit it without a context scan.
 */
export function LiveAgentWorkspace(props: WorkspaceProps) {
  const overlayHost = React.useRef<HTMLDivElement>(null)
  const getContainer = React.useCallback(() => overlayHost.current, [])
  usePortalContainerRegistration(getContainer)
  // Alerts name their senders as the roster does, in the docked card and the alert pane alike.
  return <UNSAFE_PortalProvider getContainer={getContainer}><LiveAlertNames><Workspace {...props} overlayHost={overlayHost} /></LiveAlertNames></UNSAFE_PortalProvider>
}

const scopeHash = (scope: string) => {
  let hash = 0x811c9dc5
  for (let index = 0; index < scope.length; index += 1) hash = Math.imul(hash ^ scope.charCodeAt(index), 0x01000193)
  return (hash >>> 0).toString(36)
}

const activeAgentOf = (store: { readonly getState: () => WorkspaceStateValue }) => {
  const tab = activeTab(store.getState())
  return tab === undefined ? undefined : subjectAgent(parsePane(tab.pane))
}

/** One workspace of typed tabs; local geometry and tabs never grant backend authority. */
function Workspace({ ux, onSelectConversation, dimensions, navigation = browserNavigation, persistGeometry = true, workspaceLabel = 'Agent workspace', build, initialRef, overlayHost }: WorkspaceProps & { readonly overlayHost: React.RefObject<HTMLDivElement | null> }) {
  const fleet = useFleet(), subjects = useSubjectList(), connection = useConnection(), rosterDecode = useRosterDecode()
  const [ratio, setRatio] = useAtom(sidebarRatio), [collapsed, setCollapsed] = useAtom(sidebarClosed), [preference, setPreference] = useAtom(schemePreference)
  const agentsDrawer = React.useRef<PhoneAgentDrawerHandle>(null)
  const subscribeNarrow = React.useCallback((callback: () => void) => {
    const query = window.matchMedia(narrowLayoutQuery)
    query.addEventListener('change', callback)
    return () => query.removeEventListener('change', callback)
  }, [])
  const windowNarrow = React.useSyncExternalStore(subscribeNarrow, narrowLayoutMatches, serverNarrow)
  const narrow = dimensions === undefined ? windowNarrow : dimensions.width < 900
  const scheme = useResolvedScheme(preference)
  React.useLayoutEffect(() => setTheme(preference), [preference])
  const sidebarOpener = React.useRef<HTMLButtonElement>(null)
  const windowWidth = React.useSyncExternalStore(subscribeViewport, viewportWidth, viewportWidth)
  const windowHeight = React.useSyncExternalStore(subscribePhoneViewport, phoneViewportHeight, phoneViewportHeight)
  const viewport = dimensions?.width ?? windowWidth
  const phoneHeight = dimensions?.height ?? windowHeight
  const phoneViewportStyle = narrow ? { height: `calc(${phoneHeight}px - var(--wf-devbar-space, 0px))`, '--wf-phone-height': `${phoneHeight}px` } : undefined
  const toggleSidebar = React.useCallback(() => {
    if (narrow) agentsDrawer.current?.toggle()
    else setCollapsed(value => !value)
  }, [narrow, setCollapsed])
  useSidebarShortcut(toggleSidebar)
  const location = React.useSyncExternalStore(navigation.subscribe, navigation.getLocation, navigation.getLocation)
  const workspaceSource = useDataSource()
  const workspaceScope = `${workspaceSource.gateway ?? workspaceSource.mode}${new URLSearchParams(location.split('?')[1]).get('scenario') ?? ''}`
  // Storage keys never carry a connection's identity; a hash keeps gateways and replays apart.
  const store = useWorkspaceStore({ storageKey: persistGeometry ? `fractal-web.workspace.v1/${scopeHash(workspaceScope)}` : undefined, location, initialRef })
  const workspace = useWorkspaceState(store)
  const route = useWorkspaceRoute({ store, state: workspace, location, navigation })
  const tiers = useWorkspaceResources({ store, state: workspace })
  const activeRow = activeTab(workspace)
  const activeSubject = React.useMemo(() => activeRow === undefined ? undefined : parsePane(activeRow.pane), [activeRow?.pane])
  const hoverOwner = React.useRef<HTMLElement | null>(null)
  const rememberHoverOwner = (event: React.SyntheticEvent<HTMLElement>) => {
    hoverOwner.current = event.target instanceof Element ? event.target.closest<HTMLElement>('[data-wf-agent-ref]') : null
  }
  React.useLayoutEffect(() => {
    // Use the row's existing Escape contract: it cancels pending hover intent as well
    // as closing the card, without replacing any roster nodes during a switch.
    hoverOwner.current?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    hoverOwner.current = null
    agentsDrawer.current?.close()
  }, [location])
  const agents = fleet._tag === 'Observed' ? fleet.value.agents : emptyAgents
  const rosterObserved = fleet._tag === 'Observed'
  const shellCommit = React.useCallback((node: HTMLDivElement | null) => {
    if (node === null) return
    const cancelCapture = ux?.shellCommitted()
    // The shared paint scheduler posts a task after the first shell frame, not a second rAF.
    // A selected pane requests code immediately through lazy; the empty shell warms it here.
    const cancelPrefetch = afterNextPaint(prefetchConversationPane)
    return () => { cancelCapture?.(); cancelPrefetch() }
  }, [ux])
  const rosterRef = React.useRef<HTMLElement | null>(null)
  const rosterCommit = React.useCallback((node: HTMLElement | null) => {
    rosterRef.current = node
    if (node === null || !rosterObserved) return
    const cancelTiming = measureEnd('sidebar_interactive')
    const cancelCapture = ux?.rosterCommitted()
    let cancelIdle: (() => void) | undefined
    const cancelPaint = afterNextPaint(() => {
      if (typeof window.requestIdleCallback === 'function') {
        const idle = window.requestIdleCallback(prefetchChangelog)
        cancelIdle = () => window.cancelIdleCallback(idle)
      } else {
        const timer = window.setTimeout(prefetchChangelog, 0)
        cancelIdle = () => window.clearTimeout(timer)
      }
    })
    return () => { cancelTiming(); cancelCapture?.(); cancelPaint(); cancelIdle?.() }
  }, [ux, rosterObserved])
  /** The agent the active tab is about; panels, the header and the roster selection follow it. */
  const current = subjectAgent(activeSubject) ?? ''
  const agent = agents.find((row) => row.ref === current)
  // A canonical subject address is not proof of a terminal. Keep the control disabled
  // from its first render until the roster can establish the selected agent's runtime.
  const terminalDisabledReason = terminalSubjectForAgent(current) === undefined ? undefined
    : credentialNotice(fleet) ?? (fleet._tag === 'Waiting' ? 'Waiting for the live agent roster to find this agent’s terminal.'
      : fleet._tag === 'Unavailable' ? 'The agent roster is unavailable, so this agent’s terminal cannot be found.'
        : agent === undefined ? 'This agent is not in the live agent roster, so it has no terminal.'
          : agent.terminalDisabledReason)
  const terminalRef = terminalDisabledReason === undefined ? agent?.terminal : undefined
  const terminalSource = useDataSource()
  // After first paint, idle time warms the terminal chunk and fonts; data waits for explicit intent.
  React.useLayoutEffect(() => {
    if (terminalRef === undefined) return
    let cancelIdle: (() => void) | undefined
    const cancelPaint = afterNextPaint(() => {
      if (typeof window.requestIdleCallback === 'function') {
        const idle = window.requestIdleCallback(prefetchTerminalDetail)
        cancelIdle = () => window.cancelIdleCallback(idle)
      } else {
        const timer = window.setTimeout(prefetchTerminalDetail, 0)
        cancelIdle = () => window.clearTimeout(timer)
      }
    })
    return () => { cancelPaint(); cancelIdle?.() }
  }, [terminalRef])
  const prefetchTerminalIntent = (event: React.SyntheticEvent<HTMLElement>) => {
    if (terminalRef === undefined || !(event.target instanceof Element) ||
      event.target.closest('[aria-label="Show terminal"]') === null) return
    prefetchTerminalDetail()
    terminalSource.prefetchTerminal?.(terminalRef)
  }
  const [headerSlot, setHeaderSlot] = React.useState<HTMLDivElement | null>(null)
  const [paneHost, setPaneHost] = React.useState<HTMLDivElement | null>(null)
  const [resourcesAgent, setResourcesAgent] = React.useState<string>()
  const [resourcePanel, setResourcePanel] = React.useState<ResourcePanelState>({ expanded: false, size: 288 })
  // The panel is one app-wide setting on desktop, so it stays as it is across tabs and windows. A phone's sheet closes on a switch instead.
  const [panel, writePanel] = useAtom(rightPanelAtom)
  const setPanel = React.useCallback((change: Partial<RightPanel>) => writePanel(value => ({ ...value, ...change })), [writePanel])
  const [sheetOpen, setSheetOpen] = React.useState(false)
  const [openedTool, setOpenedTool] = React.useState<WorkLogCall | undefined>(undefined)
  // A tool detail belongs to its agent and a phone's sheet to what was showing, however the tab changed: Back counts.
  const shownTab = activeRow?.id
  React.useLayoutEffect(() => setSheetOpen(false), [shownTab])
  React.useLayoutEffect(() => setOpenedTool(undefined), [current])
  /** Every workspace change from a person; transcript timing starts here. */
  const lastAgent = React.useRef(activeAgentOf(store))
  const go = React.useCallback((command: Command) => {
    route(command)
    const after = activeAgentOf(store)
    if (after === undefined || after === lastAgent.current) return
    lastAgent.current = after
    cancelMeasure('open_transcript')
    markStart('switch_to_tail', { key: after })
    onSelectConversation?.(after)
  }, [store, route, onSelectConversation])
  // One verb for every surface: the subject decides the kind, and the machine opens or focuses its one tab.
  const openRef: Open = React.useCallback((ref, gesture) => {
    agentsDrawer.current?.close()
    go(openCommand({ ref, focus: store.getState().focus, ...(gesture === undefined ? {} : { gesture }) }))
  }, [go, store])
  const open = React.useCallback((request: OpenRequest) => openRef(request.ref), [openRef])
  const openAlert = React.useCallback((id: string) => openRef(`alert/${id}`), [openRef])
  const openInbox = React.useCallback(() => openRef('inbox'), [openRef])
  useWorkspaceKeys({ dispatch: go, getState: store.getState, openInbox })
  const onTabView = React.useCallback((id: string, view: TabView) => store.dispatch({ _tag: 'SetView', id, view }), [store])
  const onReplaceTab = React.useCallback((id: string, ref: string) => go({ _tag: 'Replace', id, pane: paneKey(subjectOfRef(ref)) }), [go])
  const onCloseTab = React.useCallback((id: string) => go({ _tag: 'Close', id }), [go])
  const onOpenTool = React.useCallback((call: WorkLogCall) => { setOpenedTool(call) }, [])
  const pins = useAtomValue(terminalSource.pins ?? sidebarPins())
  const pinned = React.useMemo(() => new Set(pins.state.members.map(item => item.member)), [pins.state.members])
  const pinnedItems = React.useMemo(() => {
    const names = new Map(agents.map(agent => [agent.ref, agent.name]))
    return pins.state.members.map(({ member }) => ({ id: member, label: names.get(member) ?? member }))
  }, [agents, pins.state.members])
  const sidebarActions = React.useMemo<SidebarActions>(() => ({
    declaration: (ref: string) => openRef(`declaration/${ref}`),
    ...(pins.editable ? { pins: { has: (ref: string) => pinned.has(ref), toggle: pins.toggle } } : {}),
  }), [pins.editable, pins.toggle, pinned, openRef])
  const [panelFraction, setPanelFraction] = useAtom(panelRatio)
  const [dragWidth, setDragWidth] = React.useState<number | undefined>()
  const [dragPanel, setDragPanel] = React.useState<number | undefined>()
  const stale = fleet._tag === 'Observed' && (fleet.freshness === 'stale' || connection._tag !== 'Live')
  const rosterObservedAt = useAgentsLastObservedAt()
  const frozenAt = stale ? rosterObservedAt : undefined
  const folderSource = useDataSource()
  const sidebarSettings = (folderSource.mode === 'fixtures' ? fixtureSidebarState : sidebarState)(folderSource.gateway ?? folderSource.mode)
  const [filters, setFilters] = useAtom(sidebarSettings.filters)
  const [sidebarTab, setSidebarTab] = useAtom(sidebarSettings.tab)
  const attention = useVisibleAttention()
  const showSidebarTab = React.useCallback((tab: 'agents' | 'alerts' | 'missions') => {
    setSidebarTab(tab)
    if (narrow) agentsDrawer.current?.open()
    else setCollapsed(false)
  }, [setSidebarTab, narrow, setCollapsed])
  const shortcutHelp = useSidebarNavigation(showSidebarTab)
  const backToAlerts = React.useCallback(() => showSidebarTab('alerts'), [showSidebarTab])
  const missionMarkers = useAtomValue(React.useMemo(() => missionTabMarkersAtom(folderSource), [folderSource]))
  const machineMemory = useMachineMemory()
  const asking = useAskingAgents()
  // The query is the only Show state; live gateways start from this browser's remembered machines.
  React.useLayoutEffect(() => {
    const query = withRememberedMachines({ query: filters.query, remembered: machineMemory.remembered })
    if (query !== filters.query) setFilters({ ...filters, query })
    // eslint-disable-next-line react-hooks/exhaustive-deps -- seed once per gateway's filters, not on every edit.
  }, [sidebarSettings.filters])
  const hosts = React.useMemo(() => [...new Set(agents.map(row => row.host))].toSorted(), [agents])
  const updateFilters = React.useCallback((value: typeof filters) => {
    setFilters(value)
    machineMemory.remember(value.query, hosts)
  }, [setFilters, machineMemory, hosts])
  // Typing stays urgent; filtering and the tree collection follow at transition priority.
  const deferredFilters = React.useDeferredValue(filters)
  const sidebarQuery = React.useMemo(() => parseSidebarQuery(deferredFilters.query), [deferredFilters.query])
  // Activity frames change row presentation, not folder membership or its measured layout.
  const previousRoster = React.useRef<Parameters<typeof useSidebarFolders>[0]['roster'] | undefined>(undefined)
  const roster = React.useMemo(() => {
    const previous = previousRoster.current
    if (previous !== undefined && previous.length === agents.length && agents.every((agent, index) => {
      const row = previous[index]!
      return row.id === agent.ref && row.subject === agent.ref && row.label === agent.name && row.host === agent.host &&
        row.project === (agent.checkout._tag === 'Known' ? agent.checkout.value.repository : undefined) &&
        sameSeatChildren(row.children, agentSeatChildren(agent))
    })) return previous
    const next = agents.map(row => ({
      id: row.ref, subject: row.ref, label: row.name, host: row.host,
      project: row.checkout._tag === 'Known' ? row.checkout.value.repository : undefined,
      children: agentSeatChildren(row),
    }))
    previousRoster.current = next
    return next
  }, [agents])
  const sidebarFolders = useSidebarFolders({
    roster,
    query: deferredFilters.query,
    ...(folderSource.mode === 'fixtures' ? { source: fixtureFolders } : {}),
    collapsedSource: sidebarSettings.collapsed,
  })
  const previousNeedsYou = React.useRef<ReadonlySet<string> | undefined>(undefined)
  const needsYou = React.useMemo(() => {
    const next = sidebarAttention({ agents, subjects })
    const previous = previousNeedsYou.current
    if (previous !== undefined && next.size === previous.size && [...next].every(ref => previous.has(ref))) return previous
    previousNeedsYou.current = next
    return next
  }, [agents, subjects])
  const offline = connection._tag !== 'Live'
  useTabMetadata({
    title: tabTitle({ agentName: agent?.name, needsYouCount: needsYou.size, offline }),
    faviconState: selectedFaviconState({ agent, needsYou: agent !== undefined && needsYou.has(agent.ref), offline }),
  })
  // Filtering uses observed status, not wall time; ticking cannot change membership.
  const visibleSidebarTree = React.useMemo(() => liveSidebarTree({ tree: sidebarFolders.orderingTree, agents, needsYou, asking, filters: deferredFilters, stale }), [sidebarFolders.orderingTree, agents, needsYou, asking, deferredFilters, stale])
  const showRows = React.useMemo(() => liveShowRows({ agents, needsYou, asking, filters: deferredFilters, stale }), [agents, needsYou, asking, deferredFilters, stale])
  const selectLatest = React.useRef(openRef)
  selectLatest.current = openRef
  const selectAgent = React.useCallback((ref: string) => {
    if (previousRoster.current?.some(seat => seat.children?.some(child => child.id === ref))) return
    selectLatest.current(ref)
  }, [])
  const byRef = React.useMemo(() => new Map(subjects.map((subject) => [subject.ref, subject])), [subjects])
  const allTabs = React.useMemo(() => tabsOf(workspace.layout), [workspace.layout])
  const stripTabs = focusedGroup(workspace)?.tabs ?? allTabs
  const tabItems = React.useMemo(() => stripTabs.map(tab => ({ ...tabPresentation({ tab, agents, rosterKnown: fleet._tag === 'Observed', needsYou, attention, subjects: byRef }), ...(workspace.views[tab.id]?.dirty === true ? { dirty: true } : {}) })), [stripTabs, workspace.views, agents, fleet._tag, needsYou, attention, byRef])
  const activeLabel = tabItems.find(item => item.id === activeRow?.id)?.title
  const previousNames = React.useRef<ReadonlyMap<string, string>>(new Map())
  const names = React.useMemo(() => {
    const next = new Map(agents.map(row => [row.ref, row.name]))
    const previous = previousNames.current
    if (previous.size === next.size && [...next].every(([ref, name]) => previous.get(ref) === name)) return previous
    previousNames.current = next
    return next
  }, [agents])
  const maxSidebar = Math.max(sidebarMinimumWidth, Math.min(440, viewport - 640))
  const width = Math.min(maxSidebar, Math.max(sidebarMinimumWidth, dragWidth ?? Math.round(ratio * viewport)))
  const maxPanel = Math.max(240, viewport - (narrow || collapsed ? 0 : width) - 640)
  const panelSize = dragPanel ?? Math.min(maxPanel, Math.max(240, Math.round(panelFraction * viewport)))
  const terminalOpen = activeRow !== undefined && current !== '' && showsTerminal(activeRow, workspace.views[activeRow.id])
  // The control stays enabled while the terminal is shown: it is the way back, even from a deep link to a terminal this agent lacks.
  const shownTerminalDisabledReason = terminalOpen ? undefined : terminalDisabledReason
  const focusShownFace = React.useRef(false)
  const toggleTerminal = () => {
    if (activeRow === undefined || current === '' || (!terminalOpen && terminalRef === undefined)) return
    focusShownFace.current = true
    store.dispatch({ _tag: 'SetView', id: activeRow.id, view: { terminal: !terminalOpen } })
  }
  // Details, Changes and Resources are the panel's tabs; which one shows is part of the one app-wide setting.
  const dockedInspector = !narrow && panel.open && openedTool === undefined && current !== '' ? current : undefined
  const dockLabel = panel.tab === 'resources' ? 'Resize Resources' : panel.tab === 'changes' ? 'Resize Changes' : 'Resize Details'
  const agentHeaderActions = current === '' ? undefined : <>{agent === undefined ? null : <AgentActions key={`actions-${current}`} agent={agent} />}<ConversationHeaderActions key={current} agentRef={current} onViewDeclaration={() => openRef(`declaration/${current}`)} /></>
  // Below the breakpoint Details is a sheet over the thread, never an inline panel beside it.
  const narrowDetailsOpen = narrow && current !== '' && (sheetOpen || openedTool !== undefined)
  const closeDetails = () => { setSheetOpen(false); setOpenedTool(undefined) }
  const openDetailsUri = (uri: string) => {
    const ref = uri.startsWith('agent:') ? `agent/${uri.slice('agent:'.length)}` : uri
    open({ ref })
  }
  const toolDetail = (call: WorkLogCall, width: number | undefined) => <aside aria-label="Tool detail" style={width === undefined ? undefined : { width }} {...stylex.props(styles.changesPanel, width === undefined && styles.sheetPanel)}>
    <header {...stylex.props(styles.toolDetailHeader)}>
      <strong {...stylex.props(styles.toolDetailTitle)}>{call.title}{call.argsSummary === undefined ? '' : ` ${call.argsSummary}`}</strong>
      <Aria.Button aria-label="Close tool detail" onPress={() => setOpenedTool(undefined)} {...stylex.props(styles.iconButton)}>×</Aria.Button>
    </header>
    <pre {...stylex.props(styles.toolDetailOutput)}>{call.detail ?? (call.status === 'running' ? 'No output received yet.' : 'No output recorded.')}</pre>
  </aside>
  const sidebarHeaderActions = <>
    <SettingsControl scheme={preference} onSchemeChange={setPreference} build={build} />
  </>
  // The Agents toolbar and tree provide tab stops even while loading or empty; Alerts may have none, so its panel takes focus itself.
  const sidebarBody = <SidebarTabs panelHasTabbableContent={sidebarTab === 'agents' || sidebarTab === 'fleet'} selectedId={sidebarTab === 'alerts' || sidebarTab === 'missions' ? sidebarTab : 'agents'} onSelectionChange={id => setSidebarTab(id === 'alerts' || id === 'missions' ? id : 'agents')} tabs={[{ id: 'agents', label: 'Agents', icon: 'person', count: agents.length, questionCount: agents.filter(agent => agent.ask._tag === 'Known' && agent.ask.value === 'question').length, needsYouCount: needsYou.size }, { id: 'alerts', label: 'Alerts', icon: 'bell', ...alertTabMarkers(attention) }, { id: 'missions', label: 'Missions', icon: 'checklist', ...missionMarkers }]}>
    {sidebarTab === 'missions' ? <SidebarMissionsList {...(activeSubject?._tag === 'Mission' ? { current: activeSubject.ref } : {})} onOpen={ref => openRef(ref ?? 'missions')} /> : sidebarTab === 'alerts' ? <AlertsTabCodeBoundary {...(activeSubject?._tag === 'Alert' ? { selected: activeSubject.id } : {})} onSelect={openAlert} onOpen={ref => openRef(ref)} /> : <>
    <LiveSidebarToolbar filters={filters} update={updateFilters} hosts={hosts} showRows={showRows} onEnterResults={() => rosterRef.current?.querySelector<HTMLElement>('[role="row"]')?.focus()} />
    {credentialNotice(fleet) ? <RosterUnavailable reason="unauthorized" onRetry={undefined} /> : fleet._tag === 'Unavailable' ? <RosterUnavailable reason={fleet.reason} onRetry={fleet.reason === 'failed' && fleet.retryable !== false ? folderSource.retryAgents : undefined} />
      : fleet._tag !== 'Observed' ? null
      : <>
        {stale ? <RosterStaleNotice decodeStale={rosterDecode.decodeStale} observedAt={rosterObservedAt} partial={fleet.coverage?._tag === 'Partial'} refreshFailed={fleet.error?.reason === 'failed'} onRetry={folderSource.retryAgents} /> : null}
        {agents.length === 0 ? <SidebarEmptyState kind="empty" />
          : visibleSidebarTree.length === 0 ? <SidebarEmptyState kind="no-match" query={deferredFilters.query} onClear={() => setFilters({ ...defaultFilters, sort: filters.sort })} />
          : null}
      </>}
    <nav ref={rosterCommit} aria-label="Agent roster" onMouseOverCapture={rememberHoverOwner} onFocusCapture={rememberHoverOwner} {...stylex.props(styles.roster)}>
      <PinsSection items={pinnedItems} editable={pins.editable} onOpen={ref => open({ ref })} onUnpin={pins.toggle} detail={pins.state.phase === 'unavailable' ? 'Pins are unavailable. Reconnect to make changes.' : undefined} onRetry={pins.retry} />
      {fleet._tag === 'Waiting' ? <>
        <RosterLoading />
        <div aria-hidden="true" data-wf-roster-skeleton="waiting" {...stylex.props(styles.rosterSkeleton)}>
          {Array.from({ length: 8 }, (_, index) => <div key={index} data-wf-skeleton-row="" {...stylex.props(styles.skeletonRow)}>
            <span {...stylex.props(styles.skeletonGlyph)} />
            <span data-wf-skeleton-line="name" {...stylex.props(styles.skeletonLine, styles.skeletonName)} />
            <span data-wf-skeleton-line="host" {...stylex.props(styles.skeletonLine, styles.skeletonHost)} />
          </div>)}
        </div>
      </> : null}
      <RosterFrozenAt.Provider value={frozenAt}>
        <LiveSidebarTreeView binding={sidebarFolders} tree={visibleSidebarTree} agents={agents} stale={stale} needsYou={needsYou} current={current} query={sidebarQuery.text} revealKey={deferredFilters.query} select={selectAgent} openMission={openRef} {...(activeSubject?._tag === 'Mission' ? { currentMission: activeSubject.ref } : {})} actions={sidebarActions} />
      </RosterFrozenAt.Provider>
    </nav>
    {fleet._tag === 'Observed' ? <DecodeNotice noun="agent" decode={rosterDecode} shown={shownRosterRefs(fleet)} /> : null}
    </>}
    <LiveUsageSection onOpenUsage={() => openRef('monitor/quota')} />
  </SidebarTabs>
  // The panes stay outside changing shell providers, whose context propagation would scan every retained turn.
  // Only the stable header DOM host and narrow geometry cross that boundary.
  return (<OpenProvider value={openRef}><NarrowLayoutContext.Provider value={narrow}>
    <ThreadHeaderSlotContext.Provider value={narrow ? null : headerSlot}>
    <WorkbenchContextProvider value={{ open, focusedRef: activeSubject === undefined ? null : subjectRef(activeSubject), subjects: byRef, platform: navigator.platform.includes('Mac') ? 'mac' : 'other' }}>
      <ResourcePanelProvider value={{ state: resourcePanel, onChange: (change) => setResourcePanel((value) => ({ ...value, ...change })) }}>
        <MonitorDetailProvider value={{ size: 380, onSizeChange: (value) => setPanelFraction(value / viewport) }}>
            <div ref={shellCommit} data-testid="live-agent-workspace" data-scheme={scheme} style={phoneViewportStyle} {...stylex.props(styles.app, styles.legacyBridge, liveLegacyTheme, scheme === 'dark' && assistantDarkTheme, scheme === 'dark' && liveComposerDarkTheme, liveAccentTheme, scheme === 'light' && compositionLightTheme)}>
              {shortcutHelp}
              {narrow ? null : <ResponsiveAgentSidebar headerActions={sidebarHeaderActions} viewport={viewport} width={width} maxWidth={maxSidebar} collapsed={collapsed} drawerOpen={false} onDrawerOpenChange={() => undefined} onToggleCollapsed={() => setCollapsed(value => !value)} onWidthChange={setDragWidth} onWidthCommit={value => { setRatio(value / viewport); setDragWidth(undefined) }} onWidthReset={() => setRatio(256 / viewport)}>{sidebarBody}</ResponsiveAgentSidebar>}
              <DockedSplit persistKey={persistGeometry ? 'live.rightPanel' : undefined} label={dockLabel} defaultSize={440} minMainSize={geometryNumbers.panelMin} onCollapse={() => setPanel({ open: false })} dock={dockedInspector !== undefined ? (
                <React.Suspense fallback={<aside aria-label="Agent inspector" {...stylex.props(styles.changesPanel, styles.sheetPanel)}><SystemNotice kind="loading" title="Loading agent details" /></aside>}>
                  <AgentDetailsPanel key={dockedInspector} agentRef={dockedInspector} width="dock" scheme={scheme} view={panel.tab} onViewChange={view => setPanel({ tab: view })} onClose={() => setPanel({ open: false })} onOpen={openDetailsUri} {...(ux === undefined ? {} : { ux })} />
                </React.Suspense>
              ) : undefined}>
              <section aria-label={workspaceLabel} onPointerOverCapture={prefetchTerminalIntent} onFocusCapture={prefetchTerminalIntent} {...stylex.props(styles.workspace)}>
                {narrow ? <PhoneAgentDrawer ref={agentsDrawer} triggerRef={sidebarOpener} headerActions={sidebarHeaderActions} header={(showAgents, sheet) => <ClockedThreadHeader agent={agent} stale={stale} narrow sidebarCollapsed onToggleSidebar={showAgents} sidebarToggleRef={sidebarOpener} sidebarSheet={sheet} nativeActions={agentHeaderActions} actionPortalRef={setHeaderSlot} folder={agent?.host} title={agent?.name ?? activeLabel ?? 'Select an agent'} freshness={stale ? 'stale' : agent === undefined ? 'unobserved' : 'live'} panelOpen={narrowDetailsOpen} terminalOpen={terminalOpen} onTogglePanel={() => { if (narrowDetailsOpen) closeDetails(); else setSheetOpen(true) }} onToggleTerminal={toggleTerminal} breadcrumbLabel={workspaceLabel === 'Agent workspace' ? 'Breadcrumb' : `${workspaceLabel} breadcrumb`} />}>{sidebarBody}</PhoneAgentDrawer> : <ClockedThreadHeader agent={agent} stale={stale} sidebarCollapsed={collapsed} onToggleSidebar={toggleSidebar} sidebarToggleRef={sidebarOpener} terminalAvailable={terminalRef !== undefined || terminalOpen} {...(shownTerminalDisabledReason === undefined ? {} : { terminalDisabledReason: shownTerminalDisabledReason })} nativeActions={agentHeaderActions} actionPortalRef={setHeaderSlot} folder={agent?.host} title={agent?.name ?? activeLabel ?? 'Select an agent'} freshness={stale ? 'stale' : agent === undefined ? 'unobserved' : 'live'} panelOpen={panel.open} terminalOpen={terminalOpen} onTogglePanel={() => setPanel({ open: !panel.open })} onToggleTerminal={toggleTerminal} breadcrumbLabel={workspaceLabel === 'Agent workspace' ? 'Breadcrumb' : `${workspaceLabel} breadcrumb`} />}
                <GatewayConnectionStatus />
                <AlertFlashBand onOpen={openAlert} />
                {/* One strip and one portal host at every width: the breakpoint never remounts a surface. */}
                <WorkspaceStrip items={tabItems} active={activeRow?.id ?? null} group={workspace.focus} narrow={narrow} dispatch={go} open={openRef} trailing={narrow && (terminalRef !== undefined || terminalDisabledReason !== undefined || terminalOpen) ? <PhoneTerminalToggle pressed={terminalOpen} onPress={toggleTerminal} {...(shownTerminalDisabledReason === undefined ? {} : { disabledReason: shownTerminalDisabledReason })} /> : undefined}>
                  <div ref={setPaneHost} {...stylex.props(styles.body)} />
                </WorkspaceStrip>
              </section>
              </DockedSplit>
              {!narrow && openedTool !== undefined && current !== '' ? (
                <>
                  <ResizableSplit id="live-tool-panel" reverse value={panelSize} min={240} max={maxPanel} collapsed={false} label="Tool detail panel width" onChange={setDragPanel} onCommit={(value) => { setPanelFraction(value / viewport); setDragPanel(undefined) }} onToggle={() => setOpenedTool(undefined)} onReset={() => setPanelFraction(380 / viewport)} />
                  {toolDetail(openedTool, panelSize)}
                </>
              ) : null}
              {narrow ? <NarrowSheet label="Agent details" isOpen={narrowDetailsOpen} onOpenChange={(isOpen) => { if (!isOpen) closeDetails() }} width={400}>
                {current === '' ? null : openedTool !== undefined ? toolDetail(openedTool, undefined) : (
                  <React.Suspense fallback={<SystemNotice kind="loading" title="Loading agent details" />}>
                    <AgentDetailsPanel key={current} agentRef={current} width="fill" scheme={scheme} onClose={closeDetails} onOpen={(uri) => { closeDetails(); openDetailsUri(uri) }} />
                  </React.Suspense>
                )}
              </NarrowSheet> : null}
              <div ref={overlayHost} {...stylex.props(styles.overlayHost)} />
            </div>
        </MonitorDetailProvider>
      </ResourcePanelProvider>
    </WorkbenchContextProvider>
    {paneHost === null ? null : <RegisteredPortal container={paneHost}><WorkspaceBody tabs={allTabs} active={activeRow?.id ?? null} tiers={tiers} views={workspace.views} names={names} onOpenTool={onOpenTool} onOpen={openRef} onReplace={onReplaceTab} onClose={onCloseTab} onBackToAlerts={backToAlerts} onView={onTabView} focusShownFace={focusShownFace} {...(ux === undefined ? {} : { ux })} empty={<WorkspaceEmpty open={openRef} />} /></RegisteredPortal>}
    </ThreadHeaderSlotContext.Provider>
  </NarrowLayoutContext.Provider></OpenProvider>)
}

/** Only time-bearing surfaces subscribe to the wall clock, not the workspace providers. */
function ClockedThreadHeader({ agent, stale, ...props }: Omit<React.ComponentProps<typeof ThreadHeader>, 'now' | 'status' | 'statusLabel' | 'statusSince'> & { readonly agent?: Agent; readonly stale: boolean }) {
  const row = agent === undefined ? undefined : sidebarRow({ agent, stale })
  // Beside the title only what asks for attention: amber when it needs the person, red when it failed. Calm states show nothing.
  const marked = row?.status === 'waiting' || row?.status === 'failed' ? row : undefined
  return <ThreadHeader {...props} status={marked?.status} statusLabel={marked?.statusLabel} statusSince={marked?.statusSince} />
}



const assumedOnline = Atom.make({ _tag: 'Online' } as const)
/** Decoded gateway sync, never the socket's raw lifecycle tag, as the one connection band under the thread header. */
const GatewayConnectionStatus = () => {
  const source = useDataSource()
  const network = useAtomValue(source.network ?? assumedOnline)
  const observation = useGatewaySync()
  const now = useNow()
  if (observation === undefined) return null
  const line = syncLine({
    ...observation, label: 'connection', now,
    socket: observation.status._tag !== 'Failed',
  })
  const status = observation.status
  const reconnecting = status._tag === 'Stale' && status.reason._tag === 'Reconnecting'
  // The band's title already says "Reconnecting"; its detail is the next try, after syncLine's grace delay.
  const text = line === undefined ? undefined
    : status._tag === 'Stale' && status.reason._tag === 'Reconnecting'
      ? `Next try in ${formatDuration(Math.ceil(Math.max(0, status.reason.nextAt - now) / 1000) * 1000)} · attempt ${status.reason.attempt}`
      : line.text
  return <ConnectionStatus
    state={network._tag === 'Offline' ? 'offline' : reconnecting ? 'reconnecting' : status._tag === 'Stale' ? 'stale' : status._tag === 'Failed' ? 'failed' : 'live'}
    {...(text === undefined ? {} : { text })}
    {...((reconnecting || network._tag === 'Offline') && source.reconnect !== undefined ? { onReconnect: source.reconnect } : {})}
  />
}

/** The shared toolbar with the live sidebar's search surface, which must stand apart from the sidebar in both schemes. */
export function LiveSidebarToolbar(props: Omit<SidebarToolbarProps, 'searchStyle'>) {
  return <SidebarToolbar {...props} searchStyle={styles.searchField} />
}

const styles = stylex.create({
  legacyBridge: { '--canvas': c.canvas, '--panel': c.raised, '--recess': c.message, '--ink': c.fg, '--muted': c.fgMuted, '--line': c.borderStrong, '--accent': c.primary, '--on-accent': c.onPrimary, '--selection': c.rowActive, '--good': c.done, '--warning': c.attention, '--danger': c.dangerFg, '--sans': t.fontSans, '--mono': t.fontMono },
  app: { display: 'flex', height: 'calc(100dvh - var(--wf-devbar-space, 0px))', width: '100%', overflow: 'hidden', backgroundColor: c.canvas, color: c.fg, fontFamily: t.fontSans, fontSize: t.uiSize, lineHeight: t.uiLeading, paddingBlockStart: { default: 0, '@media (max-width: 899px)': 'env(safe-area-inset-top)' }, paddingBlockEnd: { default: 0, '@media (max-width: 899px)': 'env(safe-area-inset-bottom)' }, boxSizing: 'border-box' },
  // Out of flow, above the panes. Viewport-wide but zero-height: no hit area of
  // its own, yet shrink-to-fit popovers get the viewport as available width.
  overlayHost: { position: 'fixed', top: 0, left: 0, width: '100%', height: 0, zIndex: 20 },
  iconButton: { width: 28, height: 28, display: 'inline-flex', justifyContent: 'center', alignItems: 'center', backgroundColor: 'transparent', color: c.fgMuted, borderWidth: 0, borderRadius: 6, cursor: 'pointer' },
  // Message fill plus a hairline: raised equals the sidebar surface in light mode.
  searchField: { backgroundColor: c.message, borderWidth: g.hairline, borderStyle: 'solid', borderColor: c.border },
  // AgentFolders' Tree owns scrolling; the roster only assigns the remaining flex space.
  roster: { display: 'flex', flexDirection: 'column', overflow: 'hidden', padding: s.md, minHeight: 0, flex: '1 1 0', gap: 2 },
  // Match SR-2's shared row/control/icon geometry and the roster's 2px inter-row gap.
  rosterSkeleton: { display: 'flex', flexDirection: 'column', gap: 2 },
  skeletonRow: { height: g.sidebarRow, flexShrink: 0, display: 'grid', gridTemplateColumns: `${g.icon} minmax(0, 1fr)`, gridTemplateRows: `${g.controlSm} ${g.controlSm}`, alignContent: 'center', alignItems: 'center', columnGap: s.xs2, paddingInline: s.sm, boxSizing: 'border-box' },
  skeletonGlyph: { gridColumn: '1', gridRow: '1', width: g.icon, height: g.icon, borderRadius: '50%', backgroundColor: c.border, opacity: 0.5 },
  skeletonLine: { gridColumn: '2', height: 8, borderRadius: 4, backgroundColor: c.border, opacity: 0.5 },
  skeletonName: { gridRow: '1', width: '68%' },
  skeletonHost: { gridRow: '2', width: '44%' },
  workspace: { display: 'flex', flexDirection: 'column', flexGrow: 1, minWidth: 0, minHeight: 0 },
  body: { position: 'relative', display: 'flex', flexDirection: 'column', flexGrow: 1, minWidth: 0, minHeight: 0, overflow: 'hidden' },
  changesPanel: { display: 'flex', flexDirection: 'column', flexShrink: 0, minWidth: 0, minHeight: 0, overflowY: 'auto', borderLeftWidth: 1, borderLeftStyle: 'solid', borderLeftColor: c.border },
  sheetPanel: { flexGrow: 1, borderLeftWidth: 0 },
  toolDetailHeader: { height: 40, display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: s.md, paddingInline: s.lg, flexShrink: 0, borderBottomWidth: 1, borderBottomStyle: 'solid', borderBottomColor: c.border },
  toolDetailTitle: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontSize: t.metaSize },
  toolDetailOutput: { margin: 0, padding: s.lg, minHeight: 0, overflowY: 'auto', fontFamily: t.fontMono, fontSize: t.denseSize, lineHeight: 1.5, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere', color: c.fgMuted },
})
