import { selectedFaviconState, tabTitle, useTabMetadata } from './useTabMetadata.ts'
import { cancelMeasure, markStart, measureEnd } from './perfMarks.ts'
import { prefetchChangelog } from '../changelog/load.ts'
import { agentSeatChildren, sidebarMissionChild, sidebarRow, sidebarSubagentRow, subagentRowId, sidebarMissionFeedsAtom } from './sidebarRow.ts'
import { subagentThreadFocus } from './subagentThreadFocus.ts'
import { ConversationPaneFallback } from './ConversationPaneFallback.tsx'
import type * as ConversationPaneModule from './ConversationPane.tsx'
import type * as TerminalDetailModule from '../terminal/TerminalDetail.tsx'
import { ConversationHeaderActions } from './ConversationHeaderActions.tsx'
import { DeclarationDialog } from './DeclarationDialog.tsx'
import { AgentActions } from './AgentActions.tsx'
import type * as AlertsTabModule from './AlertsTab.tsx'
import type * as SidebarMissionsListModule from '../missions/SidebarMissionsList.tsx'
import { alertTabMarkers } from './alertsTab.ts'
import { missionTabMarkersAtom } from './missionsTab.ts'
import { isMissionRef, missionPaneId } from '../missions/missionTitle.ts'
import { isTerminalRef, retainPane, workspaceHref, workspaceView, type RetainedPane, type TerminalRef, type WorkspaceView } from './workspaceView.ts'
import { MISSIONS_REF, alertIdOfPane, alertPaneId, isAlertRef, paneTabLabel } from './workspaceView.ts'
import { AlertFlashBand } from './AlertFlashBand.tsx'
import { useVisibleAttention } from '../conversation/alerts/useAlertAnswers.ts'
import { ThreadHeaderSlotContext } from '../shell/threadHeaderSlot.tsx'
import { liveLegacyTheme } from '../ui-compat/live-theme.stylex.ts'
import { colorVars as c, typeVars as t, spaceVars as s, geometryVars as g, geometryNumbers, sidebarRowSize } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import * as React from 'react'
import { RegisteredPortal, usePortalContainerRegistration } from '../../../../packages/fractal-ui/src/assistant-ui/taste/PortalContainer.tsx'
import * as Aria from 'react-aria-components'
import { UNSAFE_PortalProvider } from 'react-aria'
import { flushSync } from 'react-dom'
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
import { ViewportStore, ViewportStoreContext } from '../../../../packages/fractal-ui/src/assistant-ui/EmbraceScrollViewport.tsx'
import { terminalSubjectForAgent } from '../data/projections.ts'
import { sidebarPins } from '../data/sidebarPins.ts'
import { useFleet, useAgent, useSubjectList, useConnection, useNow, useNowSnapshot, useFeedInterest, useDataSource, useRosterDecode, useAskingAgents } from '../data/react.tsx'
import { persistedAtom } from '../state/persistence.ts'
import { setTheme } from '../ui-compat/foundations.ts'
import { WorkbenchContextProvider, ResourcePanelProvider, MonitorDetailProvider, type OpenRequest } from '../shell/context.tsx'
import type { ResourcePanelState } from '../shell/state.ts'
import { afterNextPaint, type UxTelemetry } from '../telemetry/ux.ts'
import { formatDuration } from '@fractal/ui/human-time/format'
import { createLazyChunk, readLazyChunkAssets } from '@fractal/ui/lazy-chunk'
import { useGatewaySync } from '../data/react.tsx'
import { NarrowLayoutContext } from '../../../../packages/fractal-ui/src/assistant-ui/composition/NarrowLayout.tsx'
import { PhoneAgentDrawer, type PhoneAgentDrawerHandle } from './PhoneAgentDrawer.tsx'
import { PhoneWorkspaceTabs } from './PhoneWorkspaceTabs.tsx'
import { AgentFolders } from '../shell/AgentFolders.tsx'
import { useSidebarFolders, type SidebarFoldersBinding } from '../folders/useSidebarFolders.ts'
import { fixtureFolders } from '../folders/fixture.ts'
import { defaultFilters, fixtureSidebarState, sidebarState } from '../shell/sidebar/state.ts'
import { useMachineMemory, withRememberedMachines } from '../shell/sidebar/show.ts'
import { parseSidebarQuery } from '../shell/sidebar/filter.ts'
import { SidebarToolbar, type SidebarToolbarProps } from '../shell/sidebar/Toolbar.tsx'
import { credentialNotice, waiting, type Agent, type Feed } from '../data/source.ts'
import type { TerminalScreen } from '@fractal/client/schema'
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
const idleTerminal = Atom.make<Feed<TerminalScreen>>(waiting)


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
        ...(agentSeatChildren(agent) ?? []).flatMap(child => {
          if (child.missionId === undefined) return []
          const mission = sidebarMissionChild({ agent, missionId: child.missionId, missions: observation?.missions, attention: observation?.attention })
          return [[child.id, missionChildAccessibleName({ ...mission, title: mission.title ?? child.label })] as const]
        }),
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

// A static import would evaluate the kit Transcript/Markdown before the shell paints.
// Demand and speculative prefetch share one request; a rejection stays until a user retry.
const conversationChunk = createLazyChunk({ load: () => import('./ConversationPane.tsx'), assets: () => readLazyChunkAssets('conversation') })
const ConversationPane = React.lazy(() => {
  // An initially selected pane and later shell prefetch must retain the same lazy identity.
  // A second wrapper suspends on an already resolved module during the first seat switch.
  prefetchedConversationPane ??= ConversationPane
  return conversationChunk.load().then(module => ({ default: module.ConversationPane }))
})
let prefetchedConversationPane: typeof ConversationPane | undefined
const prefetchConversationPane = () => {
  const request = conversationChunk.load()
  // Preserve this attempt's failure for the requested pane's honest, local error surface.
  // A retry creates a fresh lazy wrapper, not React's permanently rejected cached wrapper.
  prefetchedConversationPane ??= React.lazy(() => request.then(module => ({ default: module.ConversationPane })))
  void request.catch(() => {})
}

