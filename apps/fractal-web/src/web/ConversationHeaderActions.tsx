import * as React from 'react'
import { createPortal } from 'react-dom'
import { cancelMeasure, markStart, measureEnd } from './perfMarks.ts'
import * as Aria from 'react-aria-components'
import * as stylex from '@stylexjs/stylex'
import { RegistryContext, useAtom, useAtomValue } from '@effect/atom-react'
import * as Atom from 'effect/reactivity/Atom'
import { DateTime } from 'effect'
import { SidePanel } from '../../../../packages/fractal-ui/src/assistant-ui/composition/SidePanel.tsx'
import { ChromeControl } from '@smalltalk/fractal-ui/assistant-ui/shell'
import { Icon } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Icons.tsx'
import { Tooltip } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Controls.tsx'
import { NarrowLayoutContext } from '../../../../packages/fractal-ui/src/assistant-ui/composition/NarrowLayout.tsx'
import { SyncFailureDetails } from '../../../../packages/fractal-ui/src/assistant-ui/st3-views/SyncLine.tsx'
import { colorVars as c, typeVars as t, spaceVars as s, geometryVars as g } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { HumanTime } from '@smalltalk/fractal-ui/human-time'
import { useConversation, useConversationSync, useDataSource } from '../data/react.tsx'
import { unavailable, waiting, type DataSource, type Feed } from '../data/source.ts'
import { LiveAgentTodos } from '../conversation/todos/AgentTodos.tsx'
import { resourceState, resourceTitle, type ResourcePage } from '../resources/agent/model.ts'
import { systemEventsPreference } from './conversationPreferences.ts'
import type { OpenOperation, UxTelemetry } from '../telemetry/ux.ts'
import { DeclarationDialog } from './DeclarationDialog.tsx'

const unsupportedResources = Atom.make<Feed<ResourcePage>>(unavailable({ reason: 'unsupported', detail: '' }))
const unopenedResources = Atom.make<Feed<ResourcePage>>(waiting)

interface ResourceAtoms {
  readonly resources: (ref: string) => Atom.Atom<Feed<ResourcePage>>
  readonly queue: DataSource['subjectReads']['agentQueue']['feed']
}
const resourceAtomsBySource = new WeakMap<DataSource, ResourceAtoms>()

/** Retain authoritative reads only after explicit intent, never merely selecting an agent. */
export const retainedResourceAtoms = (source: DataSource): ResourceAtoms => {
  let atoms = resourceAtomsBySource.get(source)
  if (atoms === undefined) {
    atoms = {
      resources: Atom.family((ref: string) => Atom.make(get => get(source.resources?.byAgent(ref) ?? unsupportedResources)).pipe(Atom.setIdleTTL('60 seconds'))),
      queue: Atom.family((ref: string) => Atom.make(get => get(source.subjectReads.agentQueue.feed(ref))).pipe(Atom.setIdleTTL('60 seconds'))),
    }
    resourceAtomsBySource.set(source, atoms)
  }
  return atoms
}

