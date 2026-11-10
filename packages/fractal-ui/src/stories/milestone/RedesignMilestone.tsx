// Accepted RM1 reference composition (live app ports slices separately) of shipped kit parts (Transcript, WF-1, RC-3 chips,
// DiffPanel, Composer, Pane, DockedSplit, ResizableSplit) with the milestone's own roster tree, RC-1 subagent card and shell states.
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Dialog, Modal, ModalOverlay } from 'react-aria-components'
import { UNSAFE_PortalProvider } from 'react-aria'
import { ResizableSplit } from '../../assistant-ui/composition/Shell'
import { usePersistedSize } from '../../assistant-ui/composition/usePersistedSize'
import { DockedSplit } from '../../assistant-ui/composition/DockedSplit'
import { Pane, PaneHeaderButton, PaneHeaderToggle } from '../../assistant-ui/composition/Pane'
import { ConnectionStatus } from '../../assistant-ui/composition/SystemState'
import { DiffPanel, type DiffRevealRequest } from '../../assistant-ui/composition/DiffPanel'
import { ResourceChipV1 } from '../../assistant-ui/resources/ResourceCardV1'
import type { TranscriptProps, TranscriptTurn } from '../../assistant-ui/composition/Transcript'
import type { WorkLogCall } from '../../assistant-ui/taste/work-log'
import type { SyncStatus } from '../../assistant-ui/sync-presentation/sync-status'
import { SyncLine } from '../../assistant-ui/sync-presentation/SyncLine'
import type { SidebarAgentRow as Row } from '../../assistant-ui/sidebar/model'
import { baselineTheme } from '../../assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from '../../assistant-ui/composition-theme'
import { borderVars as border, elevationVars as elevation, geometryNumbers, geometryVars as g, radiusVars as r, spaceVars as s, surfaceVars as surface, textVars as ink, typeVars as t } from '../../assistant-ui/composition-tokens.stylex'
import { MilestoneSidebar, type RosterState } from './MilestoneSidebar'
import { MilestonePalette, type PaletteCommand } from './MilestonePalette'
import { AgentResultCard, MilestoneThread, PaneGlyph, type PaneTab, type ThreadView } from './MilestonePane'
import type { DiffFile } from '../../assistant-ui/composition/DiffPanel'
import type { ResourceData } from '../../assistant-ui/resources/resource-model'
import type { RosterFolder } from './MilestoneSidebar'

/** Presentation projections; stories adapt scenario slices before crossing this boundary. */
export type MilestoneLoad<TValue> =
  | { readonly _tag: 'loading'; readonly value: TValue }
  | { readonly _tag: 'ready'; readonly value: TValue }
  | { readonly _tag: 'failed'; readonly value: TValue; readonly reason: string; readonly detail: string }
  | { readonly _tag: 'stale'; readonly value: TValue; readonly observedAt: number }
export interface MilestoneRoster {
  readonly folders: readonly RosterFolder[]
  readonly inboxCount: number
  readonly query: string
}
export interface MilestoneConversation {
  readonly threads: Readonly<Record<string, { readonly turns: readonly TranscriptTurn[]; readonly running: boolean; readonly history?: TranscriptProps['history'] }>>
  /** Host-projected status, retained without reconstructing transport facts from a load tag. */
  readonly sync?: SyncStatus
  readonly primaryRef: string
  readonly reviewerRef: string
  readonly lastSeen: Extract<ThreadView, { readonly _tag: 'Stale' }>['lastSeen']
}
type Phase = 'slice' | 'loading' | 'empty-roster' | 'empty-thread' | 'no-match' | 'roster-error' | 'thread-unavailable' | 'stale' | 'navigation' | 'roster-retry' | 'thread-retry'

