import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Quiet the conversation header sync status',
  summary: 'The conversation header no longer repeats a stale or reconnecting line that the connection band already shows. Load progress and a failure with its Retry still appear there, and to-do folds now show a chevron.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
