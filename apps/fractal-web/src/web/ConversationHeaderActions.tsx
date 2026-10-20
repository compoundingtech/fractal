import * as React from 'react'
import { cancelMeasure, markStart, measureEnd } from './perfMarks.ts'
import * as Aria from 'react-aria-components'
import * as stylex from '@stylexjs/stylex'
import { useAtom, useAtomValue } from '@effect/atom-react'
import * as Atom from 'effect/reactivity/Atom'
import { DateTime } from 'effect'
import { ChromeControl } from '@smalltalk/fractal-ui/assistant-ui/shell'
import { Icon } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Icons.tsx'
import { Tooltip } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Controls.tsx'
import { NarrowLayoutContext } from '../../../../packages/fractal-ui/src/assistant-ui/composition/NarrowLayout.tsx'
import { SyncFailureDetails } from '../../../../packages/fractal-ui/src/assistant-ui/st3-views/SyncLine.tsx'
import { colorVars as c, typeVars as t, spaceVars as s, geometryVars as g } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { compactTime } from '../../../../packages/fractal-ui/src/assistant-ui/sidebar/model.ts'
import { useConversation, useConversationSync, useDataSource, useNow } from '../data/react.tsx'
import { unavailable, waiting, type Feed } from '../data/source.ts'
import { resourceState, resourceTitle, type ResourcePage } from '../resources/agent/model.ts'
import { systemEventsPreference } from './conversationPreferences.ts'
import { DeclarationDialog } from './DeclarationDialog.tsx'

const unsupportedResources = Atom.make<Feed<ResourcePage>>(unavailable({ reason: 'unsupported', detail: '' }))
const unopenedResources = Atom.make<Feed<ResourcePage>>(waiting)

/** Native conversation controls, retained from the former ConversationDetail header portal. */
export const ConversationHeaderActions = ({ agentRef }: { readonly agentRef: string }) => {
  const source = useDataSource()
  const narrow = React.useContext(NarrowLayoutContext)
  const [showSystemEvents, setShowSystemEvents] = useAtom(systemEventsPreference)
  const [resourcesOpen, setResourcesOpen] = React.useState(false)
  const [declarationOpen, setDeclarationOpen] = React.useState(false)
  const [syncDetailsOpen, setSyncDetailsOpen] = React.useState(false)
  const conversation = useConversation(agentRef)
  const sync = useConversationSync(agentRef)
  const retainedFailure = conversation._tag === 'Observed' && conversation.value.items.length > 0 && sync?.status._tag === 'Failed'
  const changeResourcesOpen = (open: boolean) => {
    if (open) markStart('resources_panel', { key: agentRef })
    else cancelMeasure('resources_panel')
    setResourcesOpen(open)
  }
  // A closed inspector has no read demand. Otherwise each switch starts discovery plus a
  // resource read and keeps its 15-second poll alive despite showing no resources.
  const resources = useAtomValue(source.resources === undefined ? unsupportedResources
    : resourcesOpen ? source.resources.byAgent(agentRef) : unopenedResources)
  const disabledReason = resources._tag === 'Unavailable'
    ? resources.reason === 'ungranted' ? 'Read access to agent resources has not been granted.'
      : resources.reason === 'unsupported' ? 'This connection does not support agent resources.' : undefined
    : undefined
  return <div {...stylex.props(styles.actions)}>
    <span {...stylex.props(styles.secondary)}>
      <ChromeControl icon="folder" label="Resources" onPress={() => changeResourcesOpen(true)} {...(disabledReason === undefined ? {} : { disabledReason })} />
    </span>
      <Aria.ModalOverlay isOpen={resourcesOpen} onOpenChange={changeResourcesOpen} isDismissable {...stylex.props(styles.overlay)}>
        <Aria.Modal {...stylex.props(styles.modal)}>
          <Aria.Dialog aria-label="Agent resources" {...stylex.props(styles.dialog)}>
            <header {...stylex.props(styles.heading)}><strong>Resources</strong><Aria.Button aria-label="Close resources" onPress={() => changeResourcesOpen(false)} {...stylex.props(styles.button)}>Close resources</Aria.Button></header>
            <ResourceContents agentRef={agentRef} resources={resources} />
          </Aria.Dialog>
        </Aria.Modal>
      </Aria.ModalOverlay>
    <span {...stylex.props(styles.secondary)}>
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

const ResourceContents = ({ agentRef, resources }: {
  readonly agentRef: string
  readonly resources: Feed<ResourcePage>
}) => {
  const source = useDataSource()
  const queue = useAtomValue(source.subjectReads.agentQueue.feed(agentRef))
  const now = useNow()
  const settled = queue._tag === 'Observed' && resources._tag === 'Observed' && !resources.value.loadingMore
  React.useLayoutEffect(() => {
    if (settled) return measureEnd('resources_panel', agentRef)
  }, [agentRef, settled])
  return <div {...stylex.props(styles.contents)}>
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
          const observedAt = DateTime.formatIso(resource.observed_at)
          return <li key={resource.id} {...stylex.props(styles.resource)}>
            <div {...stylex.props(styles.resourceHeading)}><span {...stylex.props(styles.resourceTitle)}>{resourceTitle(resource)}</span>{state === undefined ? null : <span {...stylex.props(styles.resourceState)}>{state.replaceAll('_', ' ')}</span>}</div>
            <div {...stylex.props(styles.resourceMetadata)}><span>{resource.kind}</span><time dateTime={observedAt} title={observedAt} aria-label={`Resource observed ${observedAt}`}>{compactTime({ at: DateTime.toEpochMillis(resource.observed_at), now })} ago</time></div>
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
  declarationButton: { height: 28, minWidth: 28, justifyContent: 'center', paddingInline: { default: 8, '@container thread-header (max-width: 680px)': 0 } },
  declarationIcon: { display: { default: 'none', '@container thread-header (max-width: 680px)': 'inline-flex' } },
  declarationLabel: { display: { default: 'inline', '@container thread-header (max-width: 680px)': 'none' } },
  menuReason: { display: 'block', maxWidth: 240, color: c.fgMuted, fontSize: t.denseSize },
  button: { display: 'inline-flex', alignItems: 'center', gap: s.xs, whiteSpace: 'nowrap', flexShrink: 0, padding: '4px 8px', borderWidth: 0, borderRadius: 6, backgroundColor: 'transparent', color: c.fgMuted, fontSize: t.metaSize, cursor: 'pointer', ':hover': { backgroundColor: c.rowHover }, ':focus-visible': { outline: `2px solid ${c.primary}` }, ':disabled': { opacity: 0.5, cursor: 'default' } },
  overlay: { position: 'fixed', inset: 0, zIndex: 100, display: 'flex', justifyContent: 'flex-end', backgroundColor: 'rgba(0, 0, 0, 0.4)' },
  modal: { width: 380, maxWidth: 'calc(100vw - 24px)', height: '100%', backgroundColor: c.raised, color: c.fg, fontFamily: t.fontSans, fontSize: t.metaSize },
  dialog: { display: 'flex', flexDirection: 'column', height: '100%', outline: 'none' },
  heading: { display: 'flex', alignItems: 'center', justifyContent: 'space-between', padding: s.lg, borderBottom: `1px solid ${c.border}` },
  contents: { padding: s.lg, overflowY: 'auto' },
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
