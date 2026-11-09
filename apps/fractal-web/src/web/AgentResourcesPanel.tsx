import * as React from 'react'
import * as Aria from 'react-aria-components'
import * as stylex from '@stylexjs/stylex'
import { useAtomValue } from '@effect/atom-react'
import * as Atom from 'effect/reactivity/Atom'
import { DateTime } from 'effect'
import type { ResourceObservation } from '@fractal/client/schema'
import type { DetailsResource, DetailsSection } from '@fractal/ui/assistant-ui'
import { colorVars as c, spaceVars as s, typeVars as t, geometryVars as g, interactionVars as interaction } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { useDataSource } from '../data/react.tsx'
import { credentialNotice, unavailable, type DataSource, type Feed } from '../data/source.ts'
import { LiveAgentTodos } from '../conversation/todos/AgentTodos.tsx'
import { agentTodosAtom } from '../conversation/todos/model.ts'
import type { ResourcePage } from '../data/agentResources/model.ts'
import { cancelMeasure, measureEnd } from './perfMarks.ts'
import type { OpenOperation } from '../telemetry/ux.ts'
import { useNativeDetails, type NativeDetailsObservation } from './useNativeDetails.ts'
import { observedResourceFace } from '../workspace/faces.ts'
import { AgentMissions } from './AgentMissions.tsx'
import { PanelSection, PanelThingRow, type PanelItem } from './ResourcePanelRows.tsx'

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
export const useResourcesCount = (agentRef: string): number => {
  const source = useDataSource()
  const resources = useAtomValue(retainedResourceAtoms(source).resources(agentRef))
  const todos = useAtomValue(agentTodosAtom({ source, agentRef }))
  return (resources._tag === 'Observed' ? resources.value.items.length : 0) + (todos._tag === 'Observed' ? todos.total - todos.completed : 0)
}

export const ResourceRow = ({ resource, onOpen }: { readonly resource: ResourceObservation; readonly onOpen: (ref: string, split: boolean) => void }) =>
  <li><PanelThingRow item={{ key: resource.id, ref: resource.id, observed: { _tag: 'Known', value: observedResourceFace(resource), observedAt: DateTime.toEpochMillis(resource.observed_at), freshness: 'live' } }} onOpen={onOpen} /></li>

const nativeItem = (resource: DetailsResource, native: NativeDetailsObservation): PanelItem => ({
  key: resource.uri,
  ...(/^(details:|session:)/.test(resource.uri) ? {} : { ref: resource.uri }),
  observed: { _tag: 'Known', value: resource.face ?? { kind: 'Resource', icon: 'info', title: resource.card.label, mark: { shape: 'none', word: '' }, summary: resource.card.detail }, ...(native.observedAt === undefined ? {} : { observedAt: native.observedAt }), freshness: native.stale ? 'stale' : 'live' },
})
const notices = (label: string, feed: Exclude<Feed<unknown>, { readonly _tag: 'Observed' }>): string =>
  credentialNotice(feed) ?? (feed._tag === 'Waiting' ? `Loading ${label.toLowerCase()}…`
    : feed.reason === 'ungranted' ? `Read access to ${label.toLowerCase()} has not been granted.`
      : feed.reason === 'unsupported' ? `This connection does not support ${label.toLowerCase()}.` : `${label} could not be loaded.`)

const groups = [
  { title: 'Terminals', kinds: ['terminal', 'terminal.session'] },
  { title: 'Subagents', kinds: ['subagent'] },
  { title: 'Pull requests', kinds: ['vcs.pull-request'] },
  { title: 'Issues', kinds: ['vcs.issue'] },
  { title: 'CI runs', kinds: ['ci.run'] },
  { title: 'Reviews', kinds: ['human.review'] },
  { title: 'Declared bindings', kinds: [] },
] as const

