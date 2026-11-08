import { useAtomValue } from '@effect/atom-react'
import * as DateTime from 'effect/DateTime'
import { EntityCard, EntityCardHeader, EntityCardList, EntityCardListItem, EntityCardMeta, EntityCardMetaLine, EntityCardNote, EntityCardSection, EntityStatusMark } from '../../../../packages/fractal-ui/src/assistant-ui/entity/EntityCard.tsx'
import { useAttention, useDataSource, useNowSnapshot } from '../data/react.tsx'
import { useMissionRows } from '../missions/useMissionRows.ts'
import { quotaStateAtom } from '../monitor/state.ts'
import { usageSectionView } from '../monitor/usageSectionView.ts'
import type { PresentationWorld } from '../workspace/presentation.ts'
import { describe } from '../workspace/kinds.ts'
import { subjectOfRef } from '../workspace/subject.ts'
import { alertTabMarkers } from './alertsTab.ts'

/**
 * The cards of the lists and screens a tab can show, and of a resource no plugin claims. Each reads only while
 * it shows, from the feeds its screen reads, and states only what it observed: an unknown fact is omitted.
 */
const listLimit = 4

const plural = (count: number, word: string) => `${count} ${word}${count === 1 ? '' : 's'}`

const InboxCard = () => {
  const attention = useAttention()
  const open = attention._tag === 'Observed' ? attention.value.filter(item => item.state === 'open') : undefined
  // The tab's own marks: `?` for a question, `◆` for anything else that needs you.
  const { questionCount, needsYouCount } = alertTabMarkers(attention)
  const needs = questionCount + needsYouCount
  return <EntityCard entity={{ kind: 'view', id: 'inbox' }} testId="inbox-hover-card">
    <EntityCardHeader title="Inbox" kind="Alerts and updates"
      glyph={needs > 0 ? <EntityStatusMark status={{ tone: 'needs-you', label: 'Needs you' }} /> : undefined}
      summary={open === undefined ? ['Loading'] : [questionCount > 0 ? plural(questionCount, 'question') : undefined, needsYouCount > 0 ? `${needsYouCount} need${needsYouCount === 1 ? 's' : ''} you` : undefined, `${open.length} open`]} />
    {open !== undefined && open.length > 0 && <EntityCardSection label="Open" icon="alert">
      <EntityCardList>{open.slice(0, listLimit).map(item => <EntityCardListItem key={item.id} title={item.title} />)}</EntityCardList>
      {open.length > listLimit && <EntityCardNote>+{open.length - listLimit} more</EntityCardNote>}
    </EntityCardSection>}
  </EntityCard>
}

const MissionsCard = () => {
  const { missions, rows } = useMissionRows()
  const active = rows.filter(row => row.presentation.settled_at === null && !row.presentation.system)
  return <EntityCard entity={{ kind: 'view', id: 'missions' }} testId="missions-hover-card">
    <EntityCardHeader title="Missions" summary={missions._tag !== 'Observed' ? ['Loading'] : [plural(active.length, 'active mission'), `${rows.length} in all`]} />
    {active.length > 0 && <EntityCardSection label="Active" icon="checklist">
      <EntityCardList>{active.slice(0, listLimit).map(row => <EntityCardListItem key={row.mission.id} title={row.mission.title} trailing={`${row.presentation.progress.done}/${row.presentation.progress.total}`} detail={row.presentation.word} />)}</EntityCardList>
      {active.length > listLimit && <EntityCardNote>+{active.length - listLimit} more</EntityCardNote>}
    </EntityCardSection>}
  </EntityCard>
}

const UsageCard = () => {
  const view = usageSectionView({ state: useAtomValue(quotaStateAtom(useDataSource().usage)), now: useNowSnapshot() })
  const summary = view._tag === 'ready' ? [view.atLimit > 0 ? `${view.atLimit} at limit` : undefined, plural(view.rows.length, 'account')]
    : view._tag === 'loading' ? ['Loading'] : view._tag === 'not-configured' ? ['No quota source'] : view._tag === 'empty' ? ['No readings'] : ['Unavailable']
  return <EntityCard entity={{ kind: 'view', id: 'usage' }} testId="usage-hover-card">
    <EntityCardHeader title="Usage" kind="Spend and quota" summary={summary} />
    {view._tag === 'ready' && <EntityCardMeta>{view.rows.slice(0, listLimit).map(row => <EntityCardMetaLine key={row.id} icon="gauge" label={row.name}>{`${row.percent} of ${row.window} · ${row.reset}`}</EntityCardMetaLine>)}</EntityCardMeta>}
    {view._tag === 'ready' && view.old !== undefined && <EntityCardNote>{view.old}</EntityCardNote>}
  </EntityCard>
}

const viewCards = { inbox: InboxCard, missions: MissionsCard, usage: UsageCard } as const

/** The card of a list or screen, by its `view` entity id. */
export const LiveViewCard = ({ id }: { readonly id: string }) => {
  const Card = viewCards[id as keyof typeof viewCards]
  return Card === undefined ? null : <Card />
}

const agoLabel = (at: number, now: number): string => {
  const minutes = Math.max(0, Math.round((now - at) / 60_000))
  return minutes < 1 ? 'just now' : minutes < 60 ? `${minutes}m ago` : minutes < 48 * 60 ? `${Math.round(minutes / 60)}h ago` : `${Math.round(minutes / 1440)}d ago`
}

const ObservedChange = ({ id }: { readonly id: string }) => {
  const source = useDataSource().resources
  if (source === undefined) return null
  return <ObservedChangeFrom byId={source.byId} id={id} />
}
const ObservedChangeFrom = ({ byId, id }: { readonly byId: NonNullable<ReturnType<typeof useDataSource>['resources']>['byId']; readonly id: string }) => {
  const feed = useAtomValue(byId(id))
  const now = useNowSnapshot()
  if (feed._tag !== 'Observed') return null
  return <EntityCardMetaLine icon="clock" label="Last change">{agoLabel(DateTime.toEpochMillis(feed.value.observed_at), now)}</EntityCardMetaLine>
}

/**
 * A document's or an unclaimed resource's card: its name and kind as the tab names them, its path, and when it
 * last changed when that was observed. A claimed resource (a GitHub item) gets its plugin's card instead.
 */
export const LiveResourceCard = ({ id, world }: { readonly id: string; readonly world: PresentationWorld }) => {
  const descriptor = describe(subjectOfRef(id), world)
  const title = descriptor.title._tag === 'Known' || descriptor.title._tag === 'Fixed' ? descriptor.title.text : undefined
  const path = id.startsWith('doc/') ? id.slice('doc/'.length) : id
  return <EntityCard entity={{ kind: 'resource', id }} testId="resource-hover-card">
    <EntityCardHeader title={title ?? path} kind={descriptor.kind} />
    <EntityCardMeta>
      <EntityCardMetaLine icon="folder" label="Path">{path}</EntityCardMetaLine>
      <ObservedChange id={id} />
    </EntityCardMeta>
  </EntityCard>
}