export interface RedesignMilestoneProps {
  readonly scheme: Scheme
  readonly roster: MilestoneLoad<MilestoneRoster>
  readonly conversation: MilestoneLoad<MilestoneConversation>
  readonly now: number
  readonly changedFiles: readonly DiffFile[]
  readonly branchFiles: readonly DiffFile[]
  readonly proseResources: Readonly<Record<string, ResourceData>>
  readonly reviewerCard: { readonly path: string; readonly summary: string }
  readonly defaultOpenRefs: readonly string[]
  readonly defaultChangesOpen: boolean
  /** Navigation is a local UI demonstration, not a data-load state. */
  readonly defaultPaletteOpen?: boolean
  /** Device-local key for persisted panel sizes; stories pass a per-story key so screenshots stay deterministic. */
  readonly persistKey: string
  /** The fetch a Retry performs; the failed surface shows loading until it settles. */
  readonly reload?: () => Promise<void>
}

/** Simulated network latency for browsing the workshop; plays pass a reload they settle themselves. */
const simulatedReload = () => new Promise<void>(resolve => { window.setTimeout(resolve, 900) })
const changesKey = 'changes'
const isMac = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform)
const modKey = isMac ? '⌘' : 'Ctrl+'
/** Opens at 304px so the two-line SR-2 row with its metrics line has room; the row stays two lines at every width (RM1-Q1). */
const sidebarDefault = 304
const narrowQuery = '(max-width: 639px)'
const subscribeNarrow = (notify: () => void) => {
  const media = window.matchMedia(narrowQuery)
  media.addEventListener('change', notify)
  return () => media.removeEventListener('change', notify)
}

