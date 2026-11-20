/**
 * "Related requests": the explanatory request history Fractal returns for one thread. Each request shows its own
 * original wording and its own status; an arrow between two requests explains why one was asked after the other and
 * never makes one a prerequisite of, or an answer to, the other. Only what the reader returned is shown: hidden
 * requests leave no identifier, reason or count behind.
 */
import * as React from 'react'
import * as stylex from '@stylexjs/stylex'
import { Option } from 'effect'
import { useAtomValue } from '@effect/atom-react'
import { Button } from 'react-aria-components'
import type { HistoricalReference, PayloadUnavailable, RequestHistoryItem, RequestHistoryPage, SourceEpisode } from '@fractal/client/schema/read'
import { ago } from '../../../../clients/typescript/fractal-views/time.ts'
import { colorVars as c, geometryVars as g, radiusVars as r, spaceVars as s, typeVars as t } from '../../../../packages/fractal-ui/src/assistant-ui/composition-tokens.stylex.ts'
import { SystemNotice } from '../../../../packages/fractal-ui/src/assistant-ui/composition/SystemState.tsx'
import { agentDisplayName, personName } from '../data/agentName.ts'
import { useNow } from '../data/react.tsx'
import type { Feed } from '../data/source.ts'
import type { ThreadSource } from './threadSource.ts'
import { sourceEpisodeKey as keyOf } from './requestHistoryPages.ts'

/** One visible request and the visible requests it was asked after. */
export interface RelatedRequest {
  readonly item: RequestHistoryItem
  readonly after: readonly { readonly predecessor: RequestHistoryItem; readonly reason: string }[]
}

/** A lone request, or the visible episodes of one source kept together, each with its own status. */
export type RelatedRequestUnit =
  | { readonly _tag: 'Request'; readonly request: RelatedRequest }
  | { readonly _tag: 'SameSource'; readonly sourceId: string; readonly requests: readonly RelatedRequest[] }

/**
 * Orders the returned requests so a predecessor comes before the requests asked after it; unrelated requests keep
 * Fractal's order. Relations name only requests on this page or its boundary, so an arrow to anything else is dropped
 * rather than drawn to a placeholder. Same-source episodes are one unit in that order: the group comes after every
 * predecessor of any of its members, and its episodes keep Fractal's order (Fractal never links a source to itself).
 */
export function relatedRequests(page: RequestHistoryPage): readonly RelatedRequestUnit[] {
  const visible = new Map<string, RequestHistoryItem>()
  for (const item of [...page.items, ...page.boundary_items]) if (!visible.has(keyOf(item.key))) visible.set(keyOf(item.key), item)
  const group = new Map<string, string>()
  for (const each of page.source_episodes) {
    const members = each.episodes.map(keyOf).filter(key => visible.has(key))
    if (members.length > 1) for (const key of members) group.set(key, each.source_id)
  }
  // One unit per lone request or same-source group, in order of first appearance in Fractal's order.
  const unitOf = (key: string) => { const sourceId = group.get(key); return sourceId === undefined ? `request:${key}` : `source:${sourceId}` }
  const members = new Map<string, string[]>()
  for (const key of visible.keys()) members.set(unitOf(key), [...members.get(unitOf(key)) ?? [], key])
  const unitOrder = [...members.keys()]
  const after = new Map<string, { readonly predecessor: RequestHistoryItem; readonly reason: string }[]>()
  const successors = new Map<string, string[]>()
  const indegree = new Map(unitOrder.map(unit => [unit, 0]))
  const edges = new Set<string>()
  const reasons = new Set<string>()
  for (const relation of page.relations) {
    const from = keyOf(relation.predecessor)
    const to = keyOf(relation.request)
    const predecessor = visible.get(from)
    if (predecessor === undefined || !visible.has(to) || from === to) continue
    if (!reasons.has(JSON.stringify([from, to, relation.reason]))) {
      reasons.add(JSON.stringify([from, to, relation.reason]))
      after.set(to, [...after.get(to) ?? [], { predecessor, reason: relation.reason }])
    }
    const [fromUnit, toUnit] = [unitOf(from), unitOf(to)]
    if (fromUnit === toUnit || edges.has(JSON.stringify([fromUnit, toUnit]))) continue
    edges.add(JSON.stringify([fromUnit, toUnit]))
    successors.set(fromUnit, [...successors.get(fromUnit) ?? [], toUnit])
    indegree.set(toUnit, (indegree.get(toUnit) ?? 0) + 1)
  }
  const request = (key: string): RelatedRequest => ({ item: visible.get(key)!, after: after.get(key) ?? [] })
  // Kahn's order, always taking the earliest ready unit in Fractal's order; a cycle keeps Fractal's order.
  const units: RelatedRequestUnit[] = []
  const placed = new Set<string>()
  while (placed.size < unitOrder.length) {
    const next = unitOrder.find(unit => !placed.has(unit) && indegree.get(unit) === 0) ?? unitOrder.find(unit => !placed.has(unit))!
    placed.add(next)
    for (const successor of successors.get(next) ?? []) indegree.set(successor, (indegree.get(successor) ?? 0) - 1)
    const keys = members.get(next)!
    const sourceId = group.get(keys[0]!)
    units.push(sourceId === undefined ? { _tag: 'Request', request: request(keys[0]!) } : { _tag: 'SameSource', sourceId, requests: keys.map(request) })
  }
  return units
}

