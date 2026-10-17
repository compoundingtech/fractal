// Workshop-only composition of the shipped kit (SidebarAgentRow, Workbench, Transcript, WF-1, RC-3) with the milestone's new shell states.
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Button, Disclosure, DisclosurePanel } from 'react-aria-components'
import { Workbench } from '../../assistant-ui/workbench/Workbench'
import { group, layoutPaneKeys, parsePaneKey, split, type ThreadResource, type WorkbenchLayout, type WorkbenchPane, type WorkbenchResources } from '../../assistant-ui/workbench/workbench-model'
import type { WorkbenchPaneDetails } from '../../assistant-ui/workbench/workbench-appearance'
import { decidedAppearance } from '../../assistant-ui/workbench/workbench-fixtures'
import { terminalFixtureFrame } from '../../assistant-ui/workbench/terminal-fixtures'
import { ResizableSplit } from '../../assistant-ui/composition/Shell'
import { Icon } from '../../assistant-ui/composition/Icons'
import { ResourceChipV1 } from '../../assistant-ui/resources/ResourceCardV1'
import type { TranscriptTurn } from '../../assistant-ui/composition/Transcript'
import type { DiffRevealRequest } from '../../assistant-ui/composition/DiffPanel'
import type { WorkLogCall } from '../../assistant-ui/taste/work-log'
import type { SyncStatus } from '../../assistant-ui/st3-views/sync-status'
import type { SidebarAgentRow as Row } from '../../assistant-ui/sidebar/model'
import type { DiscRefinement } from '../../assistant-ui/sidebar/SidebarStatus'
import { baselineTheme } from '../../assistant-ui/neutral-theme'
import { lightTheme, type Scheme } from '../../assistant-ui/composition-theme'
import { accentVars as accent, borderVars as border, geometryNumbers, geometryVars as g, radiusVars as r, spaceVars as s, statusVars as tone, surfaceVars as surface, textVars as ink, typeVars as t } from '../../assistant-ui/composition-tokens.stylex'
import { MilestoneSidebar, type RosterState } from './MilestoneSidebar'
import { MilestonePalette, type PaletteCommand } from './MilestonePalette'
import { branchFiles, changedFiles, inboxCount, mainAgentRef, mainTurns, milestoneNow, proseResources, reviewerRef, reviewerTurns, rosterFolders } from './milestone-fixtures'

export type MilestoneState = 'slice' | 'loading' | 'empty-roster' | 'empty-thread' | 'no-match' | 'error' | 'stale' | 'navigation'
export interface RedesignMilestoneProps {
  readonly scheme: Scheme
  readonly state: MilestoneState
  readonly rowAdaptive: 'off' | '300' | '260'
  readonly rosterLoading: 'skeleton' | 'line'
  readonly errorPlacement: 'sidebar' | 'banner'
  readonly emptyMain: 'hints' | 'line'
  readonly paletteScope: 'mixed' | 'agents'
  readonly glyphRefinement: DiscRefinement
  readonly sectionNav: 'footer' | 'header'
  /** Device-local key for persisted sidebar width; stories pass a per-story key so screenshots stay deterministic. */
  readonly persistKey: string
}

const mainUri = `agent:${mainAgentRef.slice('agent/'.length)}`
const reviewerUri = `agent:${reviewerRef.slice('agent/'.length)}`
const diffUri = 'diff:src/sidebar/row-fit.ts'
const sliceLayout: WorkbenchLayout = split('right', group([{ uri: mainUri }, { uri: reviewerUri }]), group([{ uri: diffUri, form: 'unified' }]), 0.62)
const singleLayout: WorkbenchLayout = group([{ uri: mainUri }])
const rosterState: Record<MilestoneState, RosterState> = { slice: 'ready', loading: 'loading', 'empty-roster': 'empty', 'empty-thread': 'ready', 'no-match': 'ready', error: 'error', stale: 'stale', navigation: 'ready' }
const isMac = typeof navigator !== 'undefined' && /Mac|iPhone|iPad/.test(navigator.platform)
const modKey = isMac ? '⌘' : 'Ctrl+'

function threadSync(state: MilestoneState): SyncStatus {
  if (state === 'loading') return { _tag: 'Connecting', attempt: 1, since: milestoneNow - 1200 }
  if (state === 'stale') return { _tag: 'Stale', reason: { _tag: 'Reconnecting', attempt: 2, nextAt: milestoneNow + 4000, issue: 'Gateway connection interrupted' }, lastLiveAt: milestoneNow - 18_000 }
  return { _tag: 'Live', since: milestoneNow - 30_000 }
}

