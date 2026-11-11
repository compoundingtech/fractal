import * as React from 'react'
import * as Aria from 'react-aria-components'
import * as stylex from '@stylexjs/stylex'
import { useAtomValue } from '@effect/atom-react'
import * as Atom from 'effect/reactivity/Atom'
import { DateTime } from 'effect'
import type { ResourceObservation } from '@fractal/client/schema'
import { Icon, type IconName } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Icons.tsx'
import { NarrowLayoutContext } from '../../../../packages/fractal-ui/src/assistant-ui/composition/NarrowLayout.tsx'
import { HoverCard } from '../../../../packages/fractal-ui/src/assistant-ui/entity/HoverCard.tsx'
import { useEntityCard } from '../../../../packages/fractal-ui/src/assistant-ui/entity/EntityHoverCard.tsx'
import { ResourceEntityCard } from '../../../../packages/fractal-ui/src/assistant-ui/entity/ResourceCard.tsx'
import type { EntityStatusTone } from '../../../../packages/fractal-ui/src/assistant-ui/entity/entity.ts'
import { DetailsResources } from '../../../../packages/fractal-ui/src/assistant-ui/details/DetailsResources.tsx'
import type { DetailsSection } from '@fractal/ui/assistant-ui'
import { colorVars as c, geometryVars as g, interactionVars as interaction, radiusVars as r, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { HumanTime } from '@fractal/ui/human-time'
import { useDataSource, useNowSnapshot } from '../data/react.tsx'
import { credentialNotice, unavailable, type DataSource, type Feed } from '../data/source.ts'
import { LiveAgentTodos } from '../conversation/todos/AgentTodos.tsx'
import { agentTodosAtom } from '../conversation/todos/model.ts'
import { resourceState, resourceTitle, type ResourcePage } from '../resources/agent/model.ts'
import { cancelMeasure, measureEnd } from './perfMarks.ts'
import type { OpenOperation } from '../telemetry/ux.ts'
import { useNativeDetails } from './useNativeDetails.ts'

const unsupportedResources = Atom.make<Feed<ResourcePage>>(unavailable({ reason: 'unsupported', detail: '' }))

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

/**
 * The Resources tab's pill: observed resources plus unfinished to-dos. The panel holds this while it is open, in any
 * tab, so opening the panel is what starts the resource read; selecting an agent never does.
 */
export const useResourcesCount = (agentRef: string): number => {
  const source = useDataSource()
  const resources = useAtomValue(retainedResourceAtoms(source).resources(agentRef))
  const todos = useAtomValue(agentTodosAtom({ source, agentRef }))
  const observedCount = resources._tag === 'Observed' ? resources.value.items.length : 0
  const unfinished = todos._tag === 'Observed' ? todos.total - todos.completed : 0
  return observedCount + unfinished
}

const kinds: Readonly<Record<string, { readonly icon: IconName; readonly label: string }>> = {
  'vcs.pull-request': { icon: 'pull-request', label: 'Pull request' },
  'vcs.issue': { icon: 'alert', label: 'Issue' },
  'vcs.commit': { icon: 'dot', label: 'Commit' },
  'vcs.ref': { icon: 'swap', label: 'Branch' },
  'vcs.repository': { icon: 'folder', label: 'Repository' },
  'ci.run': { icon: 'check', label: 'CI run' },
  'filesystem.file': { icon: 'pencil', label: 'File' },
  'human.review': { icon: 'check', label: 'Review' },
  'uri.reference': { icon: 'globe', label: 'Link' },
  'harness.session-file': { icon: 'message', label: 'Session file' },
  arrangement: { icon: 'checklist', label: 'Arrangement' },
}

/** A kind names itself in words; one this client has no word for is a neutral "Resource", never its raw type. */
export const resourceKind = (kind: string): { readonly icon: IconName; readonly label: string } => kinds[kind] ?? { icon: 'info', label: 'Resource' }

/** Reported title, or the kind and the last part of its id when nothing better was reported. */
const humanTitle = (resource: ResourceObservation): string => {
  const title = resourceTitle(resource)
  return title !== resource.id ? title : `${resourceKind(resource.kind).label} ${resource.id.split('/').at(-1)}`
}

/** Click opens through the panel's host; a Cmd-click asks for the background once the tab shell's `useOpenable` replaces this. */
/** Failure words draw the kit's red status; any other reported state stays a neutral word. */
const stateTone = (state: string): EntityStatusTone => /fail|error|cancel/i.test(state) ? 'failed' : /success|merged|closed|done|complete/i.test(state) ? 'done' : 'neutral'

/** The resource's own card, when no plugin source knows it better: kind, state, when it was observed and its link. Raw refs stay out, as on the row. */
const ObservedResourceCard = ({ resource }: { readonly resource: ResourceObservation }) => {
  const kind = resourceKind(resource.kind)
  const state = resourceState(resource)
  const url = resource.facts.url
  return <ResourceEntityCard id={resource.id} title={humanTitle(resource)} kind={kind.label} icon={kind.icon}
    {...(state === undefined ? {} : { status: { tone: stateTone(state), label: state.replaceAll('_', ' ') } })}
    meta={[
      { icon: 'clock', label: 'Observed', value: <HumanTime at={DateTime.toEpochMillis(resource.observed_at)} kind="relative" /> },
      ...(typeof url === 'string' && url.length > 0 ? [{ icon: 'globe' as const, label: 'Link', value: url }] : []),
    ]} />
}

export const ResourceRow = ({ resource, onOpen }: { readonly resource: ResourceObservation; readonly onOpen: (ref: string, background: boolean) => void }) => {
  const narrow = React.useContext(NarrowLayoutContext)
  const kind = resourceKind(resource.kind)
  const state = resourceState(resource)
  // A plugin's card (a GitHub PR) when a source knows this ref, so its row, tab and mention agree; else the observed facts.
  const known = useEntityCard({ kind: 'resource', id: resource.id })
  return <li>
    <HoverCard placement="left top" content={() => known ?? <ObservedResourceCard resource={resource} />}>
      <button type="button" data-resource-row onClick={event => onOpen(resource.id, event.metaKey || event.ctrlKey)} {...stylex.props(styles.row, narrow && styles.rowTouch)}>
        <span {...stylex.props(styles.kind)}><Icon name={kind.icon} label={kind.label} size={14} /></span>
        <span {...stylex.props(styles.title)}>{humanTitle(resource)}</span>
        {state === undefined ? null : <span {...stylex.props(styles.meta)}>{state.replaceAll('_', ' ')}</span>}
        <span {...stylex.props(styles.meta)}><HumanTime at={DateTime.toEpochMillis(resource.observed_at)} kind="relative" aria-label="Resource observed" /></span>
      </button>
    </HoverCard>
  </li>
}

const notices = (label: string, feed: Exclude<Feed<unknown>, { readonly _tag: 'Observed' }>): string =>
  credentialNotice(feed) ?? (feed._tag === 'Waiting' ? `Loading ${label.toLowerCase()}…`
    : feed.reason === 'ungranted' ? `Read access to ${label.toLowerCase()} has not been granted.`
      : feed.reason === 'unsupported' ? `This connection does not support ${label.toLowerCase()}.`
        : `${label} could not be loaded; reconnect and try again.`)

const ReadNotice = ({ feed, label }: { readonly feed: Exclude<Feed<unknown>, { readonly _tag: 'Observed' }>; readonly label: string }) =>
  <p role="status" data-wf-read-reason={feed._tag === 'Unavailable' ? feed.reason : undefined} {...stylex.props(styles.note)}>{notices(label, feed)}</p>

/** Sections the agent reports that belong with its resources rather than its facts. */
const movedSections: readonly DetailsSection['title'][] = ['Reviews', 'Declared bindings']

const ReportedSections = ({ agentRef, onOpen }: { readonly agentRef: string; readonly onOpen: (ref: string, background: boolean) => void }) => {
  const native = useNativeDetails({ agentRef })
  const now = useNowSnapshot()
  return <DetailsResources state={native.state} sections={native.sections.filter(section => movedSections.includes(section.title))} trace={native.trace} now={now} form="R2" empty="E3" onOpen={onOpen} />
}

export const AgentResourcesView = ({ agentRef, onOpen, opening, cache }: {
  readonly agentRef: string
  readonly onOpen: (ref: string, background: boolean) => void
  readonly opening?: OpenOperation | undefined
  readonly cache?: 'retained' | 'cold'
}) => {
  const source = useDataSource()
  const atoms = retainedResourceAtoms(source)
  const resources = useAtomValue(atoms.resources(agentRef))
  const queue = useAtomValue(atoms.queue(agentRef))
  React.useLayoutEffect(() => {
    if (resources._tag !== 'Waiting') opening?.phase('data', { 'wf.ux.cache': cache ?? 'cold' })
    if (queue._tag !== 'Waiting') opening?.phase('queue')
    if (resources._tag !== 'Waiting' && queue._tag !== 'Waiting') return opening?.contentCommitted()
  }, [opening, cache, resources._tag, queue._tag])
  const settled = queue._tag === 'Observed' && resources._tag === 'Observed' && !resources.value.loadingMore
  React.useLayoutEffect(() => {
    if (settled) return measureEnd('resources_panel', agentRef)
  }, [agentRef, settled])
  React.useEffect(() => () => cancelMeasure('resources_panel'), [])
  return <div data-agent-resources {...stylex.props(styles.contents)}>
    <section aria-label="To-dos">
      <h2 {...stylex.props(styles.sectionTitle)}>To-dos</h2>
      <LiveAgentTodos agentRef={agentRef} placement="resources" />
    </section>
    <section aria-label="Agent queue">
      <h2 {...stylex.props(styles.sectionTitle)}>Agent queue</h2>
      {queue._tag !== 'Observed' ? <ReadNotice feed={queue} label="Agent queue" /> : credentialNotice(queue) ? <p role="status" {...stylex.props(styles.note)}>{credentialNotice(queue)}</p> : <>
        {queue.freshness === 'stale' ? <p role="status" {...stylex.props(styles.note)}>Showing the last verified agent queue while reconnecting.</p> : null}
        <p {...stylex.props(styles.note)}>{queue.value.current_work_ids.length} current work {queue.value.current_work_ids.length === 1 ? 'item' : 'items'}</p>
        {queue.value.current_work_ids.length === 0 ? null : <ul {...stylex.props(styles.plain)}>{queue.value.current_work_ids.map(id => <li key={id} {...stylex.props(styles.note)}>{id}</li>)}</ul>}
        {queue.value.runs.length === 0 ? <p {...stylex.props(styles.note)}>No queued mission runs observed.</p> : <ol {...stylex.props(styles.plain)}>{queue.value.runs.map(run => <li key={run.mission_run_id} {...stylex.props(styles.note)}>{run.mission_run_id} · {run.state} · {run.ready_work_ids.length} ready, {run.waiting_work_ids.length} waiting</li>)}</ol>}
      </>}
    </section>
    <section aria-label="Observed resources">
      <h2 {...stylex.props(styles.sectionTitle)}>Observed resources</h2>
      {resources._tag !== 'Observed' ? <ReadNotice feed={resources} label="Agent resources" /> : credentialNotice(resources) ? <p role="status" {...stylex.props(styles.note)}>{credentialNotice(resources)}</p> : <>
        {resources.freshness === 'stale' ? <p role="status" {...stylex.props(styles.note)}>Showing the last verified resources while reconnecting.</p> : null}
        {resources.value.items.length === 0 ? <p {...stylex.props(styles.note)}>No resources observed for this agent.</p> : <ul {...stylex.props(styles.plain)}>{resources.value.items.map(resource => <ResourceRow key={resource.id} resource={resource} onOpen={onOpen} />)}</ul>}
        {resources.value.pagingError === undefined ? null : <p role="status" data-wf-resource-paging="failed" {...stylex.props(styles.note)}>More resources could not be loaded; refresh to try again.</p>}
        {resources.value.nextCursor === null ? null : <Aria.Button isDisabled={resources.value.loadingMore === true} onPress={() => source.resources?.loadMore(agentRef)} {...stylex.props(styles.action)}>{resources.value.loadingMore ? 'Loading more resources…' : 'Load more resources'}</Aria.Button>}
      </>}
      {source.resources === undefined || credentialNotice(resources) || credentialNotice(queue) ? null : <Aria.Button onPress={() => source.resources?.refresh(agentRef)} {...stylex.props(styles.action)}>Refresh resources</Aria.Button>}
    </section>
    <ReportedSections agentRef={agentRef} onOpen={onOpen} />
  </div>
}

const styles = stylex.create({
  contents: { flex: '1 1 0', minHeight: 0, display: 'flex', flexDirection: 'column', gap: s.xl, padding: s.lg, overflowY: 'auto' },
  sectionTitle: { margin: 0, marginBottom: s.md, fontSize: t.metaSize, lineHeight: t.metaLeading, fontWeight: t.weightMedium, color: c.fgMuted },
  note: { margin: 0, fontSize: t.metaSize, lineHeight: t.metaLeading, color: c.fgSoft, overflowWrap: 'anywhere' },
  plain: { listStyle: 'none', margin: 0, padding: 0, display: 'flex', flexDirection: 'column', gap: s.xs2 },
  row: { display: 'flex', alignItems: 'center', gap: s.md, width: `calc(100% + 2 * ${s.xs})`, minHeight: g.controlSm, boxSizing: 'border-box', marginInline: `calc(-1 * ${s.xs})`, paddingInline: s.xs, paddingBlock: s.xs, borderWidth: 0, borderRadius: r.sm, backgroundColor: 'transparent', color: c.fg, fontFamily: t.fontSans, fontSize: t.metaSize, lineHeight: t.metaLeading, textAlign: 'start', cursor: 'pointer', outline: 'none', ':hover': { backgroundColor: c.rowHover }, ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing, outlineOffset: '-2px' } },
  rowTouch: { minHeight: g.touchTarget },
  kind: { display: 'inline-flex', flexShrink: 0, color: c.fgMuted },
  title: { flexGrow: 1, minWidth: 0, overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap' },
  meta: { flexShrink: 0, color: c.fgMuted, fontSize: t.denseSize, fontVariantNumeric: 'tabular-nums' },
  action: { alignSelf: 'flex-start', marginTop: s.md, paddingInline: s.sm, height: g.controlSm, borderWidth: g.hairline, borderStyle: 'solid', borderColor: c.border, borderRadius: r.control, backgroundColor: 'transparent', color: c.fgSoft, fontFamily: t.fontSans, fontSize: t.metaSize, cursor: 'pointer', ':hover': { backgroundColor: c.rowHover }, ':disabled': { opacity: 0.5, cursor: 'default' } },
})
