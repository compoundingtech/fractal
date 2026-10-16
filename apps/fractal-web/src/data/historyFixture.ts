import type { CollectionFrame, TimelineEntry } from '@smalltalk/st3-client'
import { decodeConversationChunk } from '@st3/sdk/effect'
import { LiveTimeline } from '../conversation/fromTimeline.ts'
import * as Atom from 'effect/reactivity/Atom'
import * as AtomRegistry from 'effect/reactivity/AtomRegistry'
import type { ConversationItem } from '../conversation/model.ts'
import { fixtureSource } from './fixtureSource.ts'
import { observed, type ConversationPage, type DataSource, type Feed } from './source.ts'
import { undeclared } from '../monitor/source.ts'

export const historyFixtureRef = 'agent/row-review'
export const historyFixtureNow = Date.UTC(2026, 0, 15, 12)
const text = (id: string, body: string): ConversationItem => ({ _tag: 'Text', id, role: 'assistant', text: body, attachments: [], streaming: false, at: new Date(historyFixtureNow).toISOString() })
export const historyRecent = text('history/recent', 'Selection is retained in the newest result.')
export const historyEarlier = text('history/earlier', 'The earlier review identified the stable row keys.')

export interface HistoryFixture {
  readonly source: DataSource
  readonly registry: AtomRegistry.AtomRegistry
  readonly settle: (success: boolean) => void
  readonly setPage: (page: ConversationPage) => void
}
/** Deliberately settled by the harness: loading and failure are observable, not timed guesses. */
export const createHistoryFixture = ({ paging = true, complete = true, earlier = historyEarlier }: { readonly paging?: boolean; readonly complete?: boolean; readonly earlier?: ConversationItem } = {}): HistoryFixture => {
  const registry = AtomRegistry.make()
  const conversation = Atom.make<Feed<ConversationPage>>(observed({ value: { items: [historyRecent], hasOlder: true } }))
  let settle: ((success: boolean) => void) | undefined
  const source = {
    ...fixtureSource({ world: { now: historyFixtureNow, agents: [], missions: [], attention: [], events: [], conversations: {}, terminals: {}, envelopes: {}, usage: undeclared } }),
    conversation: () => conversation,
    ...(paging ? { loadEarlierConversation: async () => {
      const feed = registry.get(conversation)
      if (feed._tag !== 'Observed' || !feed.value.hasOlder || feed.value.historyLoad?._tag === 'Loading') return
      registry.set(conversation, observed({ value: { ...feed.value, historyLoad: { _tag: 'Loading' } } }))
      const success = await new Promise<boolean>(resolve => { settle = resolve })
      registry.set(conversation, observed({ value: success
        ? { items: [earlier, ...feed.value.items], hasOlder: !complete }
        : { ...feed.value, historyLoad: { _tag: 'Failed', detail: 'The history service could not be reached.' } } }))
    } } : {}),
  }
  return { source, registry, setPage: page => registry.set(conversation, observed({ value: page })), settle: (success: boolean) => { settle?.(success); settle = undefined } }
}

/** Curated native windows drive the production fold; the control uses its old full replacement. */
export const createHistoryWindowFixture = ({ retainPrefix = true }: { readonly retainPrefix?: boolean } = {}) => {
  const entry = (sequence: number, text: string): TimelineEntry => ({ id: `timeline-entry/${sequence}`, sequence, revision: 1, type: 'content', role: 'assistant', final: true, timestamp: '2026-01-15T12:00:00Z', body: { text, media_type: 'text/markdown' } })
  const chunk = (items: TimelineEntry[]) => decodeConversationChunk({ kind: 'conversation', id: 'follow/example', collection: 'conversation', session_id: 'session/example', replace: true, has_more: true, items } satisfies CollectionFrame)
  const timeline = new LiveTimeline()
  timeline.apply(chunk([entry(2, 'Current boundary.'), entry(3, 'Current response.')]))
  timeline.prepend(chunk([entry(1, 'Earlier page stays visible.')]).entries, false)
  const fixture = createHistoryFixture({ paging: false })
  fixture.setPage({ items: timeline.project().items, hasOlder: timeline.hasOlder })
  return { ...fixture, replace: () => {
    const replacement = chunk([entry(3, 'Revised current response.'), entry(4, 'New live response.')])
    if (retainPrefix) timeline.applyWindow(replacement)
    else timeline.apply(replacement)
    fixture.setPage({ items: timeline.project().items, hasOlder: timeline.hasOlder })
  } }
}