/** Story host: owns turns, sync, the composer's local send and every pane resource. */
function useMilestoneResources(state: MilestoneState, onOpenTool: (call: WorkLogCall) => void, onRetry: () => void) {
  const [sent, setSent] = React.useState<readonly TranscriptTurn[]>([])
  const resources = React.useMemo<WorkbenchResources>(() => {
    const send = (text: string) => setSent(current => {
      const id = `local-${current.length + 1}`
      const at = new Date(milestoneNow).toISOString()
      return [...current, { id, prompt: { _tag: 'Text', id: `${id}-prompt`, role: 'user', text, attachments: [], streaming: false, at }, items: [], work: { calls: [], durationMs: undefined, running: false, failed: false, interrupted: false } }]
    })
    const renderInlineReference = (path: string) => {
      const resource = proseResources[path]
      return resource === undefined ? undefined : <ResourceChipV1 resource={resource} file={resource.files[0]!} />
    }
    const noTurns = state === 'loading' || state === 'empty-thread'
    const thread = (turns: readonly TranscriptTurn[], running: boolean): ThreadResource => {
      const shown = noTurns ? sent : [...turns, ...sent]
      return {
        runtime: { messages: shown.flatMap(turn => [...(turn.prompt === undefined ? [] : [turn.prompt]), ...turn.items]), isRunning: running && !noTurns && state !== 'error', onNew: async message => send(message.content.flatMap(part => part.type === 'text' ? [part.text] : []).join('\n')) },
        transcript: {
          turns: shown, sync: threadSync(state), now: milestoneNow, observedAt: milestoneNow, renderInlineReference, onOpenTool,
          ...(state === 'error' ? { availability: { _tag: 'Unavailable' as const, reason: 'Conversation unavailable', detail: 'The gateway could not load this conversation (HTTP 503). Your draft is unchanged.', action: { label: 'Try again', onPress: onRetry } } } : {}),
        },
      }
    }
    const controls = { folder: 'fractal-web', branch: 'feat/metric-track', onSend: send, onSteer: send }
    return {
      threads: new Map([[mainUri, thread(mainTurns, true)], [reviewerUri, thread(reviewerTurns, false)]]),
      threadControls: new Map([[mainUri, controls], [reviewerUri, controls]]),
      diffs: new Map([[diffUri, { lines: changedFiles[0]!.diff, path: changedFiles[0]!.path, added: 2, removed: 1, currentTurnFiles: changedFiles, branchFiles }]]),
      terminals: terminalFixtureFrame,
    }
  }, [state, sent, onOpenTool, onRetry])
  const describePane = React.useCallback((pane: WorkbenchPane): WorkbenchPaneDetails => {
    if (pane.uri === diffUri) return { title: 'Changes' }
    if (pane.uri === reviewerUri) return { title: 'Review metric track diff', breadcrumb: 'metric-track', status: 'Done', statusTone: 'done' }
    const status = state === 'loading' ? 'Connecting' : state === 'error' ? 'Unavailable' : state === 'stale' ? 'Reconnecting' : state === 'empty-thread' ? 'Idle' : 'Working'
    return { title: 'Align the sidebar metric track', breadcrumb: 'fractal-web', status, statusTone: status === 'Working' ? 'running' : status === 'Unavailable' ? 'danger' : status === 'Reconnecting' ? 'attention' : 'neutral' }
  }, [state])
  return { resources, describePane }
}

/** Opens at 304px so the approved two-line row (with its metrics line) is what a first frame shows; RM1-Q1 decides where it folds to one line. */
const sidebarDefault = 304
const narrowQuery = '(max-width: 639px)'
const subscribeNarrow = (notify: () => void) => {
  const media = window.matchMedia(narrowQuery)
  media.addEventListener('change', notify)
  return () => media.removeEventListener('change', notify)
}