export function RedesignMilestone(props: RedesignMilestoneProps) {
  const { changedFiles, branchFiles, proseResources, reviewerCard, now: milestoneNow } = props
  const { folders: rosterFolders, inboxCount } = props.roster.value
  const { primaryRef: mainAgentRef, reviewerRef, lastSeen } = props.conversation.value
  const firstOpenRef = props.defaultOpenRefs[0]
  const [recovery, setRecovery] = React.useState<Partial<Record<'roster' | 'conversation', { readonly input: RedesignMilestoneProps['roster'] | RedesignMilestoneProps['conversation']; readonly state: 'loading' | 'ready' }>>>({})
  const rosterRecovery = recovery.roster?.input === props.roster ? recovery.roster.state : undefined
  const conversationRecovery = recovery.conversation?.input === props.conversation ? recovery.conversation.state : undefined
  const rosterLoad = rosterRecovery ?? props.roster._tag
  const conversationLoad = conversationRecovery ?? props.conversation._tag
  const conversationPhase: Phase = conversationRecovery === 'loading' ? 'thread-retry'
    : conversationLoad === 'loading' ? 'loading'
    : conversationLoad === 'failed' ? 'thread-unavailable'
    : conversationLoad === 'stale' ? 'stale' : 'slice'
  // Preserve the reference canvas marker; rendering reads each slice independently below.
  const phase: Phase = rosterRecovery === 'loading' ? 'roster-retry' : (
    rosterLoad === 'failed' ? 'roster-error'
      : rosterLoad === 'loading' ? 'loading'
      : conversationPhase !== 'slice' ? conversationPhase
      : rosterLoad === 'stale' ? 'stale'
      : rosterFolders.length === 0 ? 'empty-roster'
      : props.roster.value.query !== '' ? 'no-match'
      : props.defaultOpenRefs.length === 1 && firstOpenRef !== undefined && props.conversation.value.threads[firstOpenRef]?.turns.length === 0 ? 'empty-thread'
      : props.defaultPaletteOpen ? 'navigation' : 'slice')
  const allRows = rosterFolders.flatMap(folder => folder.rows.flatMap(row => [row, ...row.children]))
  const rowFor = (ref: string) => allRows.find(row => row.ref === ref)
  const agentTab = (ref: string, current: Phase): PaneTab => {
    const row = rowFor(ref)
    const title = row?.title ?? ref
    if (current === 'loading' || current === 'thread-retry') return { key: ref, title, status: 'pending', statusLabel: 'Connecting' }
    if (current === 'thread-unavailable' || row === undefined) return { key: ref, title, status: 'offline', statusLabel: 'Unavailable' }
    if (current === 'stale' && ref === mainAgentRef) return { key: ref, title, status: 'stale', statusLabel: 'Last seen working' }
    return { key: ref, title, status: row.status, statusLabel: row.statusLabel }
  }
  const threadSync = (current: Phase): SyncStatus => {
    if (props.conversation.value.sync !== undefined) return props.conversation.value.sync
    if (current === 'loading' || current === 'thread-retry') return { _tag: 'Connecting', attempt: 1, since: milestoneNow - 1200 }
    if (current === 'stale') return { _tag: 'Stale', reason: { _tag: 'Reconnecting', attempt: 2, nextAt: milestoneNow + 4000, issue: 'Gateway connection interrupted' }, lastLiveAt: lastSeen.at }
    return { _tag: 'Live', since: milestoneNow - 30_000 }
  }
  const threadView = (current: Phase, key: string, onRetry: () => void): ThreadView => {
    if (current === 'loading' || current === 'thread-retry') return { _tag: 'Loading' }
    if (current === 'stale' && key === mainAgentRef) return { _tag: 'Stale', lastSeen }
    if (current === 'thread-unavailable' && props.conversation._tag === 'failed') return { _tag: 'Unavailable', reason: props.conversation.reason, detail: props.conversation.detail, onRetry }
    return { _tag: 'Live' }
  }
  const [scheme, setScheme] = React.useState(props.scheme)
  const narrow = React.useSyncExternalStore(subscribeNarrow, () => window.matchMedia(narrowQuery).matches, () => false)
  const [width, setWidth] = usePersistedSize(props.persistKey, sidebarDefault)
  // Docked: collapsed hides the sidebar. Narrow: the sidebar is a modal drawer, open only when the roster is the content.
  const [collapsed, setCollapsed] = React.useState(() => window.matchMedia(narrowQuery).matches && phase !== 'empty-roster' && phase !== 'roster-error')
  const [query, setQuery] = React.useState(props.roster.value.query)
  const [paletteOpen, setPaletteOpen] = React.useState(props.defaultPaletteOpen ?? false)
  const [tabs, setTabs] = React.useState(props.defaultOpenRefs)
  const [activeKey, setActiveKey] = React.useState(props.defaultOpenRefs[0] ?? '')
  const [changesOpen, setChangesOpen] = React.useState(props.defaultChangesOpen)
  const [revealRequest, setRevealRequest] = React.useState<DiffRevealRequest>()
  const [sent, setSent] = React.useState<Readonly<Record<string, readonly TranscriptTurn[]>>>({})
  const searchRef = React.useRef<HTMLInputElement>(null)
  // Only the latest Retry may finish recovery, and never after unmount.
  const attempt = React.useRef({ roster: 0, conversation: 0 })
  React.useEffect(() => () => { attempt.current = { roster: -1, conversation: -1 } }, [])
  const reload = props.reload ?? simulatedReload

  // Each Retry walks the real recovery path for its own fault only: the failed surface loads, then shows live data.
  const recover = React.useCallback((surface: 'roster' | 'conversation') => {
    const input = props[surface]
    setRecovery(previous => ({ ...previous, [surface]: { input, state: 'loading' } }))
    const current = ++attempt.current[surface]
    void reload().then(() => { if (attempt.current[surface] === current) setRecovery(previous => ({ ...previous, [surface]: { input, state: 'ready' } })) })
  }, [reload, props.roster, props.conversation])
  const retryRoster = React.useCallback(() => recover('roster'), [recover])
  const retryThread = React.useCallback(() => recover('conversation'), [recover])

  const openAgent = React.useCallback((row: Row) => {
    setTabs(current => current.includes(row.ref) ? current : [...current, row.ref])
    setActiveKey(row.ref)
    if (window.matchMedia(narrowQuery).matches) setCollapsed(true)
  }, [])
  // The kit has no tool-output pane yet, so a call's "Open" shows the change set beside the thread, at the call's file.
  const openTool = React.useCallback((call: WorkLogCall) => {
    setChangesOpen(true)
    if (window.matchMedia(narrowQuery).matches) setActiveKey(changesKey)
    const path = call.changedPath ?? changedFiles.find(file => file.path === call.argsSummary)?.path
    if (path !== undefined) setRevealRequest(previous => ({ path, sequence: (previous?.sequence ?? 0) + 1 }))
  }, [changedFiles])
  const focusFilter = React.useCallback(() => { setCollapsed(false); requestAnimationFrame(() => searchRef.current?.focus()) }, [])

  // Global chords: Mod+K palette, Mod+B sidebar, "/" filter. Single keys never fire while typing.
  React.useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const mod = isMac ? event.metaKey : event.ctrlKey
      const target = event.target as HTMLElement | null
      const typing = target !== null && (target.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName))
      if (mod && event.key.toLowerCase() === 'k') { event.preventDefault(); setPaletteOpen(true) }
      else if (mod && event.key.toLowerCase() === 'b') { event.preventDefault(); setCollapsed(value => !value) }
      else if (event.key === '/' && !typing && !mod) { event.preventDefault(); focusFilter() }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [focusFilter])

  const roster: RosterState = rosterLoad === 'loading' ? 'loading' : rosterLoad === 'failed' ? 'error' : rosterLoad === 'stale' ? 'stale' : rosterFolders.length === 0 ? 'empty' : 'ready'
  const sync = threadSync(conversationPhase)
  const renderInlineReference = (path: string) => {
    const reviewer = rowFor(reviewerRef)
    if (path === reviewerCard.path && reviewer !== undefined) return <AgentResultCard tab={agentTab(reviewerRef, 'slice')} summary={reviewerCard.summary} onOpen={() => openAgent(reviewer)} />
    const resource = proseResources[path]
    const file = resource?.files[0]
    return resource === undefined || file === undefined ? undefined : <ResourceChipV1 resource={resource} file={file} />
  }

  // Narrow: one pane, so Changes folds in as a tab derived from changesOpen alone. Docked: a resizable panel beside the thread.
  const paneKeys = narrow && changesOpen ? [...tabs, changesKey] : tabs
  const shownKey = paneKeys.includes(activeKey) ? activeKey : paneKeys[0]
  const paneTabs = paneKeys.map(key => key === changesKey ? { key, title: 'Changes' } : agentTab(key, conversationPhase))
  const sidebarHidden = narrow || collapsed
  const showSidebar = <PaneHeaderButton label="Show sidebar" icon="chevron-right" shortcut="Meta+B Control+B" onPress={() => setCollapsed(false)} />

  // The panel header names the change set; the "Changes" surface tab beside it says what kind of view it is.
  const changes = (inPane: boolean) => <DiffPanel open title="feat/metric-track" width="100%"
    currentTurnFiles={changedFiles} branchFiles={branchFiles} revealRequest={revealRequest} onClose={inPane ? undefined : () => setChangesOpen(false)} />
  const thread = (key: string) => {
    const turns = [...(props.conversation.value.threads[key]?.turns ?? []), ...(sent[key] ?? [])]
    const view = threadView(conversationPhase, key, retryThread)
    const running = (props.conversation.value.threads[key]?.running ?? false) && view._tag === 'Live'
    const send = (text: string) => setSent(current => {
      const list = current[key] ?? []
      const id = `local-${key}-${list.length + 1}`
      const at = new Date(milestoneNow).toISOString()
      return { ...current, [key]: [...list, { id, prompt: { _tag: 'Text', id: `${id}-prompt`, role: 'user', text, attachments: [], streaming: false, at }, items: [], work: { calls: [], durationMs: undefined, running: false, failed: false, interrupted: false } }] }
    })
    return <MilestoneThread key={key} title={rowFor(key)?.title ?? key} view={view} now={milestoneNow}
      runtime={{ messages: turns.flatMap(turn => [...(turn.prompt === undefined ? [] : [turn.prompt]), ...turn.items]), isRunning: running, onNew: async message => send(message.content.flatMap(part => part.type === 'text' ? [part.text] : []).join('\n')) }}
      transcript={{ turns, history: props.conversation.value.threads[key]?.history, sync, now: milestoneNow, observedAt: props.conversation._tag === 'stale' ? props.conversation.observedAt : milestoneNow, renderInlineReference, onOpenTool: openTool, viewportKey: `${props.persistKey}.${key}` }}
      composer={{ running, folder: 'fractal-web', branch: 'feat/metric-track', onSend: send, onSteer: send, draftKey: `${props.persistKey}.${key}` }} />
  }

  const sidebar = <MilestoneSidebar state={roster} folders={rosterFolders} now={milestoneNow} observedAt={props.roster._tag === 'stale' ? props.roster.observedAt : undefined} activeRef={shownKey} query={query} onQueryChange={setQuery} searchRef={searchRef}
    inboxCount={inboxCount}
    onOpen={openAgent} onRetry={retryRoster} onOpenPalette={() => setPaletteOpen(true)} onCollapse={() => setCollapsed(true)} collapseLabel={narrow ? 'Close sidebar' : 'Hide sidebar'} modKey={modKey} />
  const commands: readonly PaletteCommand[] = [
    { id: 'toggle-sidebar', label: sidebarHidden ? 'Show sidebar' : 'Hide sidebar', shortcut: `${modKey}B`, run: () => setCollapsed(value => !value) },
    { id: 'filter', label: 'Filter agents', shortcut: '/', run: focusFilter },
    { id: 'changes', label: changesOpen ? 'Hide changes' : 'Show changes', run: () => setChangesOpen(value => !value) },
    { id: 'scheme', label: scheme === 'dark' ? 'Switch to light theme' : 'Switch to dark theme', run: () => setScheme(scheme === 'dark' ? 'light' : 'dark') },
  ]
  const agents = rosterFolders.flatMap(folder => folder.rows.flatMap(row => [{ row, folder: folder.label }, ...row.children.map(child => ({ row: child, folder: folder.label }))]))

  return <div data-milestone-state={phase} data-scheme={scheme} data-narrow={narrow || undefined} {...stylex.props(...baselineTheme, scheme === 'light' && lightTheme, styles.root, scheme === 'light' && styles.light)}>
    <ThemedPortal>
      {!narrow && !collapsed && <>
        <div style={{ width }} {...stylex.props(styles.sidebarSlot)}>{sidebar}</div>
        <ResizableSplit id={props.persistKey} value={width} min={geometryNumbers.sidebarMin} max={440} onChange={setWidth} onToggle={() => setCollapsed(true)} onReset={() => setWidth(sidebarDefault)} collapsed={false} label="Resize sidebar" />
      </>}
      {narrow && <ModalOverlay isDismissable isOpen={!collapsed} onOpenChange={open => setCollapsed(!open)} {...stylex.props(styles.scrim)}>
        <Modal {...stylex.props(styles.drawer)}><Dialog aria-label="Agents" {...stylex.props(styles.drawerDialog)}>{sidebar}</Dialog></Modal>
      </ModalOverlay>}
      <main aria-label="Workspace" {...stylex.props(styles.main)}>
        {props.conversation.value.sync === undefined
          ? phase === 'stale' && <ConnectionStatus state="reconnecting" text="Next try in 4s · attempt 2" />
          : sync._tag === 'Stale' && sync.reason._tag === 'Reconnecting'
            ? <ConnectionStatus state="reconnecting" text={`${sync.reason.issue} · Next try in ${Math.ceil(Math.max(0, sync.reason.nextAt - milestoneNow) / 1000)}s · attempt ${sync.reason.attempt}`} />
            : <SyncLine status={sync} label="conversation" now={milestoneNow} observedAt={props.conversation._tag === 'stale' ? props.conversation.observedAt : milestoneNow} inline />}
        {shownKey === undefined
          ? <div {...stylex.props(styles.emptyFrame)}>
            {sidebarHidden && <div {...stylex.props(styles.emptyBar)}>{showSidebar}</div>}
            <div data-milestone-empty-main {...stylex.props(styles.emptyMain)}>
              <p {...stylex.props(styles.emptyTitle)}>No agent open</p>
              <dl {...stylex.props(styles.hints)}>
                <dt {...stylex.props(styles.hintKey)}><kbd {...stylex.props(styles.kbd)}>{modKey}K</kbd></dt><dd {...stylex.props(styles.hintText)}>Open an agent or run a command</dd>
                {roster !== 'empty' && <><dt {...stylex.props(styles.hintKey)}><kbd {...stylex.props(styles.kbd)}>/</kbd></dt><dd {...stylex.props(styles.hintText)}>Filter agents</dd></>}
                <dt {...stylex.props(styles.hintKey)}><kbd {...stylex.props(styles.kbd)}>{modKey}B</kbd></dt><dd {...stylex.props(styles.hintText)}>Show or hide the sidebar</dd>
              </dl>
            </div>
          </div>
          // Docked, Changes sits beside the thread in a kit DockedSplit; the thread keeps its place when Changes toggles.
          : <DockedSplit persistKey={`${props.persistKey}.changes`} label="Resize changes" onCollapse={() => setChangesOpen(false)} dock={!narrow && changesOpen ? changes(false) : undefined}>
            <Pane tabs={paneTabs.map(tab => ({ key: tab.key, title: tab.title, glyph: tab.status === undefined ? undefined : <PaneGlyph tab={tab} /> }))} activeKey={shownKey} onSelectTab={setActiveKey}
              leading={sidebarHidden ? showSidebar : undefined} actions={narrow ? undefined : <PaneHeaderToggle label="Show changes" icon="panel" isSelected={changesOpen} onChange={setChangesOpen} />}>
              {shownKey === changesKey ? changes(true) : thread(shownKey)}
            </Pane>
          </DockedSplit>}
      </main>
      <MilestonePalette isOpen={paletteOpen} onOpenChange={setPaletteOpen} agents={agents} commands={commands} frozen={roster === 'stale'} onOpenAgent={openAgent} />
    </ThemedPortal>
  </div>
}