const statusLabels: Readonly<Record<string, string>> = {
  ready: 'Open', waiting: 'Waiting', claimed: 'In progress', completed: 'Completed', cancelled: 'Cancelled',
  failed: 'Failed', blocked: 'Blocked', skipped: 'Skipped',
}
/** Fractal's own status word, never one inferred from a related request. */
export const requestStatusLabel = (status: string): string =>
  statusLabels[status] ?? (status === '' ? 'Status unknown' : `${status.charAt(0).toUpperCase()}${status.slice(1).replaceAll('-', ' ')}`)

/** Why the original wording is missing. Never replaced by the source's current text. */
const unavailablePayload: Readonly<Record<PayloadUnavailable, string>> = {
  'retained-definition-unavailable': 'Fractal no longer has the definition this request was asked from.',
  'retained-step-unavailable': 'Fractal no longer has the step this request was asked from.',
  'opening-unavailable': 'Fractal has no record of this request opening.',
  'historical-interpolation-unavailable': 'Fractal can’t rebuild this request’s original wording.',
  'boundary-unavailable': 'Fractal can’t tell where this request began.',
  'payload-unavailable': 'Fractal no longer has this request’s original wording.',
  'read-bound-exceeded': 'This request is too large to show here.',
}
export const originalUnavailable = 'Original request unavailable'

/** The words a "Requested after" link uses for its predecessor: its own original title or first line, or nothing invented. */
const requestName = (item: RequestHistoryItem): string => {
  if (item.payload.availability === 'unavailable') return 'an earlier request whose wording is unavailable'
  const title = Option.getOrUndefined(item.payload.title)
  return title ?? item.payload.text.split('\n', 1)[0] ?? ''
}

type AnswerView = { readonly summary: string; readonly respondent?: string; readonly at?: number }
/** The standing answers recorded for this exact episode; an answer without a summary has nothing to show. */
const answerView = (value: unknown): AnswerView | undefined => {
  if (typeof value !== 'object' || value === null || !('summary' in value)) return undefined
  const summary = value.summary
  if (typeof summary !== 'string' || summary.trim() === '') return undefined
  const respondent = 'respondent' in value ? value.respondent : undefined
  const at = 'answered_at_unix_ms' in value ? value.answered_at_unix_ms : undefined
  return {
    summary,
    ...(typeof respondent === 'string' && respondent !== '' ? { respondent } : {}),
    ...(typeof at === 'number' && Number.isFinite(at) ? { at } : {}),
  }
}

const unsupportedSource = 'Fractal keeps no request history for this item’s source.'
const incompleteHistory = 'Some of this history could not be read, so requests may be missing.'

/**
 * The section for one thread's request history read. Nothing is shown when no request relates to another, when the
 * connected Fractal cannot serve this read, or before a thread has any history; the conversation stays first-class.
 */
