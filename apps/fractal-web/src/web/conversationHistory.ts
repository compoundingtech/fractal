import type { TranscriptHistory } from '../../../../packages/fractal-ui/src/assistant-ui/composition/Transcript.tsx'
import type { ConversationPage, Feed } from '../data/source.ts'

/** Native completeness and recovery stay with the source; the kit owns paging gestures. */
export const conversationHistory = (feed: Feed<ConversationPage>, loadOlder?: () => Promise<void>): TranscriptHistory => {
  if (feed._tag !== 'Observed') return { _tag: 'Pending' }
  if (!feed.value.hasOlder) return { _tag: 'Complete' }
  if (feed.value.historyLoad?._tag === 'Loading') return { _tag: 'Loading' }
  if (loadOlder === undefined) return { _tag: 'Pending' }
  if (feed.value.historyLoad?._tag === 'Failed') return { _tag: 'Failed', retry: loadOlder, detail: feed.value.historyLoad.detail }
  return { _tag: 'More', loadOlder }
}