/**
 * Overlays (palette, drawer, hover card) portal into a host inside the themed root so they inherit the scheme. The host is
 * state, not a ref, so an overlay that is open on the first frame re-renders into it once it exists.
 */
function ThemedPortal({ children }: { readonly children: React.ReactNode }) {
  const [host, setHost] = React.useState<HTMLDivElement | null>(null)
  const getContainer = React.useCallback(() => host, [host])
  return <UNSAFE_PortalProvider getContainer={getContainer}>{children}<div ref={setHost} {...stylex.props(styles.portalHost)} /></UNSAFE_PortalProvider>
}

const styles = stylex.create({
  root: { position: 'relative', display: 'flex', height: '100dvh', minWidth: 0, overflow: 'hidden', backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, colorScheme: 'dark' },
  light: { colorScheme: 'light' },
  sidebarSlot: { flexShrink: 0, minWidth: 0, height: '100%' },
  scrim: { position: 'fixed', inset: 0, zIndex: 30, display: 'flex', backgroundColor: surface.scrim },
  drawer: { width: `min(${sidebarDefault}px, calc(100vw - ${g.gutter40}))`, height: '100%', outline: 'none', boxShadow: elevation.dialog },
  drawerDialog: { height: '100%', outline: 'none' },
  main: { display: 'flex', flexDirection: 'column', flex: '1 1 0', minWidth: 0, minHeight: 0 },
  emptyFrame: { flex: '1 1 0', display: 'flex', flexDirection: 'column', minHeight: 0 },
  emptyBar: { display: 'flex', alignItems: 'center', height: g.controlLg, flexShrink: 0, paddingInline: s.sm },
  emptyMain: { flex: '1 1 0', display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', gap: s.lg, padding: s.xl, minHeight: 0 },
  emptyTitle: { margin: 0, color: ink.fgSoft, fontSize: t.uiSize, lineHeight: t.uiLeading, fontWeight: t.weightMedium },
  hints: { display: 'grid', gridTemplateColumns: 'max-content max-content', columnGap: s.lg, rowGap: s.sm, margin: 0, alignItems: 'center' },
  hintKey: { margin: 0, justifySelf: 'end' },
  hintText: { margin: 0, color: ink.fgMuted },
  kbd: { fontFamily: t.fontSans, fontSize: t.denseSize, lineHeight: t.metaLeading, color: ink.fgSoft, paddingInline: s.xs, borderRadius: r.keyboard, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong },
  portalHost: { position: 'fixed', top: 0, left: 0, width: 0, height: 0, zIndex: 30 },
})