export function RequestHistorySection({ feed, onRetry }: {
  readonly feed: Feed<RequestHistoryPage>
  readonly onRetry?: (() => void) | undefined
}): React.ReactNode {
  const headingId = React.useId()
  const frame = (body: React.ReactNode) => <section data-request-history="" aria-labelledby={headingId} {...stylex.props(styles.section)}>
    <h3 id={headingId} {...stylex.props(styles.heading)}>Related requests</h3>
    {body}
  </section>
  if (feed._tag === 'Waiting') return frame(<SystemNotice kind="loading" scope="inline" title="Loading related requests" />)
  // A Fractal without this read leaves the thread as it was; it is not an error in the thread.
  if (feed._tag === 'Unavailable') return feed.reason === 'unsupported' ? null
    : frame(<SystemNotice kind="error" scope="inline" title={feed.detail}
      {...(feed.explanation === undefined ? {} : { explanation: feed.explanation })}
      {...(feed.reason === 'failed' && onRetry !== undefined ? { onRetry } : {})} />)
  const page = feed.value
  const units = relatedRequests(page)
  const shown = units.reduce((count, unit) => count + (unit._tag === 'Request' ? 1 : unit.requests.length), 0)
  const unsupported = Option.getOrUndefined(page.source_support) === 'unsupported-source'
  // A lone complete request relates to nothing: it is the thread's own origin, which its host already shows.
  if (shown < 2 && page.complete && !(unsupported && shown === 0)) return null
  return frame(<>
    {feed.freshness === 'stale' ? <SystemNotice kind="stale" scope="inline" title="Updates paused" {...(onRetry === undefined ? {} : { onRetry })} /> : null}
    {shown === 0 && unsupported ? <p data-request-history-unsupported="" {...stylex.props(styles.note)}>{unsupportedSource}</p> : null}
    {shown === 0 ? null : <RequestList units={units} />}
    {page.complete ? null : <p data-request-history-incomplete="" {...stylex.props(styles.note)}>{incompleteHistory}</p>}
  </>)
}

function RequestList({ units }: { readonly units: readonly RelatedRequestUnit[] }) {
  const rows = React.useRef(new Map<string, HTMLLIElement>())
  const register = (key: string) => (node: HTMLLIElement | null) => { if (node === null) rows.current.delete(key); else rows.current.set(key, node) }
  // "Requested after" moves to that request's row in this list, the ordinary in-page way back to it.
  const show = (key: SourceEpisode) => {
    const row = rows.current.get(keyOf(key))
    row?.scrollIntoView?.({ block: 'nearest' })
    row?.focus({ preventScroll: true })
  }
  const row = (request: RelatedRequest) => <RequestRow key={keyOf(request.item.key)} request={request} rowRef={register(keyOf(request.item.key))} onShow={show} />
  return <ol {...stylex.props(styles.list)}>
    {units.map(unit => unit._tag === 'Request' ? row(unit.request)
      : <li key={`source:${unit.sourceId}`} data-request-same-source={unit.sourceId} {...stylex.props(stylex.defaultMarker(), styles.group)}>
        <p {...stylex.props(styles.groupCaption)}>Asked again from the same step · each has its own status</p>
        <ol {...stylex.props(styles.groupList)}>{unit.requests.map(row)}</ol>
      </li>)}
  </ol>
}