/** Conversation controls stay in the responsive header; the workspace supplies the desktop dock. */
export const ConversationHeaderActions = ({ agentRef, ux, resourcesDock }: {
  readonly agentRef: string
  readonly ux?: UxTelemetry
  readonly resourcesDock?: { readonly host: HTMLElement | null; readonly open: boolean; readonly onOpenChange: (open: boolean) => void }
}) => {
  const source = useDataSource()
  const narrow = React.useContext(NarrowLayoutContext)
  const registry = React.useContext(RegistryContext)
  const atoms = retainedResourceAtoms(source)
  const prefetch = () => {
    registry.mount(atoms.resources(agentRef))()
    registry.mount(atoms.queue(agentRef))()
  }
  const [showSystemEvents, setShowSystemEvents] = useAtom(systemEventsPreference)
  const [standaloneResourcesOpen, setStandaloneResourcesOpen] = React.useState(false)
  const resourcesOpen = resourcesDock?.open ?? standaloneResourcesOpen
  const [declarationOpen, setDeclarationOpen] = React.useState(false)
  const [syncDetailsOpen, setSyncDetailsOpen] = React.useState(false)
  const conversation = useConversation(agentRef)
  const sync = useConversationSync(agentRef)
  const retainedFailure = conversation._tag === 'Observed' && conversation.value.items.length > 0 && sync?.status._tag === 'Failed'
  const [opening, setOpening] = React.useState<OpenOperation>()
  const [cache, setCache] = React.useState<'retained' | 'cold'>('cold')
  // The header control and the narrow overflow item share this one transition, so reopening an
  // already open panel from the menu never restarts its open measurement.
  const changeResourcesOpen = (open: boolean) => {
    if (open === resourcesOpen) return
    if (open) {
      markStart('resources_panel', { key: agentRef })
      setCache(registry.get(atoms.resources(agentRef))._tag === 'Observed' ? 'retained' : 'cold')
      setOpening(ux?.beginOpen({ surface: 'resources' }))
    }
    else { cancelMeasure('resources_panel'); opening?.cancel(); setOpening(undefined) }
    if (resourcesDock === undefined) setStandaloneResourcesOpen(open)
    else resourcesDock.onOpenChange(open)
  }
  const shellCommit = React.useCallback((node: HTMLElement | null) => {
    if (node === null || opening === undefined) return
    opening.phase('shell')
    return () => { opening.cancel(); cancelMeasure('resources_panel') }
  }, [opening])
  const panelId = React.useId()
  /** Closing releases UI demand, but the intent-created wrapper retains its poll for 60 seconds. */
  const resources = useAtomValue(source.resources === undefined ? unsupportedResources
    : resourcesOpen ? atoms.resources(agentRef) : unopenedResources)
  const disabledReason = resources._tag === 'Unavailable'
    ? resources.reason === 'ungranted' ? 'Read access to agent resources has not been granted.'
      : resources.reason === 'unsupported' ? 'This connection does not support agent resources.' : undefined
    : undefined
  const panel = resourcesOpen ? <SidePanel ref={shellCommit} id={panelId} placement={resourcesDock === undefined ? 'overlay' : 'dock'} aria-label="Agent resources" heading="Resources" actions={<Aria.Button aria-label="Close resources" onPress={() => changeResourcesOpen(false)} {...stylex.props(styles.button, narrow && styles.touchItem)}>Close resources</Aria.Button>} onClose={() => changeResourcesOpen(false)}>
    <ResourceContents agentRef={agentRef} resources={resources} opening={opening} cache={cache} />
  </SidePanel> : null
  return <div {...stylex.props(styles.actions)}>
    <span {...stylex.props(styles.secondary, narrow && styles.secondaryHidden)}>
      <ChromeControl icon="folder" label="Resources" active={resourcesOpen} expanded={resourcesOpen} {...(resourcesOpen ? { controls: panelId } : {})} onPrefetch={prefetch} onPress={() => changeResourcesOpen(!resourcesOpen)} {...(disabledReason === undefined ? {} : { disabledReason })} />
    </span>
    {resourcesDock === undefined ? panel : resourcesDock.host === null ? null : createPortal(panel, resourcesDock.host)}
    <span {...stylex.props(styles.secondary, narrow && styles.secondaryHidden)}>
      <Tooltip label="View KDL">
        <Aria.Button aria-label="View KDL" onPress={() => setDeclarationOpen(true)} {...stylex.props(styles.button, styles.declarationButton)}>
          <span {...stylex.props(styles.declarationIcon)}><Icon name="checklist" /></span>
          <span {...stylex.props(styles.declarationLabel)}>View KDL</span>
        </Aria.Button>
      </Tooltip>
    </span>
    <DeclarationDialog subject={agentRef} isOpen={declarationOpen} onOpenChange={setDeclarationOpen} />
    <Aria.ModalOverlay isOpen={syncDetailsOpen && retainedFailure} onOpenChange={setSyncDetailsOpen} isDismissable {...stylex.props(styles.overlay)}>
      <Aria.Modal {...stylex.props(styles.modal)}>
        <Aria.Dialog aria-label="Sync details for conversation" {...stylex.props(styles.dialog)}>
          <header {...stylex.props(styles.heading)}><strong>Sync details</strong><Aria.Button aria-label="Close sync details" onPress={() => setSyncDetailsOpen(false)} {...stylex.props(styles.button, narrow && styles.touchItem)}>Close</Aria.Button></header>
          {sync !== undefined && <SyncFailureDetails status={sync.status} />}
        </Aria.Dialog>
      </Aria.Modal>
    </Aria.ModalOverlay>
    <Aria.MenuTrigger>
      <ChromeControl icon="gear" label="Conversation menu" />
      <Aria.Popover {...stylex.props(styles.popover)}>
        <Aria.Menu aria-label="Conversation settings" onAction={key => { if (key === 'resources') changeResourcesOpen(true); else if (key === 'declaration') setDeclarationOpen(true); else if (key === 'sync-details') setSyncDetailsOpen(true) }} {...stylex.props(styles.menu)}>
          <Aria.MenuSection selectionMode="multiple" selectedKeys={showSystemEvents ? ['system'] : []} onSelectionChange={keys => setShowSystemEvents(keys === 'all' || keys.has('system'))}>
            <Aria.MenuItem id="system" {...stylex.props(styles.menuItem, narrow && styles.touchItem)}>Show all system events</Aria.MenuItem>
          </Aria.MenuSection>
          <Aria.MenuSection selectionMode="none">
            <Aria.MenuItem id="resources" isDisabled={disabledReason !== undefined} textValue="Resources" {...stylex.props(styles.menuItem, narrow && styles.touchItem)}>Resources{disabledReason === undefined ? null : <span slot="description" {...stylex.props(styles.menuReason)}>{disabledReason}</span>}</Aria.MenuItem>
            <Aria.MenuItem id="declaration" {...stylex.props(styles.menuItem, narrow && styles.touchItem)}>View KDL</Aria.MenuItem>
            {retainedFailure ? <Aria.MenuItem id="sync-details" {...stylex.props(styles.menuItem, narrow && styles.touchItem)}>Sync details</Aria.MenuItem> : null}
          </Aria.MenuSection>
        </Aria.Menu>
      </Aria.Popover>
    </Aria.MenuTrigger>
  </div>
}

