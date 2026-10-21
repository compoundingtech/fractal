import { cancelMeasure, markStart, measureEnd } from './perfMarks.ts'
import { sidebarRow } from './sidebarRow.ts'
import { ConversationPaneFallback } from './ConversationPaneFallback.tsx'
import type * as ConversationPaneModule from './ConversationPane.tsx'
import { ConversationHeaderActions } from './ConversationHeaderActions.tsx'
import { DeclarationDialog } from './DeclarationDialog.tsx'
import { AgentActions } from './AgentActions.tsx'
import { retainPane, workspaceHref, workspaceView, type RetainedPane, type WorkspaceView } from './workspaceView.ts'
import { MISSIONS_REF, paneTabLabel } from './workspaceView.ts'
import { ThreadHeaderSlotContext } from '../shell/threadHeaderSlot.tsx'
import { liveLegacyTheme } from '../ui-compat/live-theme.stylex.ts'
import { colorVars as c, typeVars as t, spaceVars as s, geometryVars as g, geometryNumbers } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import * as React from 'react'
import { RegisteredPortal, usePortalContainerRegistration } from '../../../../packages/fractal-ui/src/assistant-ui/taste/PortalContainer.tsx'
import * as Aria from 'react-aria-components'
import { UNSAFE_PortalProvider } from 'react-aria'
import * as stylex from '@stylexjs/stylex'
import { useAtom, useAtomValue } from '@effect/atom-react'
import { Schema } from 'effect'
import * as Atom from 'effect/reactivity/Atom'
import { ChromeControl, SidebarAgentRow, SidebarQueryContext, sidebarRowAccessibleName, SystemNotice, ConnectionStatus, ThreadHeader, ResizableSplit, DockedSplit, NarrowSheet, NarrowActionRow, assistantDarkTheme, liveComposerDarkTheme, liveAccentTheme, compositionLightTheme, type SidebarActions } from '@fractal/ui/assistant-ui/shell'
// Deep import: the assistant-ui barrel would pull the transcript graph into the eager shell.
import { Icon as CompositionIcon } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Icons.tsx'
import type { WorkLogCall } from '@fractal/ui/assistant-ui'
import { ViewportStore, ViewportStoreContext } from '../../../../packages/fractal-ui/src/assistant-ui/EmbraceScrollViewport.tsx'
import { terminalSubjectForAgent } from '../data/projections.ts'
import { useFleet, useAgent, useSubjectList, useConnection, useNow, useFeedInterest, useDataSource, useRosterDecode, useAskingAgents } from '../data/react.tsx'
import { persistedAtom } from '../state/persistence.ts'
import { setTheme } from '../ui-compat/foundations.ts'
import { WorkbenchContextProvider, ResourcePanelProvider, MonitorDetailProvider, type OpenRequest } from '../shell/context.tsx'
import type { ResourcePanelState } from '../shell/state.ts'
import { afterNextPaint, type UxTelemetry } from '../telemetry/ux.ts'
import { syncLine } from '../../../../packages/fractal-ui/src/assistant-ui/st3-views/sync-line.ts'
import { useGatewaySync } from '../data/react.tsx'
import { NarrowLayoutContext } from '@fractal/ui/assistant-ui/shell'
import { PhoneAgentDrawer, type PhoneAgentDrawerHandle } from './PhoneAgentDrawer.tsx'
import { PhoneWorkspaceTabs } from './PhoneWorkspaceTabs.tsx'
import { AgentFolders } from '../shell/AgentFolders.tsx'
import { useSidebarFolders, type SidebarFoldersBinding } from '../folders/useSidebarFolders.ts'
import { fixtureFolders } from '../folders/fixture.ts'
import { defaultFilters, fixtureSidebarState, sidebarState } from '../shell/sidebar/state.ts'
import { useMachineMemory, withRememberedMachines } from '../shell/sidebar/show.ts'
import { parseSidebarQuery } from '../shell/sidebar/filter.ts'
import { SidebarToolbar, type SidebarToolbarProps } from '../shell/sidebar/Toolbar.tsx'
import type { Agent } from '../data/source.ts'
import { sidebarAttention } from './sidebarAttention.ts'
import { FolderAttentionSignal } from './AttentionControls.tsx'
import { liveShowRows, liveSidebarTree } from './liveSidebarTree.ts'
import { RosterLoading, RosterStaleNotice, RosterUnavailable, SidebarEmptyState } from './SidebarEmptyState.tsx'
import { useAgentsLastObservedAt } from './lastObserved.tsx'
import { RosterDecodeNotice, shownRosterRefs } from './LiveRoster.tsx'

const emptyAgents: readonly Agent[] = []
const sidebarExtraSignals = [] as const

/** A last-observed roster's observation instant: its row times stop there instead of ticking on as if live. */
const RosterFrozenAt = React.createContext<number | undefined>(undefined)