function RequestRow({ request: { item, after }, rowRef, onShow }: {
  readonly request: RelatedRequest
  readonly rowRef: (node: HTMLLIElement | null) => void
  readonly onShow: (key: SourceEpisode) => void
}) {
  const now = useNow()
  const payload = item.payload
  const title = payload.availability === 'available' ? Option.getOrUndefined(payload.title) : originalUnavailable
  const text = payload.availability === 'available' ? payload.text : unavailablePayload[payload.reason]
  const status = item.state.availability === 'available' ? requestStatusLabel(item.state.status) : 'Status unknown'
  const answers = item.state.availability === 'available' ? item.state.answers.flatMap(answer => answerView(answer) ?? []) : []
  const references = payload.availability === 'available' ? payload.references : []
  return <li ref={rowRef} tabIndex={-1} data-request-source={item.key.source_id} data-request-episode={item.key.episode}
    data-request-status={item.state.availability === 'available' ? item.state.status : 'unavailable'}
    data-request-payload={payload.availability} {...stylex.props(stylex.defaultMarker(), styles.row)}>
    {after.map(({ predecessor, reason }) => <p key={`${keyOf(predecessor.key)}:${reason}`} data-request-after={predecessor.key.episode} {...stylex.props(styles.after)}>
      <Button onPress={() => onShow(predecessor.key)} {...stylex.props(styles.afterLink)}>Requested after “{requestName(predecessor)}”</Button>
      <span {...stylex.props(styles.reason)}>{reason}</span>
    </p>)}
    <div {...stylex.props(styles.header)}>
      {title === undefined ? null : <span data-request-title="" {...stylex.props(styles.title, payload.availability === 'unavailable' && styles.unavailable)}>{title}</span>}
      <span data-request-status-label="" {...stylex.props(styles.status, title === undefined && styles.statusOnly)}>{status}</span>
    </div>
    {text === title ? null : <p data-request-text="" {...stylex.props(styles.text, payload.availability === 'unavailable' && styles.unavailable)}>{text}</p>}
    {answers.length === 0 ? null : <ul {...stylex.props(styles.answers)}>
      {answers.map((answer, index) => <li key={index} data-request-answer="" {...stylex.props(stylex.defaultMarker(), styles.answer)}>
        <span {...stylex.props(styles.answerMeta)}>{answerLead(answer, now)}</span>
        <span {...stylex.props(styles.answerSummary)}>{answer.summary}</span>
      </li>)}
    </ul>}
    {references.length === 0 ? null : <ul aria-label="References" {...stylex.props(styles.references)}>
      {references.map(reference => <ReferenceLine key={`${reference.target_id}:${Option.getOrElse(reference.revision, () => '')}`} reference={reference} />)}
    </ul>}
  </li>
}

const answerLead = (answer: AnswerView, now: number): string => {
  const by = answer.respondent === undefined ? undefined : personName(answer.respondent) ?? agentDisplayName({ id: answer.respondent })
  const date = answer.at === undefined ? undefined : new Date(answer.at)
  const when = date === undefined || !Number.isFinite(date.getTime()) ? undefined : `${ago(date.toISOString(), now)} ago`
  return [by === undefined ? 'Answered' : `Answered by ${by}`, when].filter(part => part !== undefined).join(' · ')
}

/** A reference stays a reference: a pinned revision is named as such, and a live target may have changed since. */
function ReferenceLine({ reference }: { readonly reference: HistoricalReference }) {
  const revision = Option.getOrUndefined(reference.revision)
  const target = /^https?:\/\//.test(reference.target_id)
    ? <a href={reference.target_id} target="_blank" rel="noreferrer" {...stylex.props(styles.link)}>{reference.target_id}</a>
    : <code {...stylex.props(styles.code)}>{reference.target_id}</code>
  return <li data-request-reference={reference.semantics} {...stylex.props(stylex.defaultMarker(), styles.reference)}>
    {target}
    {revision === undefined ? null : <span> at revision <code {...stylex.props(styles.code)}>{revision}</code></span>}
    <span {...stylex.props(styles.referenceNote)}>{reference.semantics === 'live-reference' ? 'Live, may have changed since' : 'As asked'}</span>
  </li>
}

/** Mounts the optional request history read for one thread; a source without it shows nothing. */
export function ThreadRequestHistory({ source, threadId }: { readonly source: ThreadSource; readonly threadId: string }) {
  const read = source.requestHistory
  if (read === undefined) return null
  return <ThreadRequestHistoryRead read={read} reload={source.reloadRequestHistory} threadId={threadId} />
}

function ThreadRequestHistoryRead({ read, reload, threadId }: {
  readonly read: NonNullable<ThreadSource['requestHistory']>
  readonly reload: ThreadSource['reloadRequestHistory']
  readonly threadId: string
}) {
  const feed = useAtomValue(read(threadId))
  return <RequestHistorySection feed={feed} {...(reload === undefined ? {} : { onRetry: () => reload(threadId) })} />
}

