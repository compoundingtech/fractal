import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Earlier messages load with less delay',
  summary: 'Loading earlier messages now causes less delay in the conversation. Older messages start to appear one frame later and continue to appear in small batches.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