/** Clock ticks reach SidebarTime leaves, never the Aria row or its collection. */
const LiveSidebarAgentRow = React.memo((props: {
  readonly agentRef: string
  readonly active: boolean
  readonly select: (ref: string) => void
  readonly actions: SidebarActions
}) => {
  const feed = useAgent(props.agentRef)
  const connection = useConnection()
  const agent = feed._tag === 'Observed' ? feed.value : undefined
  const stale = feed._tag === 'Observed' && (feed.freshness === 'stale' || connection._tag !== 'Live')
  const frozenAt = React.useContext(RosterFrozenAt)
  const item = React.useMemo(() => agent === undefined ? undefined : sidebarRow({ agent, stale }), [agent, stale])
  const onOpen = React.useCallback(() => props.select(props.agentRef), [props.select, props.agentRef])
  return item === undefined ? null : <SidebarAgentRow inTree item={item} variant="SR-2" layout="SR2-A" glyph="SG-1" extraSignals={sidebarExtraSignals} active={props.active} actions={props.actions} onOpen={onOpen} {...(stale && frozenAt !== undefined ? { now: frozenAt } : {})} />
})
LiveSidebarAgentRow.displayName = 'LiveSidebarAgentRow'

const LiveSidebarTreeView = React.memo((props: {
  readonly binding: SidebarFoldersBinding
  readonly tree: SidebarFoldersBinding['tree']
  readonly accessibleNames: ReadonlyMap<string, string>
  readonly needsYou: ReadonlySet<string>
  readonly current: string
  readonly query: string
  readonly revealKey: string
  readonly select: (ref: string) => void
  readonly actions: SidebarActions
}) => {
  const renderAgent = React.useCallback((node: Extract<SidebarFoldersBinding['tree'][number], { readonly _tag: 'Agent' }>) =>
    <LiveSidebarAgentRow agentRef={node.subject} active={props.current === node.subject} select={props.select} actions={props.actions} />, [props.current, props.select, props.actions])
  const renderFolderSignal = React.useCallback((folder: Exclude<SidebarFoldersBinding['tree'][number], { readonly _tag: 'Agent' }>) =>
    <FolderAttentionSignal folder={folder} refs={props.needsYou} />, [props.needsYou])
  return <SidebarQueryContext.Provider value={props.query}><AgentFolders {...props.binding} tree={props.tree} accessibleNames={props.accessibleNames} selectedId={props.current} revealKey={props.revealKey} onSelect={props.select} renderFolderSignal={renderFolderSignal} renderAgent={renderAgent} /></SidebarQueryContext.Provider>
})
LiveSidebarTreeView.displayName = 'LiveSidebarTreeView'

// A static import would evaluate the kit Transcript/Markdown/refractor before the shell paints.
// Demand and speculative prefetch share one in-flight request; rejection releases it for retry.
let conversationPaneModule: Promise<typeof ConversationPaneModule> | undefined
const loadConversationPane = () => {
  if (conversationPaneModule === undefined) {
    conversationPaneModule = import('./ConversationPane.tsx').catch(error => {
      conversationPaneModule = undefined
      throw error
    })
  }
  return conversationPaneModule
}
const ConversationPane = React.lazy(() => {
  // An initially selected pane and later shell prefetch must retain the same lazy identity.
  // A second wrapper suspends on an already resolved module during the first seat switch.
  prefetchedConversationPane ??= ConversationPane
  return loadConversationPane().then(module => ({ default: module.ConversationPane }))
})
let prefetchedConversationPane: typeof ConversationPane | undefined
const prefetchConversationPane = () => {
  const request = loadConversationPane()
  // Preserve this attempt's failure for the requested pane's honest, local error surface.
  // A retry creates a fresh lazy wrapper, not React's permanently rejected cached wrapper.
  prefetchedConversationPane ??= React.lazy(() => request.then(module => ({ default: module.ConversationPane })))
  void request.catch(() => {})
}

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
      <button type="button" onClick={() => {
        const Pane = React.lazy(() => loadConversationPane().then(module => ({ default: module.ConversationPane })))
        prefetchedConversationPane = Pane
        this.setState({ failed: false, Pane })
      }}>Try again</button>
    </div>
    const Pane = this.state.Pane
    return <React.Suspense fallback={<ConversationPaneFallback agentName={this.props.agentName} visible={this.props.visible} />}>
      <Pane {...this.props} />
    </React.Suspense>
  }
}

// The kit diff viewer imports Markdown; keep it outside the shell's eager graph too.
const ChangesInspector = React.lazy(() => import('./ChangesInspector.tsx').then(module => ({ default: module.ChangesInspector })))
// The terminal grid, fonts and native key encoder load only when a terminal pane opens.
const TerminalDetail = React.lazy(() => import('../terminal/TerminalDetail.tsx').then((module) => ({ default: module.TerminalDetail })))
const MissionsPane = React.lazy(() => import('../missions/MissionsPane.tsx').then((module) => ({ default: module.MissionsPane })))
const DocumentPane = React.lazy(() => import('../documents/DocumentPane.tsx').then((module) => ({ default: module.DocumentPane })))