const styles = stylex.create({
  section: { display: 'flex', flexDirection: 'column', gap: s.xs, minWidth: 0, fontSize: t.metaSize, lineHeight: t.metaLeading },
  heading: { margin: 0, color: c.fgMuted, fontSize: t.metaSize, fontWeight: 600 },
  note: { margin: 0, color: c.fgMuted, fontSize: t.metaSize, overflowWrap: 'anywhere' },
  // One bordered list like the thread rows; each request after the first draws the hairline above it.
  list: { display: 'flex', flexDirection: 'column', minWidth: 0, padding: 0, margin: 0, listStyle: 'none', borderWidth: g.hairline, borderStyle: 'solid', borderColor: c.border, borderRadius: r.sm },
  row: { display: 'flex', flexDirection: 'column', gap: s.xs, minWidth: 0, padding: s.sm, borderRadius: r.sm,
    borderBlockStartWidth: { default: g.hairline, ':first-child': 0 }, borderBlockStartStyle: 'solid', borderBlockStartColor: c.border,
    outline: { default: 'none', ':focus': `${g.focusRing} solid ${c.primary}` }, outlineOffset: `calc(-1 * ${g.focusRing})` },
  group: { display: 'flex', flexDirection: 'column', minWidth: 0, borderBlockStartWidth: { default: g.hairline, ':first-child': 0 }, borderBlockStartStyle: 'solid', borderBlockStartColor: c.border },
  groupCaption: { margin: 0, paddingInline: s.sm, paddingBlockStart: s.sm, color: c.fgMuted, fontSize: t.metaSize },
  // Same-source episodes share one rule at the start edge: grouped, not merged.
  groupList: { display: 'flex', flexDirection: 'column', minWidth: 0, padding: 0, margin: 0, marginInlineStart: s.sm, listStyle: 'none',
    borderInlineStartWidth: 2, borderInlineStartStyle: 'solid', borderInlineStartColor: c.border },
  after: { display: 'flex', flexWrap: 'wrap', alignItems: 'baseline', columnGap: s.sm, rowGap: 0, margin: 0, minWidth: 0, color: c.fgMuted },
  afterLink: { minWidth: 0, maxWidth: '100%', overflow: 'hidden', textOverflow: 'ellipsis', whiteSpace: 'nowrap', padding: 0, borderWidth: 0, backgroundColor: 'transparent',
    color: { default: c.fgSoft, ':hover': c.fg }, fontFamily: 'inherit', fontSize: t.metaSize, textAlign: 'start', textDecorationLine: 'underline', textUnderlineOffset: '2px', cursor: 'pointer', borderRadius: r.sm,
    outline: { default: 'none', ':focus-visible': `${g.focusRing} solid ${c.primary}` } },
  reason: { minWidth: 0, overflowWrap: 'anywhere' },
  header: { display: 'flex', alignItems: 'baseline', gap: s.sm, minWidth: 0 },
  title: { flexGrow: 1, minWidth: 0, color: c.fg, fontWeight: t.weightMedium, overflowWrap: 'anywhere' },
  unavailable: { color: c.fgMuted },
  status: { flexShrink: 0, paddingInline: s.xs, borderWidth: g.hairline, borderStyle: 'solid', borderColor: c.borderStrong, borderRadius: r.sm, color: c.fgSoft, whiteSpace: 'nowrap' },
  statusOnly: { marginInlineStart: 'auto' },
  text: { margin: 0, color: c.fg, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' },
  answers: { display: 'flex', flexDirection: 'column', gap: s.xs, padding: 0, margin: 0, listStyle: 'none' },
  answer: { display: 'flex', flexDirection: 'column', minWidth: 0, paddingInlineStart: s.sm, borderInlineStartWidth: 2, borderInlineStartStyle: 'solid', borderInlineStartColor: c.border },
  answerMeta: { color: c.fgMuted },
  answerSummary: { color: c.fg, whiteSpace: 'pre-wrap', overflowWrap: 'anywhere' },
  references: { display: 'flex', flexDirection: 'column', gap: 0, padding: 0, margin: 0, listStyle: 'none', color: c.fgMuted },
  reference: { display: 'flex', flexWrap: 'wrap', alignItems: 'baseline', columnGap: s.xs, minWidth: 0, overflowWrap: 'anywhere' },
  referenceNote: { color: c.fgMuted, '::before': { content: '"· "' } },
  link: { color: c.fgSoft, textDecorationLine: 'underline', textUnderlineOffset: '2px', overflowWrap: 'anywhere', borderRadius: r.sm,
    outline: { default: 'none', ':focus-visible': `${g.focusRing} solid ${c.primary}` } },
  code: { fontFamily: t.fontMono, overflowWrap: 'anywhere' },
})