export const AgentResourcesView = ({ agentRef, onOpen, opening, cache, contributedSections = [] }: {
  readonly agentRef: string
  readonly onOpen: (ref: string, split: boolean) => void
  readonly opening?: OpenOperation | undefined
  readonly cache?: 'retained' | 'cold'
  readonly contributedSections?: readonly DetailsSection[]
}) => {
  const source = useDataSource()
  const atoms = retainedResourceAtoms(source)
  const resources = useAtomValue(atoms.resources(agentRef))
  const queue = useAtomValue(atoms.queue(agentRef))
  const todos = useAtomValue(agentTodosAtom({ source, agentRef }))
  const native = useNativeDetails({ agentRef })
  React.useLayoutEffect(() => {
    if (resources._tag !== 'Waiting') opening?.phase('data', { 'wf.ux.cache': cache ?? 'cold' })
    if (queue._tag !== 'Waiting') opening?.phase('queue')
    if (resources._tag !== 'Waiting' && queue._tag !== 'Waiting') return opening?.contentCommitted()
  }, [opening, cache, resources._tag, queue._tag])
  const settled = queue._tag === 'Observed' && resources._tag === 'Observed' && !resources.value.loadingMore
  React.useLayoutEffect(() => { if (settled) return measureEnd('resources_panel', agentRef) }, [agentRef, settled])
  React.useEffect(() => () => cancelMeasure('resources_panel'), [])
  const accessNotice = credentialNotice(resources) ?? credentialNotice(queue)
  const reported = resources._tag === 'Observed' && !credentialNotice(resources) ? resources.value.items : []
  const observed = (resource: ResourceObservation): PanelItem => ({ key: resource.id, ref: resource.id, observed: { _tag: 'Known', value: observedResourceFace(resource), observedAt: DateTime.toEpochMillis(resource.observed_at), freshness: resources._tag === 'Observed' ? resources.freshness : 'live' } })
  const nativeSections = new Map<string, DetailsSection>([...native.sections, ...contributedSections].map(section => [section.title, section]))
  const unreported = groups.flatMap(group => {
    const section = nativeSections.get(group.title)
    return section?.items._tag === 'Unknown' && section.pending !== true && section.failure === undefined && !reported.some(item => group.kinds.some(kind => kind === item.kind)) ? [group.title] : []
  })
  const knownKinds = new Set<string>(groups.flatMap(group => [...group.kinds]))
  const otherKinds = [...new Set(reported.filter(resource => !knownKinds.has(resource.kind)).map(resource => resource.kind))]
  return <div data-agent-resources {...stylex.props(styles.contents)}>
    {accessNotice ? <p role="status" {...stylex.props(styles.note)}>{accessNotice}</p> : resources._tag !== 'Observed' ? <p role="status" {...stylex.props(styles.note)}>{notices('Agent resources', resources)}</p> : resources.freshness === 'stale' ? <p role="status" {...stylex.props(styles.note)}>Showing the last verified resources while reconnecting.</p> : null}
    {todos._tag === 'Observed' && todos.total > 0 ? <section aria-label="To-dos"><h2 {...stylex.props(styles.sectionTitle)}>To-dos</h2><LiveAgentTodos agentRef={agentRef} placement="resources" /></section> : todos._tag !== 'Observed' ? <LiveAgentTodos agentRef={agentRef} placement="resources" /> : null}
    <AgentMissions agentRef={agentRef} onOpen={onOpen} section={nativeSections.get('Missions')} />
    {groups.map(group => {
      const section = nativeSections.get(group.title)
      const observations = reported.filter(resource => group.kinds.some(kind => kind === resource.kind))
      const ids = new Set<string>(observations.map(resource => resource.id))
      const items = [...observations.map(observed), ...(section?.items._tag === 'Known' ? section.items.value.filter(item => !ids.has(item.uri)).map(item => nativeItem(item, native)) : [])]
      return <PanelSection key={group.title} title={group.title} items={items} onOpen={onOpen} section={section} />
    })}
    {otherKinds.map(kind => {
      const entries = reported.filter(resource => resource.kind === kind)
      const first = entries[0]
      return first === undefined ? null : <PanelSection key={kind} title={observedResourceFace(first).kind} items={entries.map(observed)} onOpen={onOpen} />
    })}
    {unreported.length === 0 ? null : <p role="status" {...stylex.props(styles.note)}>Not reported: {unreported.join(', ')}.</p>}
    {resources._tag !== 'Observed' ? null : <>
      {resources.value.pagingError === undefined ? null : <p role="status" data-wf-resource-paging="failed" {...stylex.props(styles.note)}>More resources could not be loaded.</p>}
      {resources.value.nextCursor === null ? null : <Aria.Button isDisabled={resources.value.loadingMore === true} onPress={() => source.resources?.loadMore(agentRef)} {...stylex.props(styles.action)}>{resources.value.loadingMore ? 'Loading more resources…' : 'Load more resources'}</Aria.Button>}
    </>}
    {source.resources === undefined || accessNotice ? null : <Aria.Button onPress={() => source.resources?.refresh(agentRef)} {...stylex.props(styles.action)}>Refresh resources</Aria.Button>}
  </div>
}
const styles = stylex.create({
  contents: { position: 'relative', flex: '1 1 0', minHeight: 0, display: 'flex', flexDirection: 'column', gap: s.xl, padding: s.lg, overflowY: 'auto' },
  sectionTitle: { margin: 0, marginBottom: s.md, fontSize: t.metaSize, lineHeight: t.metaLeading, fontWeight: t.weightMedium, color: c.fgMuted },
  note: { margin: 0, fontSize: t.metaSize, lineHeight: t.metaLeading, color: c.fgSoft, overflowWrap: 'anywhere' },
  action: { alignSelf: 'flex-start', borderWidth: 0, backgroundColor: 'transparent', color: c.fgSoft, padding: s.xs, fontSize: t.metaSize, cursor: 'pointer', ':focus-visible': { outlineWidth: g.focusRing, outlineStyle: 'solid', outlineColor: interaction.focusRing } },
})