const sidebarRatio = persistedAtom({ key: 'round2.sidebarRatio', schema: Schema.Number, defaultValue: 256 / 1440 })
const panelRatio = persistedAtom({ key: 'round2.panelRatio', schema: Schema.Number, defaultValue: 380 / 1440 })
const sidebarClosed = persistedAtom({ key: 'round2.sidebarClosed', schema: Schema.Boolean, defaultValue: false })
const palette = persistedAtom({ key: 'round2.scheme', schema: Schema.Literals(['dark', 'light']), defaultValue: 'dark' })
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
const initialAgentFromUrl = () => {
  const encoded = window.location.pathname.match(/^\/w\/(.+)$/)?.[1]
  try {
    return encoded === undefined ? undefined : decodeURIComponent(encoded)
  } catch {
    return undefined
  }
}

/**
 * React Aria overlays (menus, popovers, tooltips, modals) portal into a host inside the themed
 * root instead of document.body, where only the legacy document palette and fonts apply. The
 * provider sits above the shell state, so its context value never changes on a switch and the
 * portalled panes inherit it without a context scan.
 */
export function LiveAgentWorkspace(props: { readonly ux?: UxTelemetry; readonly onSelectConversation?: (ref: string) => void }) {
  const overlayHost = React.useRef<HTMLDivElement>(null)
  const getContainer = React.useCallback(() => overlayHost.current, [])
  usePortalContainerRegistration(getContainer)
  return <UNSAFE_PortalProvider getContainer={getContainer}><Workspace {...props} overlayHost={overlayHost} /></UNSAFE_PortalProvider>
}

