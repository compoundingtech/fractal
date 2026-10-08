import type { EmbraceThreadProps, ConversationRuntimeOptions } from '@smalltalk/fractal-ui/assistant-ui'
import type { SyncLineProps } from '@smalltalk/fractal-ui/assistant-ui/sync'
import type { ConversationItem } from '../conversation/model.ts'
import type { FeedSyncObservation } from '../data/feedSync.ts'
import type { ConversationPage, Feed } from '../data/source.ts'

/** The kit mirrors all nine item variants. Preserve payloads and provenance without copying.
 * Future optimistic send-state mapping belongs at this single boundary when the kit supports it.
 */
export const mapConversationItemsForTranscript = (
  items: readonly ConversationItem[],
): EmbraceThreadProps['items'] => items

/** SDK and kit share the same union; never infer Live from retained rows or agent activity. */
export const mapConversationSync = (
  observation: FeedSyncObservation | undefined,
  now: number,
): SyncLineProps | undefined => observation === undefined ? undefined : {
  status: observation.status,
  observedAt: observation.observedAt,
  now,
  label: 'conversation',
}

export type ConversationTranscriptState =
  | { readonly _tag: 'Waiting' }
  | { readonly _tag: 'Unavailable'; readonly title: string; readonly detail: string }
  | {
      readonly _tag: 'Observed'
      readonly items: EmbraceThreadProps['items']
      readonly hasOlder: boolean
      readonly notice?: string
      readonly filteredEmpty: boolean
    }

const unavailableTitle = (reason: 'ungranted' | 'unsupported' | 'failed'): string => {
  switch (reason) {
    case 'ungranted': return 'Conversation access not granted'
    case 'unsupported': return 'Conversation not supported'
    case 'failed': return 'Conversation unavailable'
  }
}

export const mapConversationFeed = (feed: Feed<ConversationPage>): ConversationTranscriptState => {
  switch (feed._tag) {
    case 'Waiting': return feed
    case 'Unavailable': return { _tag: 'Unavailable', title: unavailableTitle(feed.reason), detail: feed.detail }
    case 'Observed': return {
      _tag: 'Observed',
      items: mapConversationItemsForTranscript(feed.value.items),
      hasOlder: feed.value.hasOlder,
      filteredEmpty: feed.value.items.length === 0 && feed.value.observation?.empty === false,
      ...(feed.error !== undefined
        ? { notice: `${unavailableTitle(feed.error.reason)}: ${feed.error.detail}. Showing the last verified entries.` }
        : feed.freshness === 'stale' ? { notice: 'Showing the last verified conversation entries.' } : {}),
    }
  }
}

/** Read-only transcript capabilities: no send/edit/retry transport is claimed by this layer. */
export const transcriptRuntimeOptions = (items: EmbraceThreadProps['items']): ConversationRuntimeOptions => ({
  messages: items,
  isDisabled: true,
  onNew: async () => { throw new Error('This conversation view is read-only') },
})
