import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Load earlier conversation messages',
  summary: 'Load earlier messages from served conversation history without losing your reading position. Loading and retry states keep the history boundary explicit until the complete conversation is available.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