const ResourceContents = ({ agentRef, resources, opening, cache }: {
  readonly agentRef: string
  readonly resources: Feed<ResourcePage>
  readonly opening?: OpenOperation
  readonly cache: 'retained' | 'cold'
}) => {
  const source = useDataSource()
  const queue = useAtomValue(retainedResourceAtoms(source).queue(agentRef))
  React.useLayoutEffect(() => {
    if (resources._tag !== 'Waiting') opening?.phase('data', { 'wf.ux.cache': cache })
    if (queue._tag !== 'Waiting') opening?.phase('queue')
    if (resources._tag !== 'Waiting' && queue._tag !== 'Waiting') return opening?.contentCommitted()
  }, [opening, cache, resources._tag, queue._tag])
  const settled = queue._tag === 'Observed' && resources._tag === 'Observed' && !resources.value.loadingMore
  React.useLayoutEffect(() => {
    if (settled) return measureEnd('resources_panel', agentRef)
  }, [agentRef, settled])
  return <div {...stylex.props(styles.contents)}>
    <section aria-label="Todos">
      <h2 {...stylex.props(styles.sectionTitle)}>Todos</h2>
      <LiveAgentTodos agentRef={agentRef} placement="resources" />
    </section>
    <section aria-label="Agent queue">
      <h2 {...stylex.props(styles.sectionTitle)}>Agent queue</h2>
      {queue._tag !== 'Observed' ? <ReadNotice feed={queue} label="Agent queue" /> : <>
        {queue.freshness === 'stale' ? <p role="status">Showing the last verified agent queue while reconnecting.</p> : null}
        <p>{queue.value.current_work_ids.length} current work {queue.value.current_work_ids.length === 1 ? 'item' : 'items'}</p>
        {queue.value.current_work_ids.length === 0 ? null : <ul>{queue.value.current_work_ids.map(id => <li key={id}>{id}</li>)}</ul>}
        {queue.value.runs.length === 0 ? <p>No queued mission runs observed.</p> : <ol>{queue.value.runs.map(run => <li key={run.mission_run_id}>{run.mission_run_id} · {run.state} · {run.ready_work_ids.length} ready, {run.waiting_work_ids.length} waiting</li>)}</ol>}
      </>}
    </section>
    <section aria-label="Observed resources">
      <h2 {...stylex.props(styles.sectionTitle)}>Observed resources</h2>
      {resources._tag !== 'Observed' ? <ReadNotice feed={resources} label="Agent resources" /> : <>
        {resources.freshness === 'stale' ? <p role="status">Showing the last verified resources while reconnecting.</p> : null}
        {resources.value.items.length === 0 ? <p>No resources observed for this agent.</p> : <ul {...stylex.props(styles.list)}>{resources.value.items.map(resource => {
          const state = resourceState(resource)
          return <li key={resource.id} {...stylex.props(styles.resource)}>
            <div {...stylex.props(styles.resourceHeading)}><span {...stylex.props(styles.resourceTitle)}>{resourceTitle(resource)}</span>{state === undefined ? null : <span {...stylex.props(styles.resourceState)}>{state.replaceAll('_', ' ')}</span>}</div>
            <div {...stylex.props(styles.resourceMetadata)}><span>{resource.kind}</span><HumanTime at={DateTime.toEpochMillis(resource.observed_at)} kind="relative" aria-label="Resource observed" /></div>
          </li>
        })}</ul>}
        {resources.value.pagingError === undefined ? null : <p role="status" data-wf-resource-paging="failed">More resources could not be loaded; refresh to try again.</p>}
        {resources.value.nextCursor === null ? null : <Aria.Button isDisabled={resources.value.loadingMore === true} onPress={() => source.resources?.loadMore(agentRef)} {...stylex.props(styles.button)}>{resources.value.loadingMore ? 'Loading more resources…' : 'Load more resources'}</Aria.Button>}
      </>}
      {source.resources === undefined ? null : <Aria.Button onPress={() => source.resources?.refresh(agentRef)} {...stylex.props(styles.button)}>Refresh resources</Aria.Button>}
    </section>
  </div>
}