/** The first recovery re-imports in this document; after a second rejected import, reload with fresh feature assets. Drafts and selection are already persisted. */
const LazyChunkRecoveryAction = (props: { readonly chunk: { readonly recovery: 'retry' | 'reload'; readonly reload: () => Promise<void> }; readonly onRetry: () => void }) =>
  props.chunk.recovery === 'reload'
    ? <button type="button" onClick={() => void props.chunk.reload()}>Reload and retry</button>
    : <button type="button" onClick={props.onRetry}>Try again</button>

/** Acquire visible route demand outside Suspense so the socket read overlaps pane evaluation. */
const ConversationPaneLoadBoundary = (props: React.ComponentProps<typeof ConversationPaneModule.ConversationPane>) => {
  const source = useDataSource()
  const interest = React.useMemo(() => source.conversationInterest?.(props.agentRef), [source, props.agentRef])
  useFeedInterest({ interest, visible: props.visible ?? true })
  return <ConversationPaneCodeBoundary {...props} interest={interest} />
}

class ConversationPaneCodeBoundary extends React.Component<React.ComponentProps<typeof ConversationPaneModule.ConversationPane>> {
  state: { readonly failed: boolean; readonly Pane: typeof ConversationPane } = {
    failed: false,
    Pane: prefetchedConversationPane ?? ConversationPane,
  }

  static getDerivedStateFromError() {
    return { failed: true }
  }

  render() {
    if (this.state.failed) return <div role="alert" aria-hidden={!this.props.visible} {...stylex.props(styles.empty)}>
      <p>This conversation view could not load.</p>
      <LazyChunkRecoveryAction chunk={conversationChunk} onRetry={() => {
        const Pane = React.lazy(() => conversationChunk.retry().then(module => ({ default: module.ConversationPane })))
        prefetchedConversationPane = Pane
        this.setState({ failed: false, Pane })
      }} />
    </div>
    const Pane = this.state.Pane
    return <React.Suspense fallback={<ConversationPaneFallback agentName={this.props.agentName} visible={this.props.visible} />}>
      <Pane {...this.props} />
    </React.Suspense>
  }
}

// Load the tab on demand, not with the shell.
const makeAlertsTab = () => React.lazy(() => import('./AlertsTab.tsx').then(module => ({ default: module.AlertsTab })))
const AlertsTab = makeAlertsTab()
class AlertsTabCodeBoundary extends React.Component<React.ComponentProps<typeof AlertsTabModule.AlertsTab>> {
  state = { failed: false, Tab: AlertsTab }
  static getDerivedStateFromError() { return { failed: true } }
  render() {
    if (this.state.failed) return <div role="alert" {...stylex.props(styles.empty)}>
      <p>Alerts could not load.</p>
      <button type="button" onClick={event => {
        // The retry control unmounts during Suspense; keep keyboard focus on the stable selected tab.
        event.currentTarget.closest('[data-sidebar-tabs]')?.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]')?.focus()
        this.setState({ failed: false, Tab: makeAlertsTab() })
      }}>Try again</button>
    </div>
    const Tab = this.state.Tab
    return <React.Suspense fallback={<SystemNotice kind="loading" title="Loading alerts" />}>
      <Tab {...this.props} />
    </React.Suspense>
  }
}

// Details, its measured resource tabs and Changes import the rich kit; keep them outside the eager shell.
const AgentDetailsPanel = React.lazy(() => import('./AgentDetailsPanel.tsx').then(module => ({ default: module.AgentDetailsPanel })))
const ChangesInspector = React.lazy(() => import('./ChangesInspector.tsx').then(module => ({ default: module.ChangesInspector })))
/** Set once the module resolves, so a warm reopen renders it directly instead of suspending a fresh lazy identity. */
let terminalDetailLoaded: typeof TerminalDetailModule | undefined
/** Demand and idle intent share one request; a rejection stays until a user retry. */
const terminalChunk = createLazyChunk({
  load: () => import('../terminal/TerminalDetail.tsx').then(module => {
    terminalDetailLoaded = module
    return module
  }),
  assets: () => readLazyChunkAssets('terminal'),
})
const terminalDetail = (load: () => Promise<typeof TerminalDetailModule>) => React.lazy(() =>
  load().then(module => {
    void module.prepareTerminal()
    return { default: module.TerminalDetail }
  }),
)
const prefetchTerminalDetail = () => {
  void terminalChunk.load().then(module => module.prepareTerminal()).catch(() => {})
}

/** The terminal grid, fonts and native key encoder stay outside the eager shell; a fresh lazy identity on retry avoids React's permanently rejected module cache. */
class TerminalCodeBoundary extends React.Component<{ readonly terminalRef: `terminal/${string}`; readonly shown: boolean }> {
  state = { failed: false, Detail: terminalDetail(() => terminalChunk.load()) }
  static getDerivedStateFromError() { return { failed: true } }
  render() {
    if (this.state.failed) return <div role="alert">
      <p>This terminal view could not load.</p>
      <LazyChunkRecoveryAction chunk={terminalChunk} onRetry={() => this.setState({ failed: false, Detail: terminalDetail(() => terminalChunk.retry()) })} />
    </div>
    const Detail = terminalDetailLoaded?.TerminalDetail ?? this.state.Detail
    return <React.Suspense fallback={<SystemNotice kind="loading" title="Loading terminal" />}>
      <Detail address={{ ref: this.props.terminalRef, presentation: 'detail' }} visibility={this.props.shown ? 'visible' : 'hidden'} keepConnected />
    </React.Suspense>
  }
}
const AlertDetailsPane = React.lazy(() => import('./AlertDetailsPane.tsx').then((module) => ({ default: module.AlertDetailsPane })))
const SidebarMissionsList = (props: React.ComponentProps<typeof SidebarMissionsListModule.SidebarMissionsList>) => <React.Suspense fallback={<SystemNotice kind="loading" title="Loading missions" />}><LazySidebarMissionsList {...props} /></React.Suspense>
const LazySidebarMissionsList = React.lazy(() => import('../missions/SidebarMissionsList.tsx').then((module) => ({ default: module.SidebarMissionsList })))
const MissionsPane = React.lazy(() => import('../missions/MissionsPane.tsx').then((module) => ({ default: module.MissionsPane })))
const DocumentPane = React.lazy(() => import('../documents/DocumentPane.tsx').then((module) => ({ default: module.DocumentPane })))
const makeUsagePane = () => React.lazy(() => import('../monitor/UsagePane.tsx'))
class UsageCodeBoundary extends React.Component {
  state = { failed: false, Pane: makeUsagePane() }
  static getDerivedStateFromError() { return { failed: true } }
  render() {
    if (this.state.failed) return <div role="alert" {...stylex.props(styles.paneNotice)}>
      <p>This usage view could not load.</p>
      <button type="button" onClick={() => this.setState({ failed: false, Pane: makeUsagePane() })}>Try again</button>
    </div>
    const Pane = this.state.Pane
    return <React.Suspense fallback={<div {...stylex.props(styles.paneNotice)}><SystemNotice kind="loading" title="Loading usage" /></div>}>
      <Pane />
    </React.Suspense>
  }
}

