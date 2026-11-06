import type { ChangelogEntrySource } from '../src/changelog/contract.ts'

export default {
  title: 'Earlier messages join the conversation in small batches',
  summary: 'Earlier messages now prepare in small batches without hiding answers that are already visible, even when the older page starts within a turn. Incoming updates and replacements cancel stale preparation without changing message order.',
  kind: 'improvement',
} satisfies ChangelogEntrySource
