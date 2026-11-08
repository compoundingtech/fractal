import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Objectives stop loading when you leave them',
  summary: 'Leaving or refreshing the objectives view now cancels its pending read. When you come back, the last objectives show as stale while they reload.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