const sidebarRatio = persistedAtom({ key: 'round2.sidebarRatio', schema: Schema.Number, defaultValue: 256 / 1440 })
const panelRatio = persistedAtom({ key: 'round2.panelRatio', schema: Schema.Number, defaultValue: 380 / 1440 })
const sidebarClosed = persistedAtom({ key: 'round2.sidebarClosed', schema: Schema.Boolean, defaultValue: false })
const workspacePanes = Atom.family((ref: string) => persistedAtom({ key: `round2.panes.${ref}`, schema: Schema.Array(Schema.Struct({ ref: Schema.String, presentation: Schema.optional(Schema.Literals(['detail', 'overview', 'resources'])) })), defaultValue: [] }))
const selectedAgent = persistedAtom({ key: 'round2.agent', schema: Schema.String, defaultValue: '' })
const phoneAgentTabs = persistedAtom({ key: 'round2.phoneAgentTabs', schema: Schema.Array(Schema.String), defaultValue: [] })
const viewportWidth = () => window.innerWidth
/** Below 900px the thread is the only thing on screen: agents open as a sheet from the left, the agent's actions and Details from the right. Same width as the sidebar drawer breakpoint. */
export const narrowLayoutQuery = '(max-width: 899px)'
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
const initialAgentFromUrl = (location: string) => {
  const encoded = location.split('?')[0]?.match(/^\/w\/(.+)$/)?.[1]
  try {
    return encoded === undefined ? undefined : decodeURIComponent(encoded)
  } catch {
    return undefined
  }
}