/** One authoritative agent thread per workspace; local geometry never grants backend authority. */
function Workspace({ ux, onSelectConversation, overlayHost }: { readonly ux?: UxTelemetry; readonly onSelectConversation?: (ref: string) => void; readonly overlayHost: React.RefObject<HTMLDivElement | null> }) {
  const fleet = useFleet(), subjects = useSubjectList(), connection = useConnection(), rosterDecode = useRosterDecode()
  const [storedAgent, setStoredAgent] = useAtom(selectedAgent)
  const [phoneAgents, setPhoneAgents] = useAtom(phoneAgentTabs)
  const [ratio, setRatio] = useAtom(sidebarRatio), [collapsed, setCollapsed] = useAtom(sidebarClosed), [scheme, setScheme] = useAtom(palette)
  const agentsDrawer = React.useRef<PhoneAgentDrawerHandle>(null)
  const subscribeNarrow = React.useCallback((callback: () => void) => {
    const query = window.matchMedia(narrowLayoutQuery)
    query.addEventListener('change', callback)
    return () => query.removeEventListener('change', callback)
  }, [])
  const narrow = React.useSyncExternalStore(subscribeNarrow, narrowLayoutMatches, serverNarrow)
  React.useLayoutEffect(() => setTheme(scheme), [scheme])
  const viewport = React.useSyncExternalStore(subscribeViewport, viewportWidth, viewportWidth)
  const phoneHeight = React.useSyncExternalStore(subscribePhoneViewport, phoneViewportHeight, phoneViewportHeight)
  const phoneViewportStyle = narrow ? { height: `calc(${phoneHeight}px - var(--wf-devbar-space, 0px))`, '--wf-phone-height': `${phoneHeight}px` } : undefined
  const location = React.useSyncExternalStore(subscribeLocation, locationState, locationState)
  const hoverOwner = React.useRef<HTMLElement | null>(null)
  const rememberHoverOwner = (event: React.SyntheticEvent<HTMLElement>) => {
    hoverOwner.current = event.target instanceof Element ? event.target.closest<HTMLElement>('[data-wf-agent-ref]') : null
  }
  React.useLayoutEffect(() => {
    // Use the row's existing Escape contract: it cancels pending hover intent as well
    // as closing the card, without replacing any roster nodes during a switch.
    hoverOwner.current?.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }))
    hoverOwner.current = null
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
    return () => { cancelTiming(); cancelCapture?.() }
  }, [ux, rosterObserved])
  const current = initialAgentFromUrl() ?? (storedAgent || agents[0]?.ref || '')
  const agent = agents.find((row) => row.ref === current)
  // A canonical subject address is not proof of a terminal. Keep the control disabled
  // from its first render until the roster can establish the selected agent's runtime.
  const terminalDisabledReason = terminalSubjectForAgent(current) === undefined ? undefined
    : fleet._tag === 'Waiting' ? 'Waiting for the live agent roster to find this agent’s terminal.'
      : fleet._tag === 'Unavailable' ? 'The agent roster is unavailable, so this agent’s terminal cannot be found.'
        : agent === undefined ? 'This agent is not in the live agent roster, so it has no terminal.'
          : agent.terminalDisabledReason
  const terminalRef = terminalDisabledReason === undefined ? agent?.terminal : undefined
  const [headerSlot, setHeaderSlot] = React.useState<HTMLDivElement | null>(null)
  const [paneHost, setPaneHost] = React.useState<HTMLDivElement | null>(null)
  const [panes, setPanes] = useAtom(workspacePanes(current))
  const query = new URLSearchParams(location.split('?')[1])
  const selectedPane = query.get('open') ?? 'thread'
  const [resourcePanel, setResourcePanel] = React.useState<ResourcePanelState>({ expanded: false, size: 288 })
  const [diffOpen, setDiffOpen] = React.useState(false)
  const [openedTool, setOpenedTool] = React.useState<WorkLogCall | undefined>(undefined)
  // The row menu opens one declaration sheet; a stable action object keeps rows memoizable.
  const [declarationRef, setDeclarationRef] = React.useState<string | undefined>(undefined)
  const sidebarActions = React.useMemo<SidebarActions>(() => ({ declaration: setDeclarationRef }), [])
  const terminalReasonId = React.useId()
  const [panelFraction, setPanelFraction] = useAtom(panelRatio)
  const [dragWidth, setDragWidth] = React.useState<number | undefined>()
  const [dragPanel, setDragPanel] = React.useState<number | undefined>()
  const navigate = (ref: string, pane = 'thread') => {
    setStoredAgent(ref)
    window.history.pushState(null, '', workspaceHref(ref, pane, window.location.search))
    window.dispatchEvent(new PopStateEvent('popstate'))
  }
  const select = (ref: string) => {
    if (narrow) setPhoneAgents(refs => [...new Set([...refs, current, ref])].filter(Boolean))
    if (ref !== current) {
      cancelMeasure('open_transcript')
      markStart('switch_to_tail', { key: ref })
      onSelectConversation?.(ref)
    }
    setDiffOpen(false)
    setOpenedTool(undefined)
    agentsDrawer.current?.close()
    navigate(ref)
  }
  const open = (request: OpenRequest) => {
    if (request.ref.startsWith('agent/') && request.presentation === undefined) {
      select(request.ref)
      return
    }
    const id = `${request.ref}:${request.presentation ?? 'detail'}`
    setPanes((rows) => (rows.some((row) => `${row.ref}:${row.presentation ?? 'detail'}` === id) ? rows : [...rows, { ref: request.ref, ...(request.presentation === undefined ? {} : { presentation: request.presentation }) }]))
    navigate(current, id)
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
      return row.id === agent.ref && row.subject === agent.ref && row.label === agent.name && row.host === agent.host
    })) return previous
    const next = agents.map(row => ({ id: row.ref, subject: row.ref, label: row.name, host: row.host }))
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
  // Filtering uses observed status, not wall time; ticking cannot change membership.
  const visibleSidebarTree = React.useMemo(() => liveSidebarTree({ tree: sidebarFolders.orderingTree, agents, needsYou, asking, filters: deferredFilters, stale }), [sidebarFolders.orderingTree, agents, needsYou, asking, deferredFilters, stale])
  const showRows = React.useMemo(() => liveShowRows({ agents, needsYou, asking, filters: deferredFilters, stale }), [agents, needsYou, asking, deferredFilters, stale])
  const previousAccessibleNames = React.useRef<ReadonlyMap<string, string> | undefined>(undefined)
  const accessibleNames = React.useMemo(() => {
    const next = new Map(agents.map(agent => [agent.ref, sidebarRowAccessibleName(sidebarRow({ agent, stale }))] as const))
    const previous = previousAccessibleNames.current
    if (previous !== undefined && next.size === previous.size && [...next].every(([ref, name]) => previous.get(ref) === name)) return previous
    previousAccessibleNames.current = next
    return next
  }, [agents, stale])
  const selectLatest = React.useRef(select)
  selectLatest.current = select
  const selectAgent = React.useCallback((ref: string) => selectLatest.current(ref), [])
  const byRef = React.useMemo(() => new Map(subjects.map((subject) => [subject.ref, subject])), [subjects])
  const separator = selectedPane.lastIndexOf(':')
  const urlPresentation = selectedPane.slice(separator + 1)
  const urlPane: OpenRequest | undefined =
    separator > 0 && (urlPresentation === 'detail' || urlPresentation === 'overview' || urlPresentation === 'resources')
      ? { ref: selectedPane.slice(0, separator), presentation: urlPresentation }
      : undefined
  const chosen = panes.find((row) => `${row.ref}:${row.presentation ?? 'detail'}` === selectedPane) ?? urlPane
  const visiblePanes = chosen && !panes.some((row) => `${row.ref}:${row.presentation ?? 'detail'}` === selectedPane) ? [...panes, chosen] : panes
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
  const width = dragWidth ?? Math.round(ratio * viewport)
  const maxPanel = Math.max(240, viewport - (collapsed ? 40 : Math.min(width, Math.max(208, Math.min(440, viewport - 640)))) - 640)
  const panelSize = dragPanel ?? Math.min(maxPanel, Math.max(240, Math.round(panelFraction * viewport)))
  const maxSidebar = Math.max(208, Math.min(440, viewport - 640))
  const agentName = agent?.name ?? current.split('/').at(-1) ?? 'Conversation'
  const changesWorkspace = agent?.workspace._tag === 'Known' ? JSON.stringify([agent.host, agent.workspace.value]) : current
  const terminalOpen = selectedPane.includes('terminal/')
  const toggleTerminal = () => { if (selectedPane.startsWith('terminal/')) navigate(current); else if (terminalRef !== undefined) open({ ref: terminalRef }) }
  const agentHeaderActions = current === '' ? undefined : <>{agent === undefined ? null : <AgentActions key={`actions-${current}`} agent={agent} />}<ConversationHeaderActions key={current} agentRef={current} /></>
  // Below the breakpoint Details is a sheet over the thread, never an inline panel beside it.
  const narrowDetailsOpen = narrow && current !== '' && (diffOpen || openedTool !== undefined)
  const closeDetails = () => { setDiffOpen(false); setOpenedTool(undefined) }
  const toolDetail = (call: WorkLogCall, width: number | undefined) => <aside aria-label="Tool detail" style={width === undefined ? undefined : { width }} {...stylex.props(styles.changesPanel, width === undefined && styles.sheetPanel)}>
    <header {...stylex.props(styles.toolDetailHeader)}>
      <strong {...stylex.props(styles.toolDetailTitle)}>{call.title}{call.argsSummary === undefined ? '' : ` ${call.argsSummary}`}</strong>
      <Aria.Button aria-label="Close tool detail" onPress={() => setOpenedTool(undefined)} {...stylex.props(styles.iconButton)}>×</Aria.Button>
    </header>
    <pre {...stylex.props(styles.toolDetailOutput)}>{call.detail ?? (call.status === 'running' ? 'No output received yet.' : 'No output recorded.')}</pre>
  </aside>
  // One roster for both presentations: docked beside the thread, or the agents sheet below the breakpoint.
  const sidebarBody = <>
    <LiveSidebarToolbar filters={filters} update={updateFilters} hosts={hosts} showRows={showRows} onEnterResults={() => rosterRef.current?.querySelector<HTMLElement>('[role="row"]')?.focus()} />
    {fleet._tag === 'Unavailable' ? <RosterUnavailable reason={fleet.reason} onRetry={fleet.reason === 'failed' && fleet.retryable !== false ? folderSource.retryAgents : undefined} />
      : fleet._tag !== 'Observed' ? null
      : <>
        {stale ? <RosterStaleNotice decodeStale={rosterDecode.decodeStale} observedAt={rosterObservedAt} partial={fleet.coverage?._tag === 'Partial'} refreshFailed={fleet.error?.reason === 'failed'} onRetry={folderSource.retryAgents} /> : null}
        {agents.length === 0 ? <SidebarEmptyState kind="empty" />
          : visibleSidebarTree.length === 0 ? <SidebarEmptyState kind="no-match" query={deferredFilters.query} onClear={() => setFilters({ ...defaultFilters, sort: filters.sort })} />
          : null}
      </>}
    <nav ref={rosterCommit} aria-label="Agent roster" onMouseOverCapture={rememberHoverOwner} onFocusCapture={rememberHoverOwner} {...stylex.props(styles.roster)}>
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
        <LiveSidebarTreeView binding={sidebarFolders} tree={visibleSidebarTree} accessibleNames={accessibleNames} needsYou={needsYou} current={current} query={sidebarQuery.text} revealKey={deferredFilters.query} select={selectAgent} actions={sidebarActions} />
      </RosterFrozenAt.Provider>
    </nav>
    {fleet._tag === 'Observed' ? <RosterDecodeNotice decode={rosterDecode} shown={shownRosterRefs(fleet)} /> : null}
    <footer {...stylex.props(styles.footer)}>
      <ChromeControl icon="swap" label="Toggle color scheme" tooltipPlacement="top" onPress={() => setScheme(scheme === 'dark' ? 'light' : 'dark')} />
      <ChromeControl icon="checklist" label="Missions" tooltipPlacement="top" onPress={() => open({ ref: MISSIONS_REF, presentation: 'detail' })} />
      <ChromeControl icon="gauge" label="Usage" tooltipPlacement="top" onPress={() => open({ ref: 'monitor/quota', presentation: 'detail' })} />
    </footer>
  </>
  // The panes stay outside changing shell providers, whose context propagation would scan every retained turn.
  // Only the stable header DOM host and narrow geometry cross that boundary.
  return (<NarrowLayoutContext.Provider value={narrow}>
    <ThreadHeaderSlotContext.Provider value={narrow ? null : headerSlot}>
    <WorkbenchContextProvider value={{ open, focusedRef: address.ref || null, subjects: byRef, platform: navigator.platform.includes('Mac') ? 'mac' : 'other' }}>
      <ResourcePanelProvider value={{ state: resourcePanel, onChange: (change) => setResourcePanel((value) => ({ ...value, ...change })) }}>
        <MonitorDetailProvider value={{ size: 380, onSizeChange: (value) => setPanelFraction(value / viewport) }}>
            <div ref={shellCommit} data-testid="live-agent-workspace" data-scheme={scheme} style={phoneViewportStyle} {...stylex.props(styles.app, styles.legacyBridge, liveLegacyTheme, scheme === 'dark' && assistantDarkTheme, scheme === 'dark' && liveComposerDarkTheme, liveAccentTheme, scheme === 'light' && compositionLightTheme)}>
              {narrow ? null : <>
                <aside aria-label="Agents" style={{ width: collapsed ? 40 : Math.min(width, maxSidebar) }} {...stylex.props(styles.sidebar)}>
                  <header {...stylex.props(styles.brand)}>
                    <ChromeControl icon={collapsed ? 'chevron-right' : 'chevrons-left'} label={collapsed ? 'Expand agents' : 'Collapse agents'} onPress={() => setCollapsed(!collapsed)} />
                    {collapsed ? null : <strong>Fractal</strong>}
                  </header>
                  {collapsed ? null : sidebarBody}
                </aside>
                <ResizableSplit id="live-agent-sidebar" value={width} min={208} max={maxSidebar} collapsed={collapsed} label="Agent sidebar width" onChange={(value) => { setCollapsed(false); setDragWidth(value) }} onCommit={(value) => { setRatio(value / viewport); setDragWidth(undefined) }} onToggle={() => setCollapsed(!collapsed)} onReset={() => { setCollapsed(false); setRatio(256 / viewport) }} />
              </>}
              <DockedSplit persistKey={`live.${changesWorkspace}.changes`} label="Resize changes" minMainSize={geometryNumbers.panelMin} onCollapse={() => setDiffOpen(false)} dock={!narrow && diffOpen && openedTool === undefined && current !== '' ? (
                <React.Suspense fallback={<aside aria-label="Changes" {...stylex.props(styles.changesPanel, styles.sheetPanel)}><SystemNotice kind="loading" title="Loading captured changes" /></aside>}>
                  <ChangesInspector key={current} agentRef={current} width="dock" onClose={() => setDiffOpen(false)} />
                </React.Suspense>
              ) : undefined}>
              <section aria-label="Agent workspace" {...stylex.props(styles.workspace)}>
                {narrow ? <PhoneAgentDrawer ref={agentsDrawer} header={(showAgents, sheet) => <ClockedThreadHeader agent={agent} stale={stale} narrow sidebarCollapsed onToggleSidebar={showAgents} sidebarSheet={sheet} terminalAvailable={terminalRef !== undefined} {...(terminalDisabledReason === undefined ? {} : { terminalDisabledReason })} actionPortalRef={setHeaderSlot} folder={agent?.host} title={agent?.name ?? (current || 'Select an agent')} freshness={stale ? 'stale' : agent === undefined ? 'unobserved' : 'live'} panelOpen={narrowDetailsOpen} drawerOpen={terminalOpen} onTogglePanel={() => { if (narrowDetailsOpen) closeDetails(); else setDiffOpen(true) }} onToggleDrawer={toggleTerminal} />}>{sidebarBody}</PhoneAgentDrawer> : <ClockedThreadHeader agent={agent} stale={stale} terminalAvailable={terminalRef !== undefined} {...(terminalDisabledReason === undefined ? {} : { terminalDisabledReason })} nativeActions={agentHeaderActions} actionPortalRef={setHeaderSlot} folder={agent?.host} title={agent?.name ?? (current || 'Select an agent')} freshness={stale ? 'stale' : agent === undefined ? 'unobserved' : 'live'} panelOpen={diffOpen} drawerOpen={terminalOpen} onTogglePanel={() => setDiffOpen(value => !value)} onToggleDrawer={toggleTerminal} />}
                <GatewayConnectionStatus />
                {!narrow && visiblePanes.length > 0 ? (
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
                <PhoneWorkspaceTabs enabled={narrow} tabs={phoneTabs} selected={selectedPane === 'thread' ? current : selectedPane} onSelect={id => id === current || agents.some(row => row.ref === id) ? select(id) : navigate(current, id)}>
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
              {narrow ? <NarrowSheet label="Agent details" isOpen={narrowDetailsOpen} onOpenChange={(isOpen) => { if (!isOpen) closeDetails() }} width={400} actions={
                <NarrowActionRow label="Agent actions">
                  {agentHeaderActions}
                  {terminalRef === undefined && terminalDisabledReason === undefined ? null : <>
                    <Aria.Button aria-pressed={terminalOpen} isDisabled={!terminalOpen && terminalRef === undefined} aria-describedby={terminalRef === undefined ? terminalReasonId : undefined} onPress={() => { closeDetails(); toggleTerminal() }} {...stylex.props(styles.textButton, styles.narrowAction)}>
                      <CompositionIcon name="drawer" />Terminal
                    </Aria.Button>
                    {terminalRef === undefined ? <span id={terminalReasonId} hidden>{terminalDisabledReason}</span> : null}
                  </>}
                </NarrowActionRow>
              }>
                {current === '' ? null : openedTool !== undefined ? toolDetail(openedTool, undefined) : (
                  <React.Suspense fallback={<SystemNotice kind="loading" title="Loading captured changes" />}>
                    <ChangesInspector key={current} agentRef={current} width="fill" />
                  </React.Suspense>
                )}
              </NarrowSheet> : null}
              <div ref={overlayHost} {...stylex.props(styles.overlayHost)} />
            </div>
        </MonitorDetailProvider>
      </ResourcePanelProvider>
    </WorkbenchContextProvider>
    {paneHost === null ? null : <RegisteredPortal container={paneHost}><WorkspaceBody current={current} rosterRefs={agents.map(agent => agent.ref)} view={workspaceView(chosen)} hidden={narrow && selectedPane === 'changes'} agentName={agentName} onOpenTool={setOpenedTool} onOpenDocument={openDocument} onOpenAgent={select} ux={ux} /></RegisteredPortal>}
    </ThreadHeaderSlotContext.Provider>
  </NarrowLayoutContext.Provider>)
}

/** Only time-bearing surfaces subscribe to the wall clock, not the workspace providers. */
function ClockedThreadHeader({ agent, stale, ...props }: Omit<React.ComponentProps<typeof ThreadHeader>, 'now' | 'status' | 'statusLabel' | 'statusSince'> & { readonly agent?: Agent; readonly stale: boolean }) {
  const row = agent === undefined ? undefined : sidebarRow({ agent, stale })
  return <ThreadHeader {...props} status={row?.status} statusLabel={row?.statusLabel} statusSince={row?.statusSince} />
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
      ? `Next try in ${Math.ceil(Math.max(0, status.reason.nextAt - now) / 1000)}s · attempt ${status.reason.attempt}`
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
export function WorkspaceBody({ current, rosterRefs, view, hidden = false, agentName, onOpenTool, onOpenDocument, onOpenAgent, ux }: { readonly current: string; readonly rosterRefs: readonly string[]; readonly view: WorkspaceView; readonly hidden?: boolean; readonly agentName: string; readonly onOpenTool: (call: WorkLogCall) => void; readonly onOpenDocument?: (ref: string) => void; readonly onOpenAgent?: (ref: string) => void; readonly ux?: UxTelemetry }) {
  // Reading positions belong to this surface, not its three-pane DOM cache. A new
  // surface (including reload) starts following live; evicted panes keep memory here.
  const [viewportStore] = React.useState(() => new ViewportStore())
  React.useEffect(() => {
    viewportStore.open()
    return () => viewportStore.dispose()
  }, [viewportStore])
  const [retained, setRetained] = React.useState<readonly RetainedPane[]>([])
  const [switched, setSwitched] = React.useState<{ readonly shown: string; readonly covered: string | undefined }>({ shown: current, covered: undefined })
  // The terminal is a drawer below the thread: the thread stays shown and usable beside it.
  const visible = !hidden && current !== '' && (view._tag === 'Thread' || view._tag === 'Terminal')
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
    <div {...stylex.props(styles.threadArea)}>
      {retained.map(pane => {
        const shown = visible && pane.ref === current
        return <div key={pane.ref} {...stylex.props(styles.retainedPane, shown ? styles.shownPane : visible && pane.ref === switched.covered ? null : styles.hiddenPane)}>
          <ConversationPaneLoadBoundary agentRef={pane.ref} agentName={pane.name} onOpenTool={onOpenTool} onOpenDocument={onOpenDocument} ux={ux} visible={shown} />
        </div>
      })}
      {view._tag === 'Missions' ? (
        <React.Suspense fallback={<div {...stylex.props(styles.paneNotice)}><SystemNotice kind="loading" title="Loading missions" /></div>}>
          <MissionsPane {...(onOpenAgent === undefined ? {} : { onOpenAgent })} />
        </React.Suspense>
      ) : null}
      {view._tag === 'Document' && onOpenDocument !== undefined ? (
        <React.Suspense fallback={<div {...stylex.props(styles.paneNotice)}><SystemNotice kind="loading" title="Loading document" /></div>}>
          <DocumentPane key={view.ref} name={view.ref} onOpen={onOpenDocument} />
        </React.Suspense>
      ) : null}
      {current === '' && view._tag !== 'Missions' ? <div {...stylex.props(styles.empty)}>Choose an agent to open its live thread.</div>
        : view._tag === 'ViewUnavailable' ? <div {...stylex.props(styles.paneNotice)}><SystemNotice kind="empty" title="View not available here" detail={view.reason.replace(/\.$/, '')} /></div>
          : null}
    </div>
    {current !== '' && view._tag === 'Terminal' ? (
      <section key={view.ref} aria-label="Terminal drawer" data-testid="terminal-pane" {...stylex.props(styles.terminalDrawer)}>
        <React.Suspense fallback={<SystemNotice kind="loading" title="Loading terminal" />}>
          <TerminalDetail address={{ ref: view.ref, presentation: 'detail' }} visibility="visible" />
        </React.Suspense>
      </section>
    ) : null}
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
  sidebar: { display: 'flex', flexDirection: 'column', flexShrink: 0, minHeight: 0, overflow: 'hidden', backgroundColor: c.sidebar },
  brand: { height: g.band, paddingInline: s.lg, display: 'flex', alignItems: 'center', gap: s.md, flexShrink: 0 },
  iconButton: { width: 28, height: 28, display: 'inline-flex', justifyContent: 'center', alignItems: 'center', backgroundColor: 'transparent', color: c.fgMuted, borderWidth: 0, borderRadius: 6, cursor: 'pointer' },
  textButton: { padding: '4px 8px', backgroundColor: 'transparent', color: c.fgMuted, borderWidth: 0, borderRadius: 6, cursor: 'pointer', fontSize: t.metaSize },
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
  footer: { height: 40, display: 'flex', alignItems: 'center', gap: s.md, paddingInline: s.lg, flexShrink: 0 },
  workspace: { display: 'flex', flexDirection: 'column', flexGrow: 1, minWidth: 0, minHeight: 0 },
  body: { position: 'relative', display: 'flex', flexDirection: 'column', flexGrow: 1, minWidth: 0, minHeight: 0, overflow: 'hidden' },
  phoneHidden: { display: 'none' },
  // Identical geometry in both states: revealing a pane must not invalidate its layout. Only
  // non-inherited properties toggle (no inert/pointer-events/visibility), so a reveal does not
  // restyle the retained subtree; content-visibility already removes it from focus and a11y.
  retainedPane: { position: 'absolute', inset: 0, display: 'flex', flexDirection: 'column', minWidth: 0, minHeight: 0, backgroundColor: c.canvas },
  shownPane: { zIndex: 1 },
  hiddenPane: { contentVisibility: 'hidden' },
  threadArea: { position: 'relative', display: 'flex', flexDirection: 'column', flexGrow: 1, flexBasis: 0, minWidth: 0, minHeight: 0 },
  terminalDrawer: { display: 'flex', flexDirection: 'column', flexGrow: 0, flexShrink: 0, flexBasis: '45%', minHeight: 160, minWidth: 0, overflow: 'hidden', backgroundColor: c.canvas, borderTopWidth: 1, borderTopStyle: 'solid', borderTopColor: c.border },
  tabs: { height: 32, display: 'flex', alignItems: 'center', gap: s.xs, paddingInline: s.lg, borderBottomWidth: 1, borderBottomStyle: 'solid', borderBottomColor: c.border, flexShrink: 0 },
  // Hidden retained panes stay absolutely positioned and opaque; pane notices sit above them.
  paneNotice: { position: 'relative', zIndex: 2 },
  empty: { position: 'relative', zIndex: 2, margin: 'auto', padding: s.section, color: c.fgMuted },
  changesPanel: { display: 'flex', flexDirection: 'column', flexShrink: 0, minWidth: 0, minHeight: 0, overflowY: 'auto', borderLeftWidth: 1, borderLeftStyle: 'solid', borderLeftColor: c.border },
  sheetPanel: { flexGrow: 1, borderLeftWidth: 0 },
  narrowAction: { display: 'inline-flex', alignItems: 'center', gap: s.xs, ':focus-visible': { outline: `2px solid ${c.primary}` }, ':disabled': { opacity: 0.5, cursor: 'default' } },
  toolDetailHeader: { height: 40, display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: s.md, paddingInline: s.lg, flexShrink: 0, borderBottomWidth: 1, borderBottomStyle: 'solid', borderBottomColor: c.border },
  toolDetailTitle: { minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', fontSize: t.metaSize },
  toolDetailOutput: { margin: 0, padding: s.lg, minHeight: 0, overflowY: 'auto', fontFamily: t.fontMono, fontSize: t.denseSize, lineHeight: 1.5, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere', color: c.fgMuted },
})