const ReadNotice = ({ feed, label }: { readonly feed: Exclude<Feed<unknown>, { readonly _tag: 'Observed' }>; readonly label: string }) =>
  <p role="status" data-wf-read-reason={feed._tag === 'Unavailable' ? feed.reason : undefined}>{feed._tag === 'Waiting' ? `Loading ${label.toLowerCase()}…` : feed.reason === 'ungranted' ? `Read access to ${label.toLowerCase()} has not been granted.` : feed.reason === 'unsupported' ? `This connection does not support ${label.toLowerCase()}.` : `${label} could not be loaded; reconnect and try again.`}</p>

const styles = stylex.create({
  actions: { display: 'flex', alignItems: 'center', gap: 4, flexShrink: 0 },
  secondary: { display: { default: 'inline-flex', '@container thread-header (max-width: 440px)': 'none' }, flexShrink: 0 },
  secondaryHidden: { display: 'none' },
  declarationButton: { height: 28, minWidth: 28, justifyContent: 'center', paddingInline: { default: 8, '@container thread-header (max-width: 680px)': 0 } },
  declarationIcon: { display: { default: 'none', '@container thread-header (max-width: 680px)': 'inline-flex' } },
  declarationLabel: { display: { default: 'inline', '@container thread-header (max-width: 680px)': 'none' } },
  menuReason: { display: 'block', maxWidth: 240, color: c.fgMuted, fontSize: t.denseSize },
  button: { display: 'inline-flex', alignItems: 'center', gap: s.xs, whiteSpace: 'nowrap', flexShrink: 0, padding: '4px 8px', borderWidth: 0, borderRadius: 6, backgroundColor: 'transparent', color: c.fgMuted, fontSize: t.metaSize, cursor: 'pointer', ':hover': { backgroundColor: c.rowHover }, ':focus-visible': { outline: `2px solid ${c.primary}` }, ':disabled': { opacity: 0.5, cursor: 'default' } },
  overlay: { position: 'fixed', inset: 0, zIndex: 100, display: 'flex', justifyContent: 'flex-end', backgroundColor: 'rgba(0, 0, 0, 0.4)' },
  modal: { width: 380, maxWidth: 'calc(100vw - 24px)', height: '100%', backgroundColor: c.raised, color: c.fg, fontFamily: t.fontSans, fontSize: t.metaSize },
  dialog: { display: 'flex', flexDirection: 'column', height: '100%', outline: 'none' },
  heading: { display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: s.lg, borderBottom: `1px solid ${c.border}` },
  contents: { flex: '1 1 0', minHeight: 0, padding: s.lg, overflowY: 'auto' },
  sectionTitle: { fontSize: t.uiSize, marginBlock: s.md },
  list: { listStyle: 'none', padding: 0 },
  resource: { padding: s.md, color: c.fg, border: `1px solid ${c.border}`, borderRadius: 6 },
  resourceHeading: { display: 'flex', alignItems: 'baseline', gap: s.md },
  resourceTitle: { flexGrow: 1, minWidth: 0, overflowWrap: 'anywhere' },
  resourceState: { flexShrink: 0, fontSize: t.metaSize, color: c.fgMuted },
  resourceMetadata: { display: 'flex', justifyContent: 'space-between', gap: s.md, fontSize: t.metaSize, color: c.fgMuted },
  popover: { backgroundColor: c.raised, color: c.fg, border: `1px solid ${c.border}`, borderRadius: 6, fontFamily: t.fontSans, fontSize: t.metaSize, zIndex: 100 },
  menu: { padding: s.xs, outline: 'none' },
  menuItem: { padding: s.md, outline: 'none', borderRadius: 4, cursor: 'pointer', ':focus': { backgroundColor: c.rowHover }, ':is([data-focus-visible])': { outlineWidth: 2, outlineStyle: 'solid', outlineColor: c.fgMuted, outlineOffset: -2 }, ':is([data-selected])': { color: c.primary } },
  touchItem: { minHeight: g.touchTarget, display: 'flex', flexDirection: 'column', justifyContent: 'center', boxSizing: 'border-box' },
})