export function RedesignMilestone(props: RedesignMilestoneProps) {
  const { scheme: initialScheme } = props
  const [state, setState] = React.useState(props.state)
  const retryTimer = React.useRef<number | undefined>(undefined)
  React.useEffect(() => () => window.clearTimeout(retryTimer.current), [])
  // A retry walks the real recovery path: the loading frame first, then the loaded slice.
  const retry = React.useCallback(() => {
    setState('loading')
    window.clearTimeout(retryTimer.current)
    retryTimer.current = window.setTimeout(() => setState('slice'), 900)
  }, [])
  const narrow = React.useSyncExternalStore(subscribeNarrow, () => window.matchMedia(narrowQuery).matches, () => false)
  const [scheme, setScheme] = React.useState(initialScheme)
  const [width, setWidth] = React.useState(() => {
    try { const stored = Number(window.localStorage.getItem(`composition.${props.persistKey}`)); return Number.isFinite(stored) && stored > 0 ? stored : sidebarDefault } catch { return sidebarDefault }
  })
  // Below 640px the sidebar is an overlay drawer: closed when a thread is on screen, open when the roster is the only content.
  const [collapsed, setCollapsed] = React.useState(() => window.matchMedia(narrowQuery).matches && state !== 'empty-roster' && state !== 'loading' && state !== 'error')
  const [query, setQuery] = React.useState(state === 'no-match' ? 'zz-nothing' : '')
  const [paletteOpen, setPaletteOpen] = React.useState(state === 'navigation')
  const [active, setActive] = React.useState<string | undefined>(state === 'empty-roster' ? undefined : mainAgentRef)
  const [layout, setLayout] = React.useState<WorkbenchLayout>(state === 'slice' || state === 'navigation' ? sliceLayout : singleLayout)
  const [focusedPane, setFocusedPane] = React.useState(mainUri)
  const searchRef = React.useRef<HTMLInputElement>(null)
  const [revealRequest, setRevealRequest] = React.useState<DiffRevealRequest>()
  // The kit has no tool-output pane yet, so a call's "Open" reveals the change set in a split, at the call's file when it touched one.
  const openTool = React.useCallback((call: WorkLogCall) => {
    setLayout(current => layoutPaneKeys(current).some(key => key.startsWith(diffUri)) ? current : sliceLayout)
    setFocusedPane(diffUri)
    const path = call.changedPath ?? changedFiles.find(file => file.path === call.argsSummary)?.path
    if (path !== undefined) setRevealRequest(previous => ({ path, sequence: (previous?.sequence ?? 0) + 1 }))
  }, [])
  const { resources, describePane } = useMilestoneResources(state, openTool, retry)
  const roster = rosterState[state]
  // Splits fold into one tab group when there is no room for two readable lanes.
  const shownLayout = narrow ? group(layoutPaneKeys(layout).map(parsePaneKey)) : layout
  React.useEffect(() => setScheme(initialScheme), [initialScheme])
  // Global chords: Mod+K palette, Mod+B sidebar, "/" filter. Single keys never fire while typing.
  React.useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const mod = isMac ? event.metaKey : event.ctrlKey
      const target = event.target as HTMLElement | null
      const typing = target !== null && (target.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName))
      if (mod && event.key.toLowerCase() === 'k') { event.preventDefault(); setPaletteOpen(true) }
      else if (mod && event.key.toLowerCase() === 'b') { event.preventDefault(); setCollapsed(value => !value) }
      else if (event.key === '/' && !typing && !mod) { event.preventDefault(); setCollapsed(false); requestAnimationFrame(() => searchRef.current?.focus()) }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [])
  const openAgent = (row: Row) => {
    setActive(row.ref)
    if (narrow) setCollapsed(true)
    if (row.ref === reviewerRef || row.parentRef === mainAgentRef) { setLayout(current => current === singleLayout ? group([{ uri: mainUri }, { uri: reviewerUri }]) : current); setFocusedPane(reviewerUri) }
    else setFocusedPane(mainUri)
  }
  const commands: readonly PaletteCommand[] = [
    { id: 'toggle-sidebar', label: collapsed ? 'Show sidebar' : 'Hide sidebar', shortcut: `${modKey}B`, run: () => setCollapsed(value => !value) },
    { id: 'filter', label: 'Filter agents', shortcut: '/', run: () => { setCollapsed(false); requestAnimationFrame(() => searchRef.current?.focus()) } },
    { id: 'split-changes', label: 'Open changes in a split', run: () => setLayout(sliceLayout) },
    { id: 'single-pane', label: 'Close splits', run: () => setLayout(singleLayout) },
    { id: 'scheme', label: scheme === 'dark' ? 'Switch to light theme' : 'Switch to dark theme', run: () => setScheme(scheme === 'dark' ? 'light' : 'dark') },
  ]
  const agents = rosterFolders.flatMap(folder => folder.rows.flatMap(row => [{ row, folder: folder.label }, ...row.children.map(child => ({ row: child, folder: folder.label }))]))
  const showWorkbench = active !== undefined && state !== 'empty-roster'
  const banner = state === 'error' && props.errorPlacement === 'banner' && <div role="alert" data-milestone-banner {...stylex.props(styles.banner)}>
    <Icon name="alert" size={14} /><span {...stylex.props(styles.bannerText)}><strong {...stylex.props(styles.bannerTitle)}>Couldn’t load agents.</strong> Open conversations and drafts are unchanged.</span>
    <Disclosure {...stylex.props(styles.bannerDisclosure)}><Button slot="trigger" {...stylex.props(styles.bannerLink)}>Details</Button><DisclosurePanel {...stylex.props(styles.bannerDetail)}>roster read failed · HTTP 503 · attempt 3 of 3</DisclosurePanel></Disclosure>
    <Button onPress={retry} {...stylex.props(styles.bannerButton)}>Retry</Button>
  </div>
  const sidebar = <MilestoneSidebar state={roster} folders={state === 'empty-roster' ? [] : rosterFolders} width={narrow ? Math.min(sidebarDefault, window.innerWidth - 48) : width} now={milestoneNow} activeRef={active} query={query} onQueryChange={setQuery} searchRef={searchRef}
    rowAdaptive={props.rowAdaptive} rosterLoading={props.rosterLoading} errorPlacement={props.errorPlacement} glyphRefinement={props.glyphRefinement} sectionNav={props.sectionNav} inboxCount={inboxCount}
    onOpen={openAgent} onRetry={retry} onOpenPalette={() => setPaletteOpen(true)} onCollapse={() => setCollapsed(true)} modKey={modKey} />
  const rail = <div {...stylex.props(styles.rail)}><Button aria-label="Show sidebar" aria-keyshortcuts="Meta+B Control+B" onPress={() => setCollapsed(false)} {...stylex.props(styles.railButton)}><Icon name="panel" size={14} /></Button></div>
  return <div data-milestone-state={state} data-scheme={scheme} data-narrow={narrow || undefined} {...stylex.props(styles.root, scheme === 'light' && styles.light, ...baselineTheme, scheme === 'light' && lightTheme)}>
    {narrow
      ? <>
        {rail}
        {!collapsed && <>
          <div aria-hidden onClick={() => setCollapsed(true)} {...stylex.props(styles.scrim)} />
          <div onKeyDown={event => { if (event.key === 'Escape' && (event.target as HTMLElement).tagName !== 'INPUT') setCollapsed(true) }} style={{ width: `min(${sidebarDefault}px, calc(100vw - 48px))` }} {...stylex.props(styles.drawer)}>{sidebar}</div>
        </>}
      </>
      : collapsed ? rail : <>
        <div style={{ width }} {...stylex.props(styles.sidebarSlot)}>{sidebar}</div>
        <ResizableSplit id={props.persistKey} value={width} min={geometryNumbers.sidebarMin} max={440} onChange={setWidth} onToggle={() => setCollapsed(true)} onReset={() => setWidth(sidebarDefault)} collapsed={false} label="Resize sidebar" />
      </>}
    <main aria-label="Workspace" {...stylex.props(styles.main)}>
      {banner}
      {showWorkbench
        ? <div {...stylex.props(styles.workbench)}><Workbench layout={shownLayout} resources={resources} workspaceId={`${props.persistKey}-${state}`} scheme={scheme} appearance={decidedAppearance} describePane={describePane} focusedPaneKey={focusedPane} onPaneSelect={setFocusedPane} onLayoutChange={next => { if (!narrow) setLayout(next) }} revealRequest={revealRequest} /></div>
        : <div data-milestone-empty-main {...stylex.props(styles.emptyMain)}>
          <p {...stylex.props(styles.emptyTitle)}>No agent open</p>
          {props.emptyMain === 'hints' && <dl {...stylex.props(styles.hints)}>
            <dt {...stylex.props(styles.hintKey)}><kbd {...stylex.props(styles.kbd)}>{modKey}K</kbd></dt><dd {...stylex.props(styles.hintText)}>Open an agent or run a command</dd>
            <dt {...stylex.props(styles.hintKey)}><kbd {...stylex.props(styles.kbd)}>/</kbd></dt><dd {...stylex.props(styles.hintText)}>Filter agents</dd>
            <dt {...stylex.props(styles.hintKey)}><kbd {...stylex.props(styles.kbd)}>{modKey}B</kbd></dt><dd {...stylex.props(styles.hintText)}>Show or hide the sidebar</dd>
          </dl>}
        </div>}
    </main>
    <MilestonePalette isOpen={paletteOpen} onOpenChange={setPaletteOpen} scope={props.paletteScope} agents={agents} commands={commands} glyphRefinement={props.glyphRefinement} onOpenAgent={openAgent} />
  </div>
}

