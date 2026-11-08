import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Keep tabs when another window starts',
  summary: 'An untouched empty window no longer overwrites tabs opened in another window while their layouts are syncing.',
  kind: 'fix',
} satisfies ChangelogEntrySource