/** The app uses browser history; static embeds supply a private in-memory location. */
export interface WorkspaceNavigation {
  readonly getLocation: () => string
  readonly subscribe: (callback: () => void) => () => void
  readonly navigate: (href: string, options?: { readonly replace?: boolean }) => void
}
const browserNavigation: WorkspaceNavigation = {
  getLocation: locationState,
  subscribe: subscribeLocation,
  navigate: (href, options) => {
    if (options?.replace === true) window.history.replaceState(null, '', href)
    else window.history.pushState(null, '', href)
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
  return <UNSAFE_PortalProvider getContainer={getContainer}><Workspace {...props} overlayHost={overlayHost} /></UNSAFE_PortalProvider>
}

/** One authoritative agent thread per workspace; local geometry never grants backend authority. */
function Workspace({ ux, onSelectConversation, dimensions, navigation = browserNavigation, persistGeometry = true, workspaceLabel = 'Agent workspace', build, overlayHost }: WorkspaceProps & { readonly overlayHost: React.RefObject<HTMLDivElement | null> }) {
  const fleet = useFleet(), subjects = useSubjectList(), connection = useConnection(), rosterDecode = useRosterDecode()
  const [storedAgent, setStoredAgent] = useAtom(selectedAgent)
  const [phoneAgents, setPhoneAgents] = useAtom(phoneAgentTabs)
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
  const panelOpener = React.useRef<HTMLButtonElement>(null)
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
  const current = initialAgentFromUrl(location) ?? (storedAgent || agents[0]?.ref || '')
  const agent = agents.find((row) => row.ref === current)
  // Attaches that found no terminal, per agent. Each holds until the roster reports different runtimes for that agent.
  const [terminalFailures, setTerminalFailures] = React.useState<ReadonlyMap<string, { readonly runtimeKey: string | undefined; readonly reason: string }>>(new Map())
  const failedAttach = (row: Agent) => {
    const failure = terminalFailures.get(row.ref)
    return failure !== undefined && failure.runtimeKey === row.runtimeKey ? failure.reason : undefined
  }
  // A canonical subject address is not proof of a terminal. Keep the control disabled
  // from its first render until the roster can establish the selected agent's runtime.
  const terminalDisabledReason = terminalSubjectForAgent(current) === undefined ? undefined
    : credentialNotice(fleet) ?? (fleet._tag === 'Waiting' ? 'Waiting for the live agent roster to find this agent’s terminal.'
      : fleet._tag === 'Unavailable' ? 'The agent roster is unavailable, so this agent’s terminal cannot be found.'
        : agent === undefined ? 'This agent is not in the live agent roster, so it has no terminal.'
          : agent.terminalDisabledReason ?? failedAttach(agent))
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
  const [resourcesHost, setResourcesHost] = React.useState<HTMLDivElement | null>(null)
  const [resourcesAgent, setResourcesAgent] = React.useState<string>()
  const [panes, setPanes] = useAtom(workspacePanes(current))
  const query = new URLSearchParams(location.split('?')[1])
  const selectedPane = query.get('open') ?? 'thread'
  const [resourcePanel, setResourcePanel] = React.useState<ResourcePanelState>({ expanded: false, size: 288 })
  const [diffOpen, setDiffOpen] = React.useState(false)
  const [openedTool, setOpenedTool] = React.useState<WorkLogCall | undefined>(undefined)
  // The row menu opens one declaration sheet; a stable action object keeps rows memoizable.
  const [declarationRef, setDeclarationRef] = React.useState<string | undefined>(undefined)
  const pins = useAtomValue(terminalSource.pins ?? sidebarPins())
  const pinned = React.useMemo(() => new Set(pins.state.members.map(item => item.member)), [pins.state.members])
  const pinnedItems = React.useMemo(() => {
    const names = new Map(agents.map(agent => [agent.ref, agent.name]))
    return pins.state.members.map(({ member }) => ({ id: member, label: names.get(member) ?? member }))
  }, [agents, pins.state.members])
  const sidebarActions = React.useMemo<SidebarActions>(() => ({
    declaration: setDeclarationRef,
    ...(pins.editable ? { pins: { has: (ref: string) => pinned.has(ref), toggle: pins.toggle } } : {}),
  }), [pins.editable, pins.toggle, pinned])
  const [panelFraction, setPanelFraction] = useAtom(panelRatio)
  const [dragWidth, setDragWidth] = React.useState<number | undefined>()
  const [dragPanel, setDragPanel] = React.useState<number | undefined>()
  const navigate = (ref: string, pane = 'thread', options?: { readonly replace?: boolean }) => {
    setStoredAgent(ref)
    navigation.navigate(workspaceHref(ref, pane, location.split('?')[1] ?? ''), options)
  }
  // Which agents were last left on their terminal. Memory only: a reload restores the selected agent through `?open=`.
  const terminalAgents = React.useRef(new Set<string>())
  React.useLayoutEffect(() => {
    if (selectedPane.startsWith('terminal/')) terminalAgents.current.add(current)
    else if (selectedPane === 'thread') terminalAgents.current.delete(current)
  }, [current, selectedPane])
  const rememberedPane = (ref: string) => {
    const terminal = terminalAgents.current.has(ref) ? agents.find(row => row.ref === ref && row.terminalDisabledReason === undefined)?.terminal : undefined
    return terminal === undefined ? 'thread' : `${terminal}:detail`
  }
  const select = (ref: string, pane = rememberedPane(ref)) => {
    if (narrow) setPhoneAgents(refs => [...new Set([...refs, current, ref])].filter(Boolean))
    if (ref !== current) {
      cancelMeasure('open_transcript')
      markStart('switch_to_tail', { key: ref })
      onSelectConversation?.(ref)
    }
    setDiffOpen(false)
    setResourcesAgent(undefined)
    setOpenedTool(undefined)
    agentsDrawer.current?.close()
    navigate(ref, pane)
  }
  const open = (request: OpenRequest) => {
    if (request.ref.startsWith('agent/') && request.presentation === undefined) {
      select(request.ref)
      return
    }
    if (isMissionRef(request.ref)) {
      focusMission(request.ref)
      return
    }
    const id = `${request.ref}:${request.presentation ?? 'detail'}`
    // The terminal replaces the conversation through the header control, so it never becomes a tab.
    if (isTerminalRef(request.ref)) {
      navigate(current, id)
      return
    }
    setPanes((rows) => (rows.some((row) => `${row.ref}:${row.presentation ?? 'detail'}` === id) ? rows : [...rows, { ref: request.ref, ...(request.presentation === undefined ? {} : { presentation: request.presentation }) }]))
    navigate(current, id)
  }
  // A mission is addressed by the URL alone, like an alert; collapsing it leaves the Missions screen open.
  const focusMission = (ref: string | undefined) => {
    agentsDrawer.current?.close()
    navigate(current, ref === undefined ? `${MISSIONS_REF}:detail` : missionPaneId(ref))
  }
  const openAlert = (id: string) => {
    agentsDrawer.current?.close()
    navigate(current, alertPaneId(id))
  }
  // Stable for the memoized panes: a document named in a conversation or another document opens in a tab.
  const openLatest = React.useRef(open)
  openLatest.current = open
  const openDocument = React.useCallback((ref: string) => openLatest.current({ ref, presentation: 'detail' }), [])
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
  const selectLatest = React.useRef(select)
  selectLatest.current = select
  const selectAgent = React.useCallback((ref: string) => {
    if (previousRoster.current?.some(seat => seat.children?.some(child => child.id === ref))) return
    selectLatest.current(ref)
  }, [])
  const byRef = React.useMemo(() => new Map(subjects.map((subject) => [subject.ref, subject])), [subjects])
  const separator = selectedPane.lastIndexOf(':')
  const urlPresentation = selectedPane.slice(separator + 1)
  const urlPane: OpenRequest | undefined =
    separator > 0 && (urlPresentation === 'detail' || urlPresentation === 'overview' || urlPresentation === 'resources')
      ? { ref: selectedPane.slice(0, separator), presentation: urlPresentation }
      : undefined
  const chosen = panes.find((row) => `${row.ref}:${row.presentation ?? 'detail'}` === selectedPane) ?? urlPane
  // An alert's details are a passing view: reached by URL, never a tab.
  const tabPanes = panes.filter(row => !isTerminalRef(row.ref))
  const visiblePanes = chosen && !isAlertRef(chosen.ref) && !isMissionRef(chosen.ref) && !isTerminalRef(chosen.ref) && !panes.some((row) => `${row.ref}:${row.presentation ?? 'detail'}` === selectedPane) ? [...tabPanes, chosen] : tabPanes
  const phoneTabs = React.useMemo(() => {
    const names = new Map(agents.map(agent => [agent.ref, agent.name]))
    return [
      ...[...new Set([...phoneAgents, current])]
        .filter(ref => ref === current || names.has(ref))
        .map(ref => ({ id: ref, label: ref === current ? 'Thread' : names.get(ref)! })),
      { id: 'changes', label: 'Changes' },
      ...visiblePanes.map(pane => ({ id: `${pane.ref}:${pane.presentation ?? 'detail'}`, label: paneTabLabel(pane.ref, byRef.get(pane.ref)?.title) })),
    ]
  }, [agents, phoneAgents, current, visiblePanes, byRef])
  const address = { ref: chosen?.ref ?? current, presentation: chosen?.presentation ?? 'detail' }
  const maxSidebar = Math.max(sidebarMinimumWidth, Math.min(440, viewport - 640))
  const width = Math.min(maxSidebar, Math.max(sidebarMinimumWidth, dragWidth ?? Math.round(ratio * viewport)))
  const maxPanel = Math.max(240, viewport - (narrow || collapsed ? 0 : width) - 640)
  const panelSize = dragPanel ?? Math.min(maxPanel, Math.max(240, Math.round(panelFraction * viewport)))
  const agentName = agent?.name ?? current.split('/').at(-1) ?? 'Conversation'
  const changesWorkspace = agent?.workspace._tag === 'Known' ? JSON.stringify([agent.host, agent.workspace.value]) : current
  const view = workspaceView(chosen)
  const terminalOpen = view._tag === 'Terminal'
  // The control stays enabled while the terminal is shown: it is the way back, even from a deep link to a terminal this agent lacks.
  const shownTerminalDisabledReason = terminalOpen ? undefined : terminalDisabledReason
  const focusShownView = React.useRef(false)
  // An attach that finds no terminal records its reason and replaces the failed view so Back can leave it.
  const shownTerminalRef = terminalOpen ? terminalSubjectForAgent(current) : undefined
  const shownTerminal = useAtomValue(shownTerminalRef === undefined ? idleTerminal : terminalSource.terminal(shownTerminalRef))
  const missingTerminal = shownTerminalRef !== undefined && shownTerminal._tag === 'Unavailable' && shownTerminal.code === 'no-terminal'
    ? shownTerminal.detail.trim() || 'This agent has no terminal.' : undefined
  React.useEffect(() => {
    if (missingTerminal === undefined) return
    setTerminalFailures(failures => new Map(failures).set(current, { runtimeKey: agent?.runtimeKey, reason: missingTerminal }))
    focusShownView.current = true
    navigate(current, 'thread', { replace: true })
  }, [missingTerminal, current, agent?.runtimeKey, navigate])
  const toggleTerminal = () => {
    if (terminalOpen) { focusShownView.current = true; navigate(current) }
    else if (terminalRef !== undefined) { focusShownView.current = true; navigate(current, `${terminalRef}:detail`) }
  }
  const resourcesDockOpen = !narrow && current !== '' && resourcesAgent === current
  // The dock separator names the inspector view on screen. Every panel mount starts on Details, so the stored
  // view resets whenever the mounted inspector changes or goes away, before this render paints the label.
  const dockedInspector = !resourcesDockOpen && !narrow && diffOpen && openedTool === undefined && current !== '' ? current : undefined
  const [inspectorView, setInspectorView] = React.useState<{ readonly mount: string | undefined; readonly view: 'changes' | 'details' }>({ mount: undefined, view: 'details' })
  if (inspectorView.mount !== dockedInspector) setInspectorView({ mount: dockedInspector, view: 'details' })
  const dockLabel = resourcesDockOpen ? 'Resize resources' : inspectorView.view === 'changes' ? 'Resize Changes' : 'Resize Details'
  const agentHeaderActions = current === '' ? undefined : <>{agent === undefined ? null : <AgentActions key={`actions-${current}`} agent={agent} />}<ConversationHeaderActions key={current} agentRef={current} ux={ux} {...(narrow ? {} : { resourcesDock: { host: resourcesHost, open: resourcesDockOpen, onOpenChange: (open: boolean) => { setResourcesAgent(open ? current : undefined); if (open) { setDiffOpen(false); setOpenedTool(undefined) } } } })} /></>
  // Below the breakpoint Details is a sheet over the thread, never an inline panel beside it.
  const narrowDetailsOpen = narrow && current !== '' && (diffOpen || openedTool !== undefined)
  const closeDetails = () => { setDiffOpen(false); setOpenedTool(undefined) }
  // An explicit dock close unmounts the control that has focus. Commit the close, then hand focus to the header
  // toggle that reopens the dock. Passive closes (agent switch, crossing the narrow breakpoint) leave focus alone.
  const closeDockedInspector = () => {
    flushSync(() => setDiffOpen(false))
    panelOpener.current?.focus()
  }
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
    {sidebarTab === 'missions' ? <SidebarMissionsList {...(isMissionRef(address.ref) ? { current: address.ref } : {})} onOpen={focusMission} /> : sidebarTab === 'alerts' ? <AlertsTabCodeBoundary {...(alertIdOfPane(selectedPane) === undefined ? {} : { selected: alertIdOfPane(selectedPane)! })} onSelect={openAlert} onOpen={ref => { if (ref.startsWith('agent/')) select(ref, 'thread'); else { agentsDrawer.current?.close(); open({ ref }) } }} /> : <>
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
        <LiveSidebarTreeView binding={sidebarFolders} tree={visibleSidebarTree} agents={agents} stale={stale} needsYou={needsYou} current={current} query={sidebarQuery.text} revealKey={deferredFilters.query} select={selectAgent} openMission={openDocument} {...(isMissionRef(address.ref) ? { currentMission: address.ref } : {})} actions={sidebarActions} />
      </RosterFrozenAt.Provider>
    </nav>
    {fleet._tag === 'Observed' ? <DecodeNotice noun="agent" decode={rosterDecode} shown={shownRosterRefs(fleet)} /> : null}
    </>}
    <LiveUsageSection onOpenUsage={() => { agentsDrawer.current?.close(); open({ ref: 'monitor/quota', presentation: 'detail' }) }} />
  </SidebarTabs>
  // The panes stay outside changing shell providers, whose context propagation would scan every retained turn.
  // Only the stable header DOM host and narrow geometry cross that boundary.
  return (<NarrowLayoutContext.Provider value={narrow}>
    <ThreadHeaderSlotContext.Provider value={narrow ? null : headerSlot}>
    <WorkbenchContextProvider value={{ open, focusedRef: address.ref || null, subjects: byRef, platform: navigator.platform.includes('Mac') ? 'mac' : 'other' }}>
      <ResourcePanelProvider value={{ state: resourcePanel, onChange: (change) => setResourcePanel((value) => ({ ...value, ...change })) }}>
        <MonitorDetailProvider value={{ size: 380, onSizeChange: (value) => setPanelFraction(value / viewport) }}>
            <div ref={shellCommit} data-testid="live-agent-workspace" data-scheme={scheme} style={phoneViewportStyle} {...stylex.props(styles.app, styles.legacyBridge, liveLegacyTheme, scheme === 'dark' && assistantDarkTheme, scheme === 'dark' && liveComposerDarkTheme, liveAccentTheme, scheme === 'light' && compositionLightTheme)}>
              {shortcutHelp}
              {narrow ? null : <ResponsiveAgentSidebar headerActions={sidebarHeaderActions} viewport={viewport} width={width} maxWidth={maxSidebar} collapsed={collapsed} drawerOpen={false} onDrawerOpenChange={() => undefined} onToggleCollapsed={() => setCollapsed(value => !value)} onWidthChange={setDragWidth} onWidthCommit={value => { setRatio(value / viewport); setDragWidth(undefined) }} onWidthReset={() => setRatio(256 / viewport)}>{sidebarBody}</ResponsiveAgentSidebar>}
              <DockedSplit persistKey={persistGeometry ? `live.${changesWorkspace}.${resourcesDockOpen ? 'resources' : 'changes'}` : undefined} label={dockLabel} defaultSize={resourcesDockOpen ? 380 : 440} minMainSize={geometryNumbers.panelMin} onCollapse={() => { setResourcesAgent(undefined); if (dockedInspector === undefined) setDiffOpen(false); else closeDockedInspector() }} dock={resourcesDockOpen ? <div ref={setResourcesHost} {...stylex.props(styles.resourcesDock)} /> : dockedInspector !== undefined ? (
                <React.Suspense fallback={<aside aria-label="Agent inspector" {...stylex.props(styles.changesPanel, styles.sheetPanel)}><SystemNotice kind="loading" title="Loading agent details" /></aside>}>
                  <AgentDetailsPanel key={dockedInspector} agentRef={dockedInspector} width="dock" scheme={scheme} onClose={closeDockedInspector} onOpen={openDetailsUri} onSelectedViewChange={view => setInspectorView({ mount: dockedInspector, view })} />
                </React.Suspense>
              ) : undefined}>
              <section aria-label={workspaceLabel} onPointerOverCapture={prefetchTerminalIntent} onFocusCapture={prefetchTerminalIntent} {...stylex.props(styles.workspace)}>
                {narrow ? <PhoneAgentDrawer ref={agentsDrawer} triggerRef={sidebarOpener} headerActions={sidebarHeaderActions} header={(showAgents, sheet) => <ClockedThreadHeader agent={agent} stale={stale} narrow sidebarCollapsed onToggleSidebar={showAgents} sidebarToggleRef={sidebarOpener} sidebarSheet={sheet} nativeActions={agentHeaderActions} actionPortalRef={setHeaderSlot} folder={agent?.host} title={agent?.name ?? (current || 'Select an agent')} freshness={stale ? 'stale' : agent === undefined ? 'unobserved' : 'live'} panelOpen={narrowDetailsOpen} terminalOpen={terminalOpen} onTogglePanel={() => { if (narrowDetailsOpen) closeDetails(); else setDiffOpen(true) }} onToggleTerminal={toggleTerminal} breadcrumbLabel={workspaceLabel === 'Agent workspace' ? 'Breadcrumb' : `${workspaceLabel} breadcrumb`} />}>{sidebarBody}</PhoneAgentDrawer> : <ClockedThreadHeader agent={agent} stale={stale} sidebarCollapsed={collapsed} onToggleSidebar={toggleSidebar} sidebarToggleRef={sidebarOpener} panelToggleRef={panelOpener} terminalAvailable={terminalRef !== undefined || terminalOpen} {...(shownTerminalDisabledReason === undefined ? {} : { terminalDisabledReason: shownTerminalDisabledReason })} nativeActions={agentHeaderActions} actionPortalRef={setHeaderSlot} folder={agent?.host} title={agent?.name ?? (current || 'Select an agent')} freshness={stale ? 'stale' : agent === undefined ? 'unobserved' : 'live'} panelOpen={diffOpen} terminalOpen={terminalOpen} onTogglePanel={() => { setResourcesAgent(undefined); setDiffOpen(value => !value) }} onToggleTerminal={toggleTerminal} breadcrumbLabel={workspaceLabel === 'Agent workspace' ? 'Breadcrumb' : `${workspaceLabel} breadcrumb`} />}
                <GatewayConnectionStatus />
                <AlertFlashBand onOpen={openAlert} />
                {!narrow && !terminalOpen && visiblePanes.length > 0 ? (
                  <div role="toolbar" aria-label="Workspace views" {...stylex.props(styles.tabs)}>
                    <Aria.Button aria-pressed={selectedPane === 'thread'} onPress={() => navigate(current)} {...stylex.props(styles.textButton)}>
                      Thread
                    </Aria.Button>
                    {visiblePanes.map((pane) => {
                      const id = `${pane.ref}:${pane.presentation ?? 'detail'}`
                      return (
                        <Aria.Button key={id} aria-pressed={id === selectedPane} onPress={() => navigate(current, id)} {...stylex.props(styles.textButton)}>
                          {paneTabLabel(pane.ref, byRef.get(pane.ref)?.title)}
                        </Aria.Button>
                      )
                    })}
                  </div>
                ) : null}
                {/* Keep this panel and its portal container at one reconciliation position across the breakpoint. */}
                <PhoneWorkspaceTabs enabled={narrow} tabsHidden={terminalOpen} trailing={narrow && (terminalRef !== undefined || terminalDisabledReason !== undefined || terminalOpen) ? <PhoneTerminalToggle pressed={terminalOpen} onPress={toggleTerminal} {...(shownTerminalDisabledReason === undefined ? {} : { disabledReason: shownTerminalDisabledReason })} /> : undefined} tabs={phoneTabs} selected={selectedPane === 'thread' ? current : selectedPane} onSelect={id => id === current || agents.some(row => row.ref === id) ? select(id) : navigate(current, id)}>
                  <div ref={setPaneHost} {...stylex.props(styles.body, narrow && selectedPane === 'changes' && styles.phoneHidden)} />
                  {narrow && selectedPane === 'changes' && current !== '' ? <React.Suspense fallback={<SystemNotice kind="loading" title="Loading captured changes" />}><ChangesInspector key={current} agentRef={current} width="fill" /></React.Suspense> : null}
                </PhoneWorkspaceTabs>
              </section>
              </DockedSplit>
              {!narrow && openedTool !== undefined && current !== '' ? (
                <>
                  <ResizableSplit id="live-tool-panel" reverse value={panelSize} min={240} max={maxPanel} collapsed={false} label="Tool detail panel width" onChange={setDragPanel} onCommit={(value) => { setPanelFraction(value / viewport); setDragPanel(undefined) }} onToggle={() => setOpenedTool(undefined)} onReset={() => setPanelFraction(380 / viewport)} />
                  {toolDetail(openedTool, panelSize)}
                </>
              ) : null}
              {/* Always mounted: the theme portal host must exist before the sheet opens; contents render only while open. */}
              <DeclarationDialog subject={declarationRef ?? ''} isOpen={declarationRef !== undefined} onOpenChange={(open) => { if (!open) setDeclarationRef(undefined) }} />
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
    {paneHost === null ? null : <RegisteredPortal container={paneHost}><WorkspaceBody current={current} rosterRefs={agents.map(agent => agent.ref)} view={view} hidden={narrow && selectedPane === 'changes'} focusShownView={focusShownView} agentName={agentName} onOpenTool={call => { setResourcesAgent(undefined); setOpenedTool(call) }} onOpenDocument={openDocument} onOpenAgent={ref => select(ref, 'thread')} onFocusMission={focusMission} alertControls={{ onOpenAlert: openAlert, onOpenRef: ref => { if (ref.startsWith('agent/')) select(ref, 'thread'); else open({ ref }) }, onClose: () => navigate(current), onBack: () => showSidebarTab('alerts') }} ux={ux} /></RegisteredPortal>}
    </ThreadHeaderSlotContext.Provider>
  </NarrowLayoutContext.Provider>)
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

/** The three most recently opened panes stay mounted in a stable DOM order; hidden ones skip
 * rendering through `content-visibility: hidden`, which keeps their style/layout state so a
 * switch back only reveals them. Hidden panes release visible follow demand; the SDK may
 * evict their follows. Render-time history updates avoid a second committed selection.
 * Locking a pane costs O(its rows) in the browser, so the outgoing pane is only covered by the
 * opaque shown pane during the switch frame and locked after that frame paints. */
export function WorkspaceBody({ current, rosterRefs, view, hidden = false, focusShownView, agentName, onOpenTool, onOpenDocument, onOpenAgent, onFocusMission, alertControls, ux }: { readonly onFocusMission?: (ref: string | undefined) => void; readonly current: string; readonly rosterRefs: readonly string[]; readonly view: WorkspaceView; readonly hidden?: boolean; readonly focusShownView?: React.RefObject<boolean>; readonly agentName: string; readonly onOpenTool: (call: WorkLogCall) => void; readonly onOpenDocument?: (ref: string) => void; readonly onOpenAgent?: (ref: string) => void; readonly alertControls?: Omit<React.ComponentProps<typeof AlertDetailsPane>, 'id' | 'ux'>; readonly ux?: UxTelemetry }) {
  // Reading positions belong to this surface, not its three-pane DOM cache. A new
  // surface (including reload) starts following live; evicted panes keep memory here.
  const [viewportStore] = React.useState(() => new ViewportStore())
  React.useEffect(() => {
    viewportStore.open()
    return () => viewportStore.dispose()
  }, [viewportStore])
  const [retained, setRetained] = React.useState<readonly RetainedPane[]>([])
  const [switched, setSwitched] = React.useState<{ readonly shown: string; readonly covered: string | undefined }>({ shown: current, covered: undefined })
  // The terminal replaces the conversation, which stays retained behind it like any hidden pane.
  const visible = !hidden && current !== '' && view._tag === 'Thread'
  // The selected agent's terminal stays mounted and connected once shown, so switching back reveals it. Leaving the agent releases it.
  const [terminal, setTerminal] = React.useState<{ readonly owner: string; readonly ref: TerminalRef }>()
  if (current !== '' && view._tag === 'Terminal') {
    if (terminal?.ref !== view.ref || terminal.owner !== current) setTerminal({ owner: current, ref: view.ref })
  } else if (terminal !== undefined && terminal.owner !== current) setTerminal(undefined)
  const terminalShown = !hidden && view._tag === 'Terminal' && view.ref === terminal?.ref
  const area = React.useRef<HTMLDivElement>(null)
  const narrow = React.useContext(NarrowLayoutContext)
  // An explicit switch moves focus to the shown view. Terminal focus stays on the pane: focusing the grid would arm typing without a gesture.
  // A phone keeps focus on its toggle, so the switch does not raise the keyboard over the view it just revealed.
  React.useLayoutEffect(() => {
    if (focusShownView?.current !== true) return
    focusShownView.current = false
    if (narrow) return
    area.current?.querySelector<HTMLElement>(view._tag === 'Terminal' ? '[data-testid="terminal-pane"]' : '[data-wf-shown-pane] textarea:not(:disabled)')?.focus({ preventScroll: true })
  }, [view._tag, focusShownView, narrow])
  React.useLayoutEffect(() => {
    if (visible) return
    // Abandon at the eager view boundary, even if the transcript module or data
    // has not loaded. Effect cleanup only cancels paint (StrictMode can retry it).
    cancelMeasure('switch_to_tail', current)
    cancelMeasure('open_transcript', current)
  }, [current, visible])
  const [visited, setVisited] = React.useState<readonly string[]>([])
  const rosterKeys = new Set(rosterRefs)
  const recent = visited.filter(ref => rosterKeys.has(ref))
  const readingKeys = visible && rosterKeys.has(current)
    ? [current, ...recent.filter(ref => ref !== current)].slice(0, 32)
    : recent
  if (visited.length !== readingKeys.length || visited.some((ref, index) => ref !== readingKeys[index])) setVisited(readingKeys)
  // Viewports save in mutation cleanup. Prune after those saves, including late
  // eviction saves, rather than allowing departed keys to re-enter the memory.
  React.useLayoutEffect(() => { viewportStore.retain(new Set(readingKeys)) })
  const latest = retained.reduce<RetainedPane | undefined>((best, pane) => best === undefined || pane.used > best.used ? pane : best, undefined)
  if (visible && (latest?.ref !== current || latest.name !== agentName)) {
    setRetained(retainPane({ panes: retained, ref: current, name: agentName, limit: 3 }))
  }
  if (visible && switched.shown !== current) setSwitched({ shown: current, covered: switched.shown })
  React.useEffect(() => {
    if (switched.covered === undefined) return
    let timer: number | undefined
    // rAF runs before the switch frame paints; the timeout runs after it.
    const frame = window.requestAnimationFrame(() => { timer = window.setTimeout(() => setSwitched(value => ({ shown: value.shown, covered: undefined }))) })
    return () => { window.cancelAnimationFrame(frame); window.clearTimeout(timer) }
  }, [switched])
  return <ViewportStoreContext.Provider value={viewportStore}>
    <div ref={area} {...stylex.props(styles.threadArea)}>
      {retained.map(pane => {
        const shown = visible && pane.ref === current
        return <div key={pane.ref} data-wf-shown-pane={shown ? '' : undefined} {...stylex.props(styles.retainedPane, shown ? styles.shownPane : visible && pane.ref === switched.covered ? null : styles.hiddenPane)}>
          <ConversationPaneLoadBoundary agentRef={pane.ref} agentName={pane.name} onOpenTool={onOpenTool} onOpenDocument={onOpenDocument} ux={ux} visible={shown} />
        </div>
      })}
      {terminal === undefined ? null : (
        <section key={terminal.ref} aria-label="Terminal" data-testid="terminal-pane" data-terminal-shown={terminalShown} tabIndex={-1} inert={!terminalShown} aria-hidden={!terminalShown} {...stylex.props(styles.retainedPane, styles.terminalPane, terminalShown ? styles.shownPane : styles.hiddenPane)}>
          <TerminalCodeBoundary terminalRef={terminal.ref} shown={terminalShown} />
        </section>
      )}
      {view._tag === 'Missions' ? (
        <React.Suspense fallback={<div {...stylex.props(styles.paneNotice)}><SystemNotice kind="loading" title="Loading missions" /></div>}>
          <MissionsPane {...(onOpenAgent === undefined ? {} : { onOpenAgent })} {...(view.mission === undefined ? {} : { focusedMission: view.mission })} {...(onFocusMission === undefined ? {} : { onFocusMission })} />
        </React.Suspense>
      ) : null}
      {view._tag === 'Alert' && alertControls !== undefined ? (
        <React.Suspense fallback={<div {...stylex.props(styles.paneNotice)}><SystemNotice kind="loading" title="Loading alert" /></div>}>
          <AlertDetailsPane id={view.id} {...alertControls} {...(ux === undefined ? {} : { ux })} />
        </React.Suspense>
      ) : null}
      {view._tag === 'Usage' ? (
        <UsageCodeBoundary />
      ) : null}
      {view._tag === 'Document' && onOpenDocument !== undefined ? (
        <React.Suspense fallback={<div {...stylex.props(styles.paneNotice)}><SystemNotice kind="loading" title="Loading document" /></div>}>
          <DocumentPane key={view.ref} name={view.ref} onOpen={onOpenDocument} />
        </React.Suspense>
      ) : null}
      {current === '' && view._tag !== 'Missions' && view._tag !== 'Usage' && view._tag !== 'Alert' ? <div {...stylex.props(styles.empty)}>Choose an agent to open its live thread.</div>
        : view._tag === 'ViewUnavailable' ? <div {...stylex.props(styles.paneNotice)}><SystemNotice kind="empty" title="View not available here" detail={view.reason.replace(/\.$/, '')} /></div>
          : null}
    </div>
  </ViewportStoreContext.Provider>
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
  textButton: { padding: '4px 8px', backgroundColor: 'transparent', color: c.fgMuted, borderWidth: 0, borderRadius: 6, cursor: 'pointer', fontSize: t.metaSize },
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
  resourcesDock: { display: 'flex', flexDirection: 'column', flexGrow: 1, minWidth: 0, minHeight: 0, height: '100%' },
  body: { position: 'relative', display: 'flex', flexDirection: 'column', flexGrow: 1, minWidth: 0, minHeight: 0, overflow: 'hidden' },
  phoneHidden: { display: 'none' },
  // Identical geometry in both states: revealing a pane must not invalidate its layout. Only
  // non-inherited properties toggle (no inert/pointer-events/visibility), so a reveal does not
  // restyle the retained subtree; content-visibility already removes it from focus and a11y.
  retainedPane: { position: 'absolute', inset: 0, display: 'flex', flexDirection: 'column', minWidth: 0, minHeight: 0, backgroundColor: c.canvas },
  shownPane: { zIndex: 1 },
  hiddenPane: { contentVisibility: 'hidden' },
  threadArea: { position: 'relative', display: 'flex', flexDirection: 'column', flexGrow: 1, flexBasis: 0, minWidth: 0, minHeight: 0 },
  terminalPane: { overflow: 'hidden', outline: 'none' },
  tabs: { height: 32, display: 'flex', alignItems: 'center', gap: s.xs, paddingInline: s.lg, borderBottomWidth: 1, borderBottomStyle: 'solid', borderBottomColor: c.border, flexShrink: 0 },
  // Hidden retained panes stay absolutely positioned and opaque; pane notices sit above them.
  paneNotice: { position: 'relative', zIndex: 2 },
  empty: { position: 'relative', zIndex: 2, margin: 'auto', padding: s.section, color: c.fgMuted },
  changesPanel: { display: 'flex', flexDirection: 'column', flexShrink: 0, minWidth: 0, minHeight: 0, overflowY: 'auto', borderLeftWidth: 1, borderLeftStyle: 'solid', borderLeftColor: c.border },
  sheetPanel: { flexGrow: 1, borderLeftWidth: 0 },
  toolDetailHeader: { height: 40, display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: s.md, paddingInline: s.lg, flexShrink: 0, borderBottomWidth: 1, borderBottomStyle: 'solid', borderBottomColor: c.border },
  toolDetailTitle: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontSize: t.metaSize },
  toolDetailOutput: { margin: 0, padding: s.lg, minHeight: 0, overflowY: 'auto', fontFamily: t.fontMono, fontSize: t.denseSize, lineHeight: 1.5, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere', color: c.fgMuted },
})