const focusRing = { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: accent.primary, outlineOffset: '-2px' } as const
const styles = stylex.create({
  root: { position: 'relative', display: 'flex', height: '100dvh', minWidth: 0, overflow: 'hidden', backgroundColor: surface.canvas, color: ink.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, colorScheme: 'dark' },
  light: { colorScheme: 'light' },
  sidebarSlot: { flexShrink: 0, minWidth: 0, height: '100%' },
  scrim: { position: 'absolute', inset: 0, zIndex: 20, backgroundColor: 'rgb(0 0 0 / 0.4)' },
  drawer: { position: 'absolute', insetBlock: 0, insetInlineStart: 0, zIndex: 21, minWidth: 0, boxShadow: '0 0 24px rgb(0 0 0 / 0.35)' },
  rail: { display: 'flex', flexDirection: 'column', alignItems: 'center', width: g.gutter40, flexShrink: 0, paddingBlockStart: s.xs, backgroundColor: surface.sidebar, borderInlineEndWidth: g.hairline, borderInlineEndStyle: 'solid', borderInlineEndColor: border.border },
  railButton: { display: 'inline-flex', alignItems: 'center', justifyContent: 'center', width: g.controlSm, height: g.controlSm, padding: 0, borderWidth: 0, borderRadius: r.sm, backgroundColor: surface.transparent, color: ink.sidebarFgMuted, cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover, color: ink.fg }, ':focus-visible': focusRing },
  main: { display: 'flex', flexDirection: 'column', flex: '1 1 0', minWidth: 0, minHeight: 0 },
  workbench: { display: 'flex', flex: '1 1 0', minWidth: 0, minHeight: 0 },
  banner: { display: 'flex', alignItems: 'center', gap: s.md, minHeight: g.controlLg, flexShrink: 0, paddingInline: s.lg, color: tone.dangerFg, backgroundColor: tone.diffRemovedWash, borderBottomWidth: g.hairline, borderBottomStyle: 'solid', borderBottomColor: border.border },
  bannerText: { flex: '1 1 0', minWidth: 0, color: ink.fgSoft, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  bannerTitle: { color: tone.dangerFg, fontWeight: t.weightMedium },
  bannerDisclosure: { display: 'flex', alignItems: 'center', gap: s.md, minWidth: 0 },
  bannerDetail: { color: ink.fgMuted, fontFamily: t.fontMono, fontSize: t.denseSize, whiteSpace: 'nowrap' },
  bannerLink: { height: g.controlSm, padding: 0, borderWidth: 0, backgroundColor: surface.transparent, color: ink.fgMuted, fontFamily: t.fontSans, fontSize: t.denseSize, cursor: 'pointer', ':hover': { color: ink.fg }, ':focus-visible': focusRing },
  bannerButton: { height: g.controlSm, paddingInline: s.md, borderRadius: r.sm, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong, backgroundColor: surface.controlFill, color: ink.fg, fontFamily: t.fontSans, fontSize: t.denseSize, cursor: 'pointer', ':hover': { backgroundColor: surface.rowHover }, ':focus-visible': focusRing },
  emptyMain: { flex: '1 1 0', display: 'flex', flexDirection: 'column', alignItems: 'center', justifyContent: 'center', gap: s.lg, padding: s.xl, minHeight: 0 },
  emptyTitle: { margin: 0, color: ink.fgSoft, fontSize: t.uiSize, lineHeight: t.uiLeading, fontWeight: t.weightMedium },
  hints: { display: 'grid', gridTemplateColumns: 'max-content max-content', columnGap: s.lg, rowGap: s.sm, margin: 0, alignItems: 'center' },
  hintKey: { margin: 0, justifySelf: 'end' },
  hintText: { margin: 0, color: ink.fgMuted },
  kbd: { fontFamily: t.fontSans, fontSize: t.denseSize, lineHeight: t.metaLeading, color: ink.fgSoft, paddingInline: s.xs, borderRadius: r.keyboard, borderWidth: g.hairline, borderStyle: 'solid', borderColor: border.borderStrong },
})
